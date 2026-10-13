//! 管理员工具：安装和卸载隔离账户、管理库、添加和移除软件、备份和还原权限。
//! install 和 uninstall 默认只打印计划，加 --apply 才真正修改系统。

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use isobox::{
    account::{lookup_sid, owner_sid, profile_path},
    acl,
    apps::{self, AddOptions},
    config::Config,
    cred, drives, launch, library, paths, shortcut,
    wide::to_wide,
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        NetworkManagement::NetManagement::{
            NERR_Success, NetApiBufferFree, NetUserAdd, NetUserDel, NetUserGetInfo,
            UF_DONT_EXPIRE_PASSWD, UF_SCRIPT, USER_INFO_1, USER_PRIV_USER,
        },
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
        UI::Shell::DeleteProfileW,
    },
    core::{PCWSTR, PWSTR},
};

/// 卸载时在这些位置查找沙盒用户留下的文件夹。它们不属于个人文件夹，删除账户不会带走
const SHARED_DIRS: &[&str] = &[r"C:\ProgramData", r"C:\Users\Public"];

/// 写进账户备注，卸载时据此确认这个账户确实是 isobox 创建的
const MARKER: &str = "isobox sandbox account";

const HIDE_KEY: &str =
    r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList";

/// 旧版本安装目录里属于 IsoBox 的文件，迁移后删除
const OLD_INSTALL_FILES: &[&str] = &[
    "isobox.exe",
    "isobox-setup.exe",
    "isobox-launch.exe",
    "add-app.ps1",
    "remove-app.ps1",
    "isobox.toml",
];

#[derive(Parser)]
#[command(name = "isobox-setup", about = "IsoBox 管理员工具：隔离账户、库、软件、权限备份")]
struct Cli {
    /// 配置文件路径，默认找 exe 旁边的 isobox.toml
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    /// install、uninstall 真正执行。不加时只打印将要做的事
    #[arg(long, global = true)]
    apply: bool,
    /// 把所有输出写进这个文件。图形界面以管理员身份隐藏运行本程序时使用
    #[arg(long, global = true)]
    log: Option<PathBuf>,
    /// 不等待键盘输入：需要选择时自动选推荐项
    #[arg(long, global = true)]
    non_interactive: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 创建隔离账户和系统盘的库，禁止它访问要保护的位置，建立交换文件夹
    Install,
    /// 按安全顺序撤销 install 做的所有改动，删除所有库
    Uninstall {
        /// 账户已被手动删除时，用它的 SID 清理残留权限，例如 S-1-5-21-...-1001
        #[arg(long)]
        sid: Option<String>,
    },
    /// 列出所有盘，以及能否保护、能否建库
    Drives,
    /// 管理库：每个盘上固定位置 <盘>:\Program Files\IsoBox
    Library {
        #[command(subcommand)]
        cmd: LibCmd,
    },
    /// 添加软件：快速安装（拆包）或运行原版安装程序，写入配置，创建快捷方式
    Add {
        /// 配置里的英文名，也用作文件夹名，例如 baidunetdisk
        name: String,
        /// 官方安装包路径
        installer: PathBuf,
        /// 快捷方式显示的名字，例如 百度网盘
        #[arg(long)]
        display: Option<String>,
        /// 主程序文件名。不写的话会列出候选让你选
        #[arg(long)]
        exe: Option<String>,
        /// 不拆包，改为以隔离账户身份运行原版安装程序（图形化安装）
        #[arg(long)]
        install: bool,
        /// 装进哪个盘的库，例如 D。默认系统盘
        #[arg(long)]
        library: Option<char>,
        /// 同时加入开机启动
        #[arg(long)]
        startup: bool,
        /// 覆盖同名的已有软件
        #[arg(long)]
        force: bool,
    },
    /// 移除一个软件：程序文件、快捷方式和配置
    RemoveApp { name: String },
    /// 把软件移动到另一个盘的库
    MoveApp {
        name: String,
        /// 目标盘符，例如 D
        #[arg(long)]
        to: char,
    },
    /// 从旧版本的安装位置迁移：配置、软件、快捷方式，然后删除旧文件
    Migrate {
        /// 旧安装目录，例如 "D:\Program Files\IsoBox"
        #[arg(long)]
        from: PathBuf,
    },
    /// 管理受保护的位置
    Protect {
        #[command(subcommand)]
        cmd: ProtectCmd,
    },
    /// 更换交换文件夹：授权新文件夹，撤销旧文件夹的授权，写入配置
    SetExchange { path: PathBuf },
    /// 备份某个目录或整个盘的权限
    BackupAcl { path: PathBuf },
    /// 用备份还原权限。参数是 backup 目录下的某个子文件夹
    RestoreAcl { backup: PathBuf },
}

#[derive(Subcommand)]
enum ProtectCmd {
    /// 列出受保护的位置
    List,
    /// 禁止隔离账户访问某个位置，例如 isobox-setup protect add D:\  修改前自动备份权限
    Add { path: PathBuf },
    /// 撤销对某个位置的保护
    Remove { path: PathBuf },
}

#[derive(Subcommand)]
enum LibCmd {
    /// 列出库
    List,
    /// 在某个盘上建库，例如 isobox-setup library add D
    Add { drive: char },
    /// 移除某个盘的库，库里必须没有软件
    Remove { drive: char },
}

enum Step {
    CreateUser,
    HideFromLogin,
    CreateProfile,
    CreateLibrary(char),
    BackupAcl(PathBuf),
    Deny(PathBuf),
    MakeDir(PathBuf),
    GrantExchange(PathBuf),
    KillProcesses,
    RemoveTasks,
    RemoveDeny(PathBuf, String),
    RemoveGrant(PathBuf, String),
    RemoveLibrary(library::Library),
    RemoveShortcuts,
    DeletePath(PathBuf),
    DeleteProfile(String),
    DeleteUser,
    UnhideFromLogin,
    DeleteCredential,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(log) = &cli.log {
        redirect_output(log)?;
    }
    if cli.non_interactive {
        // SAFETY: 单线程，程序刚启动，还没有其他线程读取环境变量
        unsafe { std::env::set_var("ISOBOX_NONINTERACTIVE", "1") };
    }
    let cfg_path = cli.config.clone().unwrap_or_else(Config::default_path);
    let cfg = Config::load(&cfg_path)?;

    let me = std::env::var("USERNAME").unwrap_or_default();
    if cfg.sandbox_user.eq_ignore_ascii_case(&me) {
        bail!("sandbox_user 不能是当前登录的账户 {me}，请检查配置");
    }

    match cli.cmd {
        Cmd::Drives => {
            print_drives();
            Ok(())
        }
        Cmd::Library { cmd: LibCmd::List } => {
            print_libraries(&cfg);
            Ok(())
        }
        Cmd::Library { cmd: LibCmd::Add { drive } } => {
            admin()?;
            let sid = lookup_sid(&cfg.sandbox_user).context("隔离账户不存在，请先运行 install")?;
            let lib = library::create(drive, &cfg.sandbox_user, &sid)?;
            println!("已建库 {}", lib.root.display());
            Ok(())
        }
        Cmd::Library { cmd: LibCmd::Remove { drive } } => {
            admin()?;
            let sid = lookup_sid(&cfg.sandbox_user).context("隔离账户不存在")?;
            library::remove(drive, &sid)?;
            println!("已移除 {} 盘的库", drive.to_ascii_uppercase());
            Ok(())
        }
        Cmd::Add { name, installer, display, exe, install, library, startup, force } => {
            admin()?;
            let o = AddOptions { name, installer, display, exe, install, startup, force, library };
            apps::add(&cfg_path, &cfg, &o)
        }
        Cmd::RemoveApp { name } => {
            admin()?;
            apps::remove(&cfg_path, &cfg, &name)
        }
        Cmd::MoveApp { name, to } => {
            admin()?;
            apps::move_app(&cfg_path, &cfg, &name, to)
        }
        Cmd::Migrate { from } => {
            admin()?;
            migrate(&cfg_path, &from)
        }
        Cmd::Protect { cmd: ProtectCmd::List } => {
            let st = isobox::status::overview(&cfg_path, &cfg, false);
            if st.protected.is_empty() {
                println!("没有受保护的位置");
            }
            for p in st.protected {
                println!(
                    "{}  {}{}",
                    p.path,
                    if p.active { "已保护" } else { "未生效" },
                    if p.in_config { "" } else { "（不在配置里）" }
                );
            }
            Ok(())
        }
        Cmd::Protect { cmd: ProtectCmd::Add { path } } => {
            admin()?;
            protect_add(&cfg_path, &cfg, &path)
        }
        Cmd::Protect { cmd: ProtectCmd::Remove { path } } => {
            admin()?;
            protect_remove(&cfg_path, &cfg, &path)
        }
        Cmd::SetExchange { path } => {
            admin()?;
            set_exchange(&cfg_path, &cfg, &path)
        }
        Cmd::BackupAcl { path } => {
            admin()?;
            println!("正在备份 {} 的权限，文件多的话需要一些时间……", path.display());
            let dir = acl::backup(&path, &backup_root()?)?;
            println!("已备份到 {}", dir.display());
            Ok(())
        }
        Cmd::RestoreAcl { backup } => {
            admin()?;
            let target = acl::restore(&backup)?;
            println!("已还原 {} 的权限", target.display());
            Ok(())
        }
        Cmd::Install => {
            let steps = install_plan(&cfg)?;
            confirm_and_run(&steps, &cfg, cli.apply, true)?;
            if cli.apply {
                println!(
                    "\n完成。下一步：用开始菜单的“IsoBox 添加软件”，或 isobox-setup add <名字> <安装包> 添加软件。"
                );
            }
            Ok(())
        }
        Cmd::Uninstall { sid } => {
            let steps = uninstall_plan(&cfg, sid)?;
            confirm_and_run(&steps, &cfg, cli.apply, false)?;
            if cli.apply {
                if let Ok(x) = cfg.exchange_dir() {
                    println!("\n交换文件夹 {} 里的文件没有删除，确认不需要后可手动删掉。", x.display());
                }
                println!("卸载完成。");
            }
            Ok(())
        }
    }
}

fn admin() -> Result<()> {
    if !is_elevated()? {
        bail!("需要管理员权限，请在“以管理员身份运行”的终端里执行");
    }
    paths::require_installed()?;
    Ok(())
}

fn backup_root() -> Result<PathBuf> {
    Ok(paths::install_dir()?.join("backup"))
}

/// 打印计划；带 --apply 时执行。安装遇错即停，卸载尽量做完每一步
fn confirm_and_run(steps: &[Step], cfg: &Config, apply: bool, stop_on_error: bool) -> Result<()> {
    println!("计划：");
    for s in steps {
        println!("  - {}", describe(s, &cfg.sandbox_user));
    }
    if !apply {
        println!("\n这是预演。确认无误后在管理员终端里加 --apply 运行。");
        return Ok(());
    }
    admin()?;
    let mut failed = Vec::new();
    for s in steps {
        if let Err(e) = execute(s, cfg) {
            if stop_on_error {
                return Err(e).with_context(|| describe(s, &cfg.sandbox_user));
            }
            eprintln!("  失败: {}: {e:#}", describe(s, &cfg.sandbox_user));
            failed.push(describe(s, &cfg.sandbox_user));
        }
    }
    if !failed.is_empty() {
        bail!("{} 个步骤失败，修正后可再次运行，已完成的步骤会被跳过或无害重复", failed.len());
    }
    Ok(())
}

/// 把所有输出写进文件：图形界面以管理员身份隐藏运行时，用它取回输出。
/// Rust 每次写标准输出都会重新取句柄，所以替换系统的标准句柄就够了，子进程也会继承
fn redirect_output(log: &Path) -> Result<()> {
    use std::os::windows::io::IntoRawHandle;
    use windows::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};
    let f = std::fs::File::create(log).with_context(|| format!("无法创建日志 {}", log.display()))?;
    let h = HANDLE(f.into_raw_handle());
    unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, h)?;
        SetStdHandle(STD_ERROR_HANDLE, h)?;
    }
    Ok(())
}

/// 当前配置下要保护的位置（把“自动”展开成具体的盘）
fn current_protect_list(cfg: &Config) -> Vec<PathBuf> {
    if cfg.deny_paths.is_empty() && cfg.protect_auto() {
        drives::list()
            .into_iter()
            .filter(|d| d.protect_blocker().is_none())
            .map(|d| PathBuf::from(d.root()))
            .collect()
    } else {
        cfg.deny_paths.clone()
    }
}

/// 写回保护列表。手动改过之后就不再自动选择，避免以后重新安装时又把移除的盘加回来
fn save_protect_list(cfg_path: &Path, list: &[PathBuf]) -> Result<()> {
    let arr = toml::Value::Array(list.iter().map(|p| toml::Value::from(p.display().to_string())).collect());
    isobox::config::set_top_level(cfg_path, "deny_paths", arr)?;
    isobox::config::set_top_level(cfg_path, "protect_auto", toml::Value::from(false))
}

fn protect_add(cfg_path: &Path, cfg: &Config, path: &Path) -> Result<()> {
    check_protectable(path)?;
    let sid = lookup_sid(&cfg.sandbox_user).context("隔离账户不存在，请先运行 install")?;
    if acl::has_explicit_deny(path, &sid) {
        println!("{} 已经受保护", path.display());
    } else {
        println!("正在备份 {} 的权限，文件多的话需要一些时间……", path.display());
        let dir = acl::backup(path, &backup_root()?)?;
        println!("已备份到 {}", dir.display());
        acl::icacls(&[&path.display().to_string(), "/deny", &format!("{}:(OI)(CI)F", cfg.sandbox_user), "/q"])?;
        println!("已禁止 {} 访问 {}", cfg.sandbox_user, path.display());
    }
    let mut list = current_protect_list(cfg);
    if !list.iter().any(|p| paths::same_path(p, path)) {
        list.push(path.to_path_buf());
    }
    save_protect_list(cfg_path, &list)
}

fn protect_remove(cfg_path: &Path, cfg: &Config, path: &Path) -> Result<()> {
    let sid = lookup_sid(&cfg.sandbox_user).context("隔离账户不存在")?;
    if acl::has_explicit_deny(path, &sid) {
        acl::icacls(&[&path.display().to_string(), "/remove:d", &format!("*{sid}"), "/q"])?;
        println!("已撤销 {} 上的禁止规则", path.display());
    } else {
        println!("{} 上没有禁止规则", path.display());
    }
    let list: Vec<PathBuf> = current_protect_list(cfg).into_iter().filter(|p| !paths::same_path(p, path)).collect();
    save_protect_list(cfg_path, &list)
}

fn set_exchange(cfg_path: &Path, cfg: &Config, path: &Path) -> Result<()> {
    let sid = lookup_sid(&cfg.sandbox_user).context("隔离账户不存在，请先运行 install")?;
    let old = cfg.exchange_dir().ok();
    std::fs::create_dir_all(path).with_context(|| format!("创建 {} 失败", path.display()))?;
    acl::icacls(&[&path.display().to_string(), "/grant", &format!("{}:(OI)(CI)M", cfg.sandbox_user), "/q"])?;
    println!("已允许 {} 读写 {}", cfg.sandbox_user, path.display());
    if let Some(old) = old.filter(|o| !paths::same_path(o, path) && o.exists()) {
        acl::icacls(&[&old.display().to_string(), "/remove:g", &format!("*{sid}"), "/q"])?;
        println!("已撤销对旧交换文件夹 {} 的授权，里面的文件保留", old.display());
    }
    isobox::config::set_top_level(cfg_path, "exchange_dir", toml::Value::from(path.display().to_string()))
}

/// 要保护的位置：配置里写了就用配置的，否则自动选所有能保护的固定硬盘
fn protect_targets(cfg: &Config) -> Result<Vec<PathBuf>> {
    if cfg.deny_paths.is_empty() && !cfg.protect_auto() {
        return Ok(Vec::new());
    }
    if cfg.deny_paths.is_empty() {
        let auto: Vec<PathBuf> = drives::list()
            .into_iter()
            .filter(|d| d.protect_blocker().is_none())
            .map(|d| PathBuf::from(d.root()))
            .collect();
        return Ok(auto);
    }
    let mut out = Vec::new();
    for p in &cfg.deny_paths {
        if std::fs::metadata(p).is_err() {
            eprintln!("注意：{} 当前不可用，跳过。接上后可再次运行 install", p.display());
            continue;
        }
        check_protectable(p)?;
        out.push(p.clone());
    }
    Ok(out)
}

/// 能否给这个位置加禁止规则：文件系统要支持权限；不能是系统目录或其上级
fn check_protectable(p: &Path) -> Result<()> {
    let letter = paths::drive_letter(p).with_context(|| format!("{} 不是本地路径", p.display()))?;
    let d = drives::get(letter).with_context(|| format!("没有 {letter} 盘"))?;
    let is_root = p.to_string_lossy().trim_end_matches('\\').len() <= 2;
    if is_root {
        if let Some(why) = d.protect_blocker() {
            bail!("{} 不能保护：{why}", p.display());
        }
    } else if !d.acl {
        bail!("{} 所在的盘不支持权限控制", p.display());
    }
    let env_dir = |k: &str| std::env::var(k).ok().map(PathBuf::from);
    let critical: Vec<PathBuf> = [
        env_dir("SystemRoot"),
        env_dir("ProgramW6432"),
        env_dir("ProgramFiles(x86)"),
        env_dir("ProgramData"),
        Some(PathBuf::from(format!("{}:\\Users", drives::system_drive()))),
    ]
    .into_iter()
    .flatten()
    .collect();
    let lp = p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    for c in critical {
        let lc = c.to_string_lossy().trim_end_matches('\\').to_lowercase();
        if lc == lp || lc.starts_with(&format!("{lp}\\")) {
            bail!("{} 包含系统需要的目录 {}，不能保护", p.display(), c.display());
        }
    }
    Ok(())
}

fn install_plan(cfg: &Config) -> Result<Vec<Step>> {
    let mut steps = vec![
        Step::CreateUser,
        Step::HideFromLogin,
        Step::CreateProfile,
        Step::CreateLibrary(drives::system_drive()),
    ];
    let targets = protect_targets(cfg)?;
    if targets.is_empty() {
        eprintln!("注意：没有要保护的硬盘。你的用户目录默认就是隔离的");
    }
    for p in targets {
        steps.push(Step::BackupAcl(p.clone()));
        steps.push(Step::Deny(p));
    }
    let x = cfg.exchange_dir()?;
    steps.push(Step::MakeDir(x.clone()));
    steps.push(Step::GrantExchange(x));
    Ok(steps)
}

/// 顺序很重要：先用 SID 撤销权限、删配置文件，最后才删用户。
/// 反过来做的话，权限规则会变成指向不存在账户的“未知 SID”。
fn uninstall_plan(cfg: &Config, sid_arg: Option<String>) -> Result<Vec<Step>> {
    let user = &cfg.sandbox_user;
    let sid = match lookup_sid(user) {
        Ok(s) => {
            // 账户存在时，必须带有 isobox 写入的备注，否则拒绝动它
            let comment = user_comment(user)?;
            if comment != MARKER {
                bail!(
                    "账户 {user} 的备注是“{comment}”，不是 isobox 创建的，拒绝卸载。\n\
                     如果确认要清理，请先手动把它的备注改成“{MARKER}”"
                );
            }
            Some(s)
        }
        Err(_) => sid_arg,
    };
    // 防止误传当前账户的 SID：那样会把你自己在 ProgramData 里的文件夹和个人文件夹列入删除
    if let Some(s) = &sid {
        let me = lookup_sid(&std::env::var("USERNAME").unwrap_or_default()).unwrap_or_default();
        if s.eq_ignore_ascii_case(&me) {
            bail!("{s} 是当前登录账户的 SID，拒绝卸载");
        }
    }

    let mut steps = Vec::new();
    match &sid {
        Some(sid) => {
            steps.push(Step::KillProcesses);
            steps.push(Step::RemoveTasks);
            // 禁止规则：配置里写的位置，加上所有盘的根目录，凡是带这个 SID 的都撤销
            let mut candidates: Vec<PathBuf> = cfg.deny_paths.clone();
            candidates.extend(drives::list().into_iter().filter(|d| d.ready && d.acl).map(|d| PathBuf::from(d.root())));
            let mut seen = Vec::<String>::new();
            for p in candidates {
                let key = p.to_string_lossy().trim_end_matches('\\').to_lowercase();
                if seen.contains(&key) || !acl::has_explicit_deny(&p, sid) {
                    continue;
                }
                seen.push(key);
                steps.push(Step::RemoveDeny(p, sid.clone()));
            }
            if let Ok(x) = cfg.exchange_dir()
                && x.exists() {
                    steps.push(Step::RemoveGrant(x, sid.clone()));
                }
            for lib in library::discover(sid) {
                steps.push(Step::RemoveLibrary(lib));
            }
            for p in find_owned(sid) {
                steps.push(Step::DeletePath(p));
            }
            // 个人文件夹里包括旧位置 Apps 下的程序和所有聊天记录等数据
            steps.push(Step::DeleteProfile(sid.clone()));
        }
        None => eprintln!(
            "注意：系统里找不到用户 {user}，也没有给出 --sid，将跳过权限和配置文件清理。\n\
             查找残留 SID 的方法：icacls D:\\ 输出里形如 S-1-5-21-... 的条目。"
        ),
    }
    if sid.is_some() && lookup_sid(user).is_ok() {
        steps.push(Step::DeleteUser);
    }
    steps.push(Step::RemoveShortcuts);
    if let Ok(icons) = shortcut::icon_path("x").map(|p| p.parent().unwrap().to_path_buf())
        && icons.exists() {
            steps.push(Step::DeletePath(icons));
        }
    steps.push(Step::UnhideFromLogin);
    steps.push(Step::DeleteCredential);
    Ok(steps)
}

fn describe(s: &Step, user: &str) -> String {
    match s {
        Step::CreateUser => format!("创建标准用户 {user}，随机密码，存入凭据管理器"),
        Step::HideFromLogin => format!("在登录界面隐藏 {user}"),
        Step::CreateProfile => format!("以 {user} 身份登录一次，创建它的个人文件夹"),
        Step::CreateLibrary(d) => format!("在 {d} 盘建库 {}", library::root(*d).display()),
        Step::BackupAcl(p) => format!("备份 {} 的权限", p.display()),
        Step::Deny(p) => format!("禁止 {user} 访问 {}", p.display()),
        Step::MakeDir(d) => format!("创建交换文件夹 {}", d.display()),
        Step::GrantExchange(d) => format!("允许 {user} 读写交换文件夹 {}", d.display()),
        Step::KillProcesses => format!("结束所有以 {user} 身份运行的进程，以及后台的主题监视进程"),
        Step::RemoveTasks => format!("删除以 {user} 身份运行的计划任务（执行时查找）"),
        Step::RemoveDeny(p, _) => format!("撤销 {} 上针对 {user} 的禁止规则", p.display()),
        Step::RemoveGrant(p, _) => format!("撤销 {user} 对交换文件夹 {} 的授权", p.display()),
        Step::RemoveLibrary(l) => {
            let apps = l.apps();
            if apps.is_empty() {
                format!("删除 {} 盘的库", l.drive)
            } else {
                format!("删除 {} 盘的库及其中的软件：{}", l.drive, apps.join("、"))
            }
        }
        Step::RemoveShortcuts => "删除桌面、开始菜单和开机启动里的 IsoBox 快捷方式".into(),
        Step::DeletePath(p) => format!("删除 {user} 留下的 {}", p.display()),
        Step::DeleteProfile(sid) => format!("删除 {user} 的个人文件夹和注册表配置（{sid}）"),
        Step::DeleteUser => format!("删除用户 {user}"),
        Step::UnhideFromLogin => "删除登录界面隐藏设置".into(),
        Step::DeleteCredential => format!("删除凭据管理器里的 isobox:{user}"),
    }
}

fn execute(s: &Step, cfg: &Config) -> Result<()> {
    let user = cfg.sandbox_user.as_str();
    match s {
        Step::CreateUser => create_user(user),
        Step::HideFromLogin => run("reg", &["add", HIDE_KEY, "/v", user, "/t", "REG_DWORD", "/d", "0", "/f"]),
        Step::CreateProfile => {
            let pw = cred::load(user)?;
            let sys32 = Path::new(r"C:\Windows\System32");
            let code = launch::launch_as_hidden(user, &pw, &sys32.join("cmd.exe"), "/c exit", sys32)?.wait()?;
            let sid = lookup_sid(user)?;
            match profile_path(&sid)? {
                Some(p) => {
                    println!("  个人文件夹: {}（cmd 退出码 {code}）", p.display());
                    Ok(())
                }
                None => bail!("以 {user} 身份登录后仍没有个人文件夹，cmd 退出码 {code}"),
            }
        }
        Step::CreateLibrary(d) => {
            let sid = lookup_sid(user)?;
            let lib = library::create(*d, user, &sid)?;
            println!("  库: {}", lib.root.display());
            Ok(())
        }
        Step::BackupAcl(p) => {
            println!("  正在备份 {} 的权限，文件多的话需要一些时间……", p.display());
            let dir = acl::backup(p, &backup_root()?)?;
            println!("  已备份到 {}", dir.display());
            Ok(())
        }
        Step::Deny(p) => {
            // (OI)(CI) 让禁止规则继承到所有子文件和子目录。只改顶层，Windows 会自动传播
            acl::icacls(&[&p.display().to_string(), "/deny", &format!("{user}:(OI)(CI)F"), "/q"])?;
            Ok(())
        }
        Step::MakeDir(d) => std::fs::create_dir_all(d).with_context(|| format!("创建 {} 失败", d.display())),
        Step::GrantExchange(d) => {
            // 显式允许排在继承来的禁止之前，所以交换文件夹即使在被保护的盘里也能访问
            acl::icacls(&[&d.display().to_string(), "/grant", &format!("{user}:(OI)(CI)M"), "/q"])?;
            Ok(())
        }
        Step::RemoveDeny(p, sid) => {
            acl::icacls(&[&p.display().to_string(), "/remove:d", &format!("*{sid}"), "/q"])?;
            Ok(())
        }
        Step::RemoveGrant(p, sid) => {
            acl::icacls(&[&p.display().to_string(), "/remove:g", &format!("*{sid}"), "/q"])?;
            Ok(())
        }
        Step::RemoveLibrary(lib) => library::remove_files(lib),
        Step::RemoveShortcuts => {
            for p in shortcut::remove_all()? {
                println!("  已删除 {}", p.display());
            }
            Ok(())
        }
        Step::RemoveTasks => {
            let sid = lookup_sid(user)?;
            let script = format!(
                "$t = Get-ScheduledTask | Where-Object {{ $_.Principal.UserId -match '(^|\\\\){user}$' -or $_.Principal.UserId -eq '{sid}' }}; \
                 $t | ForEach-Object {{ 'removing task ' + $_.TaskPath + $_.TaskName }}; \
                 $t | Unregister-ScheduledTask -Confirm:$false"
            );
            run("powershell.exe", &["-NoProfile", "-Command", &script])
        }
        Step::DeletePath(p) => {
            if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
                .with_context(|| format!("删除 {} 失败", p.display()))
        }
        Step::KillProcesses => {
            // 没有匹配进程时 taskkill 也可能返回非零，这里不当作错误
            let _ = Command::new("taskkill").args(["/f", "/t", "/fi", &format!("USERNAME eq {user}")]).output();
            // 你的账户下的主题监视进程也一并结束
            let _ = Command::new("taskkill").args(["/f", "/im", shortcut::LAUNCHER_EXE]).output();
            std::thread::sleep(std::time::Duration::from_secs(2));
            Ok(())
        }
        Step::DeleteProfile(sid) => {
            let sid_w = to_wide(sid);
            match unsafe { DeleteProfileW(PCWSTR(sid_w.as_ptr()), PCWSTR::null(), PCWSTR::null()) } {
                Ok(()) => Ok(()),
                // 从未登录过就没有配置文件，ERROR_FILE_NOT_FOUND 视为成功
                Err(e) if e.code().0 as u32 == 0x8007_0002 => Ok(()),
                Err(e) => Err(e).context("删除配置文件失败，确认没有 isobox 的进程仍在运行"),
            }
        }
        Step::DeleteUser => {
            let name_w = to_wide(user);
            let rc = unsafe { NetUserDel(PCWSTR::null(), PCWSTR(name_w.as_ptr())) };
            if rc != NERR_Success {
                bail!("NetUserDel 失败，错误码 {rc}");
            }
            Ok(())
        }
        Step::UnhideFromLogin => {
            // 值不存在时 reg delete 会失败，这里先查再删
            let exists = Command::new("reg")
                .args(["query", HIDE_KEY, "/v", user])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if exists { run("reg", &["delete", HIDE_KEY, "/v", user, "/f"]) } else { Ok(()) }
        }
        Step::DeleteCredential => cred::delete(user).map(|_| ()),
    }
}

fn print_drives() {
    println!("{:<4} {:<10} {:<6} {:>9} {:>9}  {:<8} 建库", "盘", "类型", "格式", "总容量", "剩余", "保护");
    for d in drives::list() {
        let yes_no = |b: Option<&str>| match b {
            None => "可以".to_string(),
            Some(why) => format!("不能（{why}）"),
        };
        println!(
            "{:<4} {:<10} {:<6} {:>9} {:>9}  {:<8} {}",
            format!("{}:", d.letter),
            d.describe_kind(),
            if d.ready { d.fs.as_str() } else { "-" },
            if d.ready { drives::human_size(d.total) } else { "-".into() },
            if d.ready { drives::human_size(d.free) } else { "-".into() },
            yes_no(d.protect_blocker()),
            yes_no(d.library_blocker()),
        );
    }
}

fn print_libraries(cfg: &Config) {
    let Ok(sid) = lookup_sid(&cfg.sandbox_user) else {
        println!("隔离账户 {} 不存在，还没有库", cfg.sandbox_user);
        return;
    };
    let libs = library::discover(&sid);
    if libs.is_empty() {
        println!("还没有库");
    }
    for l in libs {
        let free = drives::get(l.drive).map(|d| drives::human_size(d.free)).unwrap_or_default();
        let apps = l.apps();
        println!(
            "{} 盘  {}  剩余 {free}  软件: {}",
            l.drive,
            l.root.display(),
            if apps.is_empty() { "无".to_string() } else { apps.join("、") }
        );
    }
}

/// 从旧版本的安装目录迁移到当前的固定安装目录
fn migrate(cfg_path: &Path, from: &Path) -> Result<()> {
    let here = paths::install_dir()?;
    if paths::same_path(from, &here) {
        bail!("旧目录和当前安装目录相同，不需要迁移");
    }
    let old_cfg = from.join("isobox.toml");
    if !old_cfg.exists() {
        // 上次迁移已经完成、只差删除旧目录时，再运行一次就只做清理
        let only_ours = std::fs::read_dir(from).is_ok_and(|it| {
            it.flatten().all(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                n == "icons" || OLD_INSTALL_FILES.contains(&n.as_str())
            })
        });
        if from.exists() && only_ours {
            println!("{} 里只剩 IsoBox 的残留文件，只做清理", from.display());
            cleanup_old_install(from);
            return Ok(());
        }
        bail!("{} 里没有 isobox.toml，不像是 IsoBox 的旧安装目录", from.display());
    }
    // 旧目录里的主题监视进程会占用文件
    let _ = Command::new("taskkill").args(["/f", "/im", shortcut::LAUNCHER_EXE]).output();
    std::thread::sleep(std::time::Duration::from_secs(1));

    // 1. 配置：用旧配置替换新目录里的模板，模板留个备份
    if cfg_path.exists() {
        let bak = cfg_path.with_extension("toml.template");
        std::fs::copy(cfg_path, &bak)?;
    }
    std::fs::copy(&old_cfg, cfg_path).context("复制旧配置失败")?;
    println!("已迁移配置 {} -> {}", old_cfg.display(), cfg_path.display());
    let cfg = Config::load(cfg_path)?;

    // 2. 系统盘的库
    let sys = drives::system_drive();
    apps::ensure_library(&cfg, sys)?;

    // 3. 旧位置（个人文件夹）里的绿色版软件移进系统盘的库；其余软件只更新快捷方式
    let names: Vec<String> = cfg.apps.keys().cloned().collect();
    for name in names {
        let a = &cfg.apps[&name];
        let legacy = a.portable && a.mode(&cfg) == isobox::config::Mode::User && a.library.is_none();
        if legacy {
            if let Err(e) = apps::move_app(cfg_path, &cfg, &name, sys) {
                eprintln!("移动 {name} 失败: {e:#}");
            }
        } else {
            // 库里的绿色版用库里的主程序做图标；原版安装的用配置里的完整路径
            let exe = match (a.portable, a.library_drive()) {
                (true, Some(d)) => a.exe_name().ok().map(|n| library::app_dir(d, &name).join(n)),
                (false, _) => Some(a.exe.clone()),
                _ => None,
            };
            let n = shortcut::update_for_app(&name, exe.as_deref())?.len();
            if n > 0 {
                println!("已更新 {name} 的 {n} 个快捷方式");
            }
        }
    }

    // 4. 删除旧安装目录里属于 IsoBox 的文件。失败不影响迁移结果，只提示手动处理
    cleanup_old_install(from);
    println!("\n迁移完成。");
    Ok(())
}

/// 删除旧安装目录里属于 IsoBox 的文件，然后删除空目录。
/// 常见的失败原因是某个终端或资源管理器窗口正停在这个目录里，这时只提示，不当作错误
fn cleanup_old_install(from: &Path) {
    let mut problems = Vec::new();
    for f in OLD_INSTALL_FILES {
        let p = from.join(f);
        if p.exists()
            && let Err(e) = std::fs::remove_file(&p) {
                problems.push(format!("{}: {e}", p.display()));
            }
    }
    let icons = from.join("icons");
    if icons.exists()
        && let Err(e) = std::fs::remove_dir_all(&icons) {
            problems.push(format!("{}: {e}", icons.display()));
        }
    let empty = std::fs::read_dir(from).is_ok_and(|mut it| it.next().is_none());
    if empty {
        match std::fs::remove_dir(from) {
            Ok(()) => println!("已删除旧安装目录 {}", from.display()),
            Err(e) => problems.push(format!("{}: {e}", from.display())),
        }
    } else if problems.is_empty() {
        println!("旧安装目录 {} 里还有其他文件，保留了这个目录", from.display());
    }
    if !problems.is_empty() {
        println!("\n注意：旧安装目录没能完全删除，不影响使用：");
        for p in &problems {
            println!("  {p}");
        }
        println!(
            "通常是某个终端或资源管理器窗口正停在这个目录里。关闭或切换到别的目录后，\n\
             在管理员终端里运行下面的命令即可：\n  Remove-Item -LiteralPath \"{}\" -Recurse -Force",
            from.display()
        );
    }
}

fn create_user(user: &str) -> Result<()> {
    let password = random_password()?;
    let mut name_w = to_wide(user);
    let mut pass_w = to_wide(&password);
    let mut comment_w = to_wide(MARKER);
    let info = USER_INFO_1 {
        usri1_name: PWSTR(name_w.as_mut_ptr()),
        usri1_password: PWSTR(pass_w.as_mut_ptr()),
        usri1_priv: USER_PRIV_USER,
        usri1_comment: PWSTR(comment_w.as_mut_ptr()),
        usri1_flags: UF_SCRIPT | UF_DONT_EXPIRE_PASSWD,
        ..Default::default()
    };
    let mut parm_err = 0u32;
    let rc = unsafe { NetUserAdd(None, 1, &info as *const _ as *const u8, Some(&mut parm_err)) };
    if rc != NERR_Success {
        bail!("NetUserAdd 失败，错误码 {rc}，2224 表示用户已存在");
    }
    cred::store(user, &password)?;
    println!("  已创建用户 {user}，密码已存入凭据管理器");
    Ok(())
}

fn user_comment(user: &str) -> Result<String> {
    let name_w = to_wide(user);
    let mut buf: *mut u8 = std::ptr::null_mut();
    let rc = unsafe { NetUserGetInfo(PCWSTR::null(), PCWSTR(name_w.as_ptr()), 1, &mut buf) };
    if rc != NERR_Success {
        bail!("NetUserGetInfo 失败，错误码 {rc}");
    }
    unsafe {
        let info = &*(buf as *const USER_INFO_1);
        let comment = info.usri1_comment.to_string().unwrap_or_default();
        NetApiBufferFree(Some(buf as *const _));
        Ok(comment)
    }
}

/// 在 ProgramData 和公用文件夹里找出所有者是 sid 的条目。
/// 只看两层：顶层条目归它所有就整体删除，否则再看下一层，避免误删别人建的父目录。
fn find_owned(sid: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in SHARED_DIRS {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if owner_sid(&p).is_ok_and(|o| o == sid) {
                found.push(p);
                continue;
            }
            if !p.is_dir() {
                continue;
            }
            let Ok(children) = std::fs::read_dir(&p) else { continue };
            for c in children.flatten() {
                let cp = c.path();
                if owner_sid(&cp).is_ok_and(|o| o == sid) {
                    found.push(cp);
                }
            }
        }
    }
    found
}

fn run(prog: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(prog).args(args).status().with_context(|| format!("无法运行 {prog}"))?;
    if !status.success() {
        bail!("{prog} {} 失败: {status}", args.join(" "));
    }
    Ok(())
}

fn random_password() -> Result<String> {
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#%^*-_=+";
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(|e| anyhow::anyhow!("生成随机数失败: {e}"))?;
    Ok(buf.iter().map(|b| CHARS[*b as usize % CHARS.len()] as char).collect())
}

fn is_elevated() -> Result<bool> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let mut elev = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let r = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elev as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        r?;
        Ok(elev.TokenIsElevated != 0)
    }
}
