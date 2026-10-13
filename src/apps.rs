//! 添加、移除、移动隔离软件。需要管理员权限：要写库和沙盒用户的个人文件夹。
//!
//! 添加有两种方式：
//! - 快速安装（默认，静默）：用 7-Zip 解开安装包，程序放进库的 apps\<名字>，不运行安装程序、不写注册表
//! - 原版安装程序（--install，图形化）：以沙盒用户身份运行安装程序，并尽量把安装目录预填为库里的位置

use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

use crate::{
    account::{lookup_sid, profile_path},
    acl,
    config::{self, Config},
    cred, drives, launch,
    library::{self, Library},
    shortcut,
    unpack::{self, Candidate, InstallerKind},
};

pub struct AddOptions {
    pub name: String,
    pub installer: PathBuf,
    pub display: Option<String>,
    pub exe: Option<String>,
    pub install: bool,
    pub startup: bool,
    pub force: bool,
    /// 装进哪个盘的库，默认系统盘
    pub library: Option<char>,
}

/// ERROR_ELEVATION_REQUIRED：程序清单要求管理员权限
const HR_ELEVATION_REQUIRED: u32 = 0x8007_02E4;

/// 沙盒用户、它的 SID 和个人文件夹
struct Sandbox {
    user: String,
    sid: String,
    profile: PathBuf,
}

fn sandbox(cfg: &Config) -> Result<Sandbox> {
    let user = cfg.sandbox_user.clone();
    let sid = lookup_sid(&user).with_context(|| format!("找不到用户 {user}，请先运行 isobox-setup install --apply"))?;
    let profile = profile_path(&sid)?.with_context(|| format!("{user} 还没有个人文件夹，请先运行 install"))?;
    Ok(Sandbox { user, sid, profile })
}

/// 取得某个盘的库，没有就建一个
pub fn ensure_library(cfg: &Config, drive: char) -> Result<Library> {
    let sb = sandbox(cfg)?;
    if let Some(lib) = library::get(drive, &sb.sid) {
        return Ok(lib);
    }
    println!("{drive} 盘还没有库，正在创建 {}", library::root(drive).display());
    library::create(drive, &sb.user, &sb.sid)
}

pub fn add(cfg_path: &Path, cfg: &Config, o: &AddOptions) -> Result<()> {
    config::validate_app_name(&o.name)?;
    // 指向不存在程序的非绿色版条目只是占位，可以直接替换
    let placeholder = cfg
        .apps
        .get(&o.name)
        .is_some_and(|a| !a.portable && a.library.is_none() && a.exe.metadata().is_err());
    if placeholder {
        println!("配置里的 {} 指向的程序不存在，视为占位条目，将被替换", o.name);
    }
    if cfg.apps.contains_key(&o.name) && !o.force && !placeholder {
        bail!(
            "配置里已经有 {0}。换个名字，或者用 isobox-setup remove-app {0} 先移除，或者加 --force 覆盖",
            o.name
        );
    }
    if !o.installer.is_file() {
        bail!("找不到安装包: {}", o.installer.display());
    }
    let sb = sandbox(cfg)?;
    let drive = o.library.unwrap_or_else(drives::system_drive).to_ascii_uppercase();
    let lib = ensure_library(cfg, drive)?;
    let display = o.display.clone().unwrap_or_else(|| o.name.clone());
    let app_dir = lib.apps_dir().join(&o.name);

    let (exe, mut fields) = if o.install {
        let exe = install_as_user(cfg, &sb, &app_dir, o)?;
        let mut f = vec![("exe", toml::Value::from(exe.display().to_string()))];
        if exe.starts_with(&lib.root) {
            f.push(("library", toml::Value::from(drive.to_string())));
        } else {
            println!(
                "注意：安装程序没有使用库里的位置，软件装在了 {}。\n\
                 它仍然是隔离的，只是不在库里，也不能移动到其他库",
                exe.parent().unwrap_or(&exe).display()
            );
        }
        (exe, f)
    } else {
        let seven = cfg.sevenzip.clone().unwrap_or_else(|| unpack::DEFAULT_7Z.into());
        let exe = unpack::unpack(&seven, &o.installer, &app_dir, o.exe.as_deref(), o.force, &|c| pick(c))?;
        let file = exe.file_name().unwrap().to_string_lossy().to_string();
        (
            exe,
            vec![
                ("portable", toml::Value::from(true)),
                ("exe", toml::Value::from(file)),
                ("library", toml::Value::from(drive.to_string())),
            ],
        )
    };
    fields.push(("display", toml::Value::from(display.clone())));
    println!("主程序: {}", exe.display());

    if cfg.apps.contains_key(&o.name) {
        config::remove_app(cfg_path, &o.name)?;
    }
    config::append_app(cfg_path, &o.name, &fields)?;
    println!("已写入配置 {}", cfg_path.display());

    // 库里的程序你的账户能直接读，快捷方式直接用它的图标；装在隔离账户个人文件夹里的才需要复制图标
    if !exe.starts_with(&lib.root)
        && let Err(e) = copy_icon(&o.name, &exe) {
            eprintln!("注意：复制图标失败，快捷方式会用默认图标: {e:#}");
        }
    for p in shortcut::create(&o.name, &display, o.startup, Some(&exe))? {
        println!("已创建快捷方式 {}", p.display());
    }
    println!("\n完成。可以从桌面或开始菜单的 IsoBox 分组启动 {display}。");
    Ok(())
}

/// 以沙盒用户身份运行安装程序，安装完成后找出新出现的程序。
/// 能识别安装包类型时，把安装目录预填为库里的 app_dir
fn install_as_user(cfg: &Config, sb: &Sandbox, app_dir: &Path, o: &AddOptions) -> Result<PathBuf> {
    let appdata = sb.profile.join("AppData");
    if app_dir.exists() && std::fs::read_dir(app_dir)?.next().is_some() {
        if !o.force {
            bail!("{} 已存在且不为空，加 --force 覆盖", app_dir.display());
        }
        std::fs::remove_dir_all(app_dir)?;
    }
    // 预先建好空目录，它会继承库里 apps 目录的权限，沙盒用户可以写入
    std::fs::create_dir_all(app_dir)?;
    let before: HashSet<PathBuf> = exe_set(&appdata);

    // 沙盒用户读不到你的下载目录，先把安装包复制进它自己的临时目录
    let tmp = appdata.join(r"Local\Temp\isobox-install");
    std::fs::create_dir_all(&tmp)?;
    let inst = tmp.join(o.installer.file_name().context("安装包没有文件名")?);
    std::fs::copy(&o.installer, &inst).context("复制安装包失败")?;

    let seven = cfg.sevenzip.clone().unwrap_or_else(|| unpack::DEFAULT_7Z.into());
    let kind = unpack::installer_kind(&seven, &o.installer);
    let dir = app_dir.display().to_string();
    let (exe_to_run, args) = match kind {
        // .msi 不是可执行文件，要交给 msiexec；ALLUSERS=2 加 MSIINSTALLPERUSER=1 要求“仅为当前用户安装”
        InstallerKind::Msi => (
            PathBuf::from(r"C:\Windows\System32\msiexec.exe"),
            format!("/i \"{}\" ALLUSERS=2 MSIINSTALLPERUSER=1 INSTALLDIR=\"{dir}\"", inst.display()),
        ),
        // NSIS 的 /D= 必须放在最后，路径不能加引号
        InstallerKind::Nsis => (inst.clone(), format!("/D={dir}")),
        InstallerKind::Inno => (inst.clone(), format!("/DIR=\"{dir}\"")),
        InstallerKind::Unknown => (inst.clone(), String::new()),
    };
    match kind {
        InstallerKind::Unknown => println!(
            "无法识别安装包类型，安装目录需要你自己选。建议选 {dir}\n\
             隔离账户只能写库和它自己的个人文件夹，选别的位置会失败"
        ),
        k => println!("识别为 {k:?} 安装包，安装目录已预填为 {dir}"),
    }
    println!(
        "正在以 {} 身份运行安装程序，请在弹出的窗口里完成安装。\n\
         如果弹出要求管理员密码或确认的窗口，请选“否”：同意的话软件会装进系统，而不是隔离账户。",
        sb.user
    );
    let pw = cred::load(&sb.user)?;
    let child = match launch::launch_as(&sb.user, &pw, &exe_to_run, &args, &tmp) {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            let _ = std::fs::remove_dir(app_dir);
            let elevation = e.chain().any(|c| {
                c.downcast_ref::<windows::core::Error>()
                    .is_some_and(|w| w.code().0 as u32 == HR_ELEVATION_REQUIRED)
            });
            if elevation {
                bail!(
                    "这个安装包强制要求管理员权限，没法装进隔离账户。\n\
                     可以先试试快速安装（拆包）；如果也不行，这个软件就不适合用账户模式隔离"
                );
            }
            return Err(e);
        }
    };
    let code = child.wait()?;
    println!("安装程序已退出，退出码 {code}");
    // 有些安装程序退出后还有子进程在收尾
    std::thread::sleep(std::time::Duration::from_secs(3));
    let _ = std::fs::remove_dir_all(&tmp);

    let mut new: Vec<Candidate> = exe_set(&appdata)
        .difference(&before)
        .cloned()
        .chain(exe_set(app_dir))
        .map(|p| Candidate {
            size: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
            depth: p
                .strip_prefix(app_dir)
                .or_else(|_| p.strip_prefix(&appdata))
                .map(|r| r.components().count())
                .unwrap_or(99),
            likely_helper: unpack::is_likely_helper(&p.file_name().unwrap_or_default().to_string_lossy()),
            path: p,
        })
        .collect();
    if new.is_empty() {
        let _ = std::fs::remove_dir(app_dir);
        bail!(
            "安装程序没有在库或 {} 的用户目录里留下新程序。\n\
             可能安装被取消、失败，或者它装进了系统目录",
            sb.user
        );
    }
    if std::fs::read_dir(app_dir).is_ok_and(|mut it| it.next().is_none()) {
        let _ = std::fs::remove_dir(app_dir);
    }
    unpack::sort_candidates(&mut new);
    if let Some(want) = &o.exe {
        return new
            .iter()
            .find(|c| c.path.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(want)))
            .map(|c| c.path.clone())
            .with_context(|| format!("新安装的程序里没有 {want}"));
    }
    Ok(new[pick(&new)?].path.clone())
}

fn exe_set(root: &Path) -> HashSet<PathBuf> {
    let mut out = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
                out.insert(p);
            }
        }
    }
    out
}

/// 在终端里列出候选主程序让用户选择。只有一个时直接选它。
/// 设置了 ISOBOX_NONINTERACTIVE=1 时（自动测试）直接选推荐项
fn pick(c: &[Candidate]) -> Result<usize> {
    if c.len() == 1 {
        println!("找到主程序 {}", c[0].path.display());
        return Ok(0);
    }
    println!("找到以下程序，请选择主程序：");
    for (i, x) in c.iter().take(15).enumerate() {
        let tail: Vec<_> = x.path.components().rev().take(3).collect();
        let short: PathBuf = tail.into_iter().rev().collect();
        let note = if i == 0 {
            "  <- 推荐"
        } else if x.likely_helper {
            "  （像辅助程序）"
        } else {
            ""
        };
        println!("  {:>2}. {}  {:.1} MB{note}", i + 1, short.display(), x.size as f64 / 1048576.0);
    }
    if std::env::var("ISOBOX_NONINTERACTIVE").is_ok_and(|v| v == "1") {
        println!("非交互模式，选择 1");
        return Ok(0);
    }
    loop {
        print!("输入序号，直接回车选 1: ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let t = line.trim();
        if t.is_empty() {
            return Ok(0);
        }
        match t.parse::<usize>() {
            Ok(n) if n >= 1 && n <= c.len().min(15) => return Ok(n - 1),
            _ => println!("请输入 1 到 {} 之间的数字", c.len().min(15)),
        }
    }
}

/// 你的账户读不到沙盒用户个人文件夹里的程序，快捷方式就拿不到图标。
/// 所以把主程序复制一份到安装目录，只用来提供图标，不会被运行。
pub fn copy_icon(app: &str, exe: &Path) -> Result<()> {
    let dst = shortcut::icon_path(app)?;
    std::fs::create_dir_all(dst.parent().unwrap())?;
    std::fs::copy(exe, &dst).with_context(|| format!("复制 {} 失败", exe.display()))?;
    println!("图标已复制到 {}", dst.display());
    Ok(())
}

/// 结束以沙盒用户身份运行的某个程序
fn kill_app(user: &str, exe_name: &str) {
    if exe_name.is_empty() {
        return;
    }
    let _ = Command::new("taskkill")
        .args(["/f", "/t", "/fi", &format!("USERNAME eq {user}"), "/im", exe_name])
        .output();
    std::thread::sleep(std::time::Duration::from_secs(2));
}

/// 绿色版软件当前的程序目录：库里，或旧位置（沙盒用户个人文件夹的 Apps）
fn portable_dir(sb: &Sandbox, name: &str, a: &config::App) -> PathBuf {
    match a.library_drive() {
        Some(d) => library::app_dir(d, name),
        None => sb.profile.join("Apps").join(name),
    }
}

/// 移除一个软件：程序文件、图标、快捷方式和配置。
/// 软件在沙盒用户个人文件夹里写的数据不删，卸载整个 isobox 时会一起清掉。
pub fn remove(cfg_path: &Path, cfg: &Config, name: &str) -> Result<()> {
    let a = cfg.apps.get(name).with_context(|| format!("配置里没有 {name}"))?;
    let user = &cfg.sandbox_user;
    let exe_name = a.exe.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    kill_app(user, &exe_name);

    let sb = sandbox(cfg).ok();
    let placeholder = !a.portable && a.library.is_none() && a.exe.metadata().is_err();
    if placeholder {
        println!("{name} 是占位条目，指向的程序 {} 不存在，只删除配置", a.exe.display());
    } else if a.portable {
        if let Some(sb) = &sb {
            let dir = portable_dir(sb, name, a);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .with_context(|| format!("删除 {} 失败，确认软件已完全退出", dir.display()))?;
                println!("已删除程序目录 {}", dir.display());
            }
        }
    } else if let Some(sb) = sb.as_ref().filter(|sb| a.exe.starts_with(&sb.profile) || a.library.is_some()) {
        run_uninstaller(sb, &a.exe)?;
        // 装在库里的：卸载程序没删干净的部分一并删除
        if let Some(d) = a.library_drive() {
            let dir = library::app_dir(d, name);
            if dir.exists() {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    } else {
        println!("{name} 不在库里，也不在 {user} 的个人文件夹里，程序文件请自行卸载: {}", a.exe.display());
    }

    if let Ok(icon) = shortcut::icon_path(name)
        && icon.exists() {
            std::fs::remove_file(&icon)?;
            println!("已删除图标 {}", icon.display());
        }
    for p in shortcut::remove_for_app(name)? {
        println!("已删除快捷方式 {}", p.display());
    }
    if config::remove_app(cfg_path, name)? {
        println!("已从配置中删除 {name}");
    }
    println!(
        "\n完成。{name} 写在 {user} 个人文件夹里的数据（例如登录信息）没有删除，\n\
         卸载整个 isobox 时会一起清掉。"
    );
    Ok(())
}

/// 以沙盒用户身份运行安装目录里的卸载程序
fn run_uninstaller(sb: &Sandbox, exe: &Path) -> Result<()> {
    let dir = exe.parent().context("主程序没有父目录")?;
    let uninst = std::fs::read_dir(dir)?.flatten().map(|e| e.path()).find(|p| {
        let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        n.ends_with(".exe") && (n.starts_with("unins") || n.starts_with("uninstall"))
    });
    match uninst {
        Some(u) => {
            println!("正在以 {} 身份运行卸载程序 {}，请在弹出的窗口里完成卸载", sb.user, u.display());
            let pw = cred::load(&sb.user)?;
            let code = launch::launch_as(&sb.user, &pw, &u, "", dir)?.wait()?;
            println!("卸载程序已退出，退出码 {code}");
        }
        None => println!("没有找到卸载程序，程序文件留在 {}", dir.display()),
    }
    Ok(())
}

/// 把绿色版软件移动到另一个盘的库。也用于把旧位置（个人文件夹）的软件迁移进库
pub fn move_app(cfg_path: &Path, cfg: &Config, name: &str, to: char) -> Result<()> {
    let to = to.to_ascii_uppercase();
    let a = cfg.apps.get(name).with_context(|| format!("配置里没有 {name}"))?;
    if !a.portable {
        bail!(
            "{name} 是用原版安装程序装的，安装程序在注册表里记下了它的位置，移动后可能无法运行或卸载。\n\
             如果要换盘，请移除后重新添加"
        );
    }
    let sb = sandbox(cfg)?;
    if a.library_drive() == Some(to) {
        bail!("{name} 已经在 {to} 盘的库里");
    }
    let src = portable_dir(&sb, name, a);
    if !src.exists() {
        bail!("找不到 {name} 的程序目录 {}", src.display());
    }
    let lib = ensure_library(cfg, to)?;
    let dst = lib.apps_dir().join(name);
    if dst.exists() {
        bail!("{} 已经存在，请先检查是否是之前移动失败留下的", dst.display());
    }
    let exe_name = a.exe_name()?;
    kill_app(&sb.user, &exe_name);

    println!("正在移动 {} 到 {}", src.display(), dst.display());
    let same_volume = crate::paths::drive_letter(&src) == Some(to);
    if same_volume {
        std::fs::rename(&src, &dst).with_context(|| format!("移动失败，确认 {name} 已完全退出"))?;
    } else {
        // 跨盘：robocopy 移动，返回码小于 8 都表示成功
        let status = Command::new("robocopy")
            .args([
                &src.display().to_string(),
                &dst.display().to_string(),
                "/E",
                "/MOVE",
                "/R:1",
                "/W:1",
                "/NFL",
                "/NDL",
                "/NJH",
                "/NJS",
                "/NP",
            ])
            .status()
            .context("无法运行 robocopy")?;
        if status.code().unwrap_or(16) >= 8 {
            bail!("robocopy 移动失败，返回码 {status}。原目录 {} 仍保留", src.display());
        }
        if src.exists() {
            let _ = std::fs::remove_dir_all(&src);
        }
    }
    // 移动过来的文件可能还带着旧位置的权限，重新继承库里 apps 目录的权限
    acl::reset_tree(&dst)?;
    config::set_app_field(cfg_path, name, "library", toml::Value::from(to.to_string()))?;

    let exe = dst.join(&exe_name);
    if let Ok(icon) = shortcut::icon_path(name) {
        let _ = std::fs::remove_file(icon);
    }
    let updated = shortcut::update_for_app(name, Some(&exe))?;
    println!("已移动到 {to} 盘的库，更新了 {} 个快捷方式", updated.len());
    Ok(())
}
