//! 日常使用的启动器，不需要管理员权限。

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use isobox::{
    appcontainer::{self, Container},
    config::{Config, Mode, default_capabilities},
    cred, shortcut, theme,
    start::{self, Exe, get_app, prepare_container, resolve_exe},
    unpack,
};

#[derive(Parser)]
#[command(name = "isobox", about = "在隔离环境里启动软件")]
struct Cli {
    /// 配置文件路径，默认找 exe 旁边的 isobox.toml
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 启动配置里的某个软件，例如 isobox run wechat
    Run {
        app: String,
        /// 临时覆盖配置里的隔离方式
        #[arg(long, value_enum)]
        mode: Option<Mode>,
        /// 等待程序退出并显示退出码，用于排查启动失败
        #[arg(long)]
        wait: bool,
        /// 把程序的标准输出和标准错误写进这个文件，用于排查
        #[arg(long)]
        stdio: Option<PathBuf>,
        /// 启动前不同步深浅色模式，也不拉起后台监视进程，用于排查
        #[arg(long)]
        no_theme_sync: bool,
        /// 追加到配置 args 后面的参数，写在 -- 之后，例如 isobox run qq -- --no-sandbox
        #[arg(last = true)]
        extra: Vec<String>,
    },
    /// 把安装包拆成绿色版放进软件的容器里，不运行安装程序、不写注册表
    Unpack {
        app: String,
        /// 官方安装包路径
        installer: PathBuf,
        /// 覆盖已有的程序文件，用于升级。聊天记录等数据不受影响
        #[arg(long)]
        force: bool,
    },
    /// 在桌面和开始菜单创建快捷方式，例如 isobox shortcut wechat --name 微信
    Shortcut {
        app: String,
        /// 快捷方式显示的名字，默认用配置里的 display，没有就用英文名
        #[arg(long)]
        name: Option<String>,
        /// 同时加入开机启动
        #[arg(long)]
        startup: bool,
        /// 删除这个名字的快捷方式
        #[arg(long)]
        remove: bool,
    },
    /// 立即把你的深浅色模式和强调色同步给沙盒用户，并通知正在运行的软件刷新
    ThemeSync,
    /// 查看安装包类型，决定能否预填安装目录
    Inspect { installer: PathBuf },
    /// 安装前预览：类型、能否拆包、主程序候选。解到临时目录，用完删除
    Preview {
        installer: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// 汇总当前状态：账户、库、受保护的位置、软件是否在运行
    Status {
        #[arg(long)]
        json: bool,
        /// 同时提取软件图标（JSON 里的 data URL）
        #[arg(long)]
        icons: bool,
    },
    /// 在 AppContainer 里临时运行任意程序，用于测试兼容性
    Exec {
        /// 容器名后缀，实际容器名为 isobox.<name>
        #[arg(long, default_value = "try")]
        name: String,
        exe: PathBuf,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// 列出配置里的软件、隔离方式，并检查程序是否存在
    List,
    /// 显示某个软件的 AppContainer 信息：SID、数据文件夹、能力
    Info { app: String },
    /// 删除某个软件的 AppContainer：撤销授权、删除配置和全部数据，绿色版程序也一并删除
    Reset { app: String },
    /// 手动把沙盒用户的密码存进凭据管理器，仅 user 模式需要
    StorePassword,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg_path = cli.config.clone().unwrap_or_else(Config::default_path);
    let cfg = Config::load(&cfg_path)?;

    match cli.cmd {
        Cmd::List => {
            for (name, app) in &cfg.apps {
                let mode = match (app.mode(&cfg), app.library_drive()) {
                    (Mode::User, Some(d)) => format!("{d} 盘库      "),
                    (Mode::User, None) if app.portable => "旧位置      ".to_string(),
                    (Mode::User, None) => "原版安装    ".to_string(),
                    (Mode::AppContainer, _) => "appcontainer".to_string(),
                };
                let (mark, path) = match resolve_exe(&cfg, name, app, app.mode(&cfg)) {
                    Ok(Exe::Found(p)) => ("OK    ", p.display().to_string()),
                    Ok(Exe::Missing(p)) => ("未找到", p.display().to_string()),
                    Ok(Exe::Unchecked(p)) => ("无法查", format!("{}（在 {} 的个人文件夹里，你的账户无权查看）", p.display(), cfg.sandbox_user)),
                    Ok(Exe::NotReady(msg)) => ("未就绪", msg),
                    Err(e) => ("配置错", e.to_string()),
                };
                println!("{name:14} {mode} {mark} {path}");
            }
        }
        Cmd::StorePassword => {
            let pw = rpassword::prompt_password(format!("{} 的密码: ", cfg.sandbox_user))?;
            cred::store(&cfg.sandbox_user, &pw)?;
            println!("已保存到凭据管理器。");
        }
        Cmd::Run { app, mode, wait, stdio, no_theme_sync, extra } => {
            let (child, msg) =
                start::start(&cfg, &app, mode, &join_args(&extra), stdio.as_deref(), !no_theme_sync)?;
            println!("{msg}");
            if wait {
                let code = child.wait()?;
                println!("{app} 已退出，退出码 {code} (0x{code:08X})");
            }
        }
        Cmd::Preview { installer, json } => {
            let seven = cfg.sevenzip.clone().unwrap_or_else(|| unpack::DEFAULT_7Z.into());
            let p = unpack::preview(&seven, &installer);
            if json {
                println!("{}", serde_json::to_string_pretty(&p)?);
            } else {
                println!("类型 {}，建议英文名 {}，能否快速安装 {}", p.kind, p.guessed_name, p.unpackable);
                for c in &p.candidates {
                    println!("  {}  {:.1} MB{}", c.rel_path, c.size as f64 / 1048576.0, if c.likely_helper { "  （像辅助程序）" } else { "" });
                }
                if let Some(e) = &p.error {
                    println!("{e}");
                }
            }
        }
        Cmd::Status { json, icons } => {
            let st = isobox::status::overview(&cfg_path, &cfg, icons);
            if json {
                println!("{}", serde_json::to_string_pretty(&st)?);
            } else {
                println!("隔离账户 {}：{}", st.account.user, if st.account.exists { "正常" } else { "不存在" });
                for l in &st.libraries {
                    println!("库 {} 盘：{}", l.drive, l.apps.join("、"));
                }
                for p in &st.protected {
                    println!("保护 {}：{}", p.path, if p.active { "已生效" } else { "未生效" });
                }
                for a in &st.apps {
                    println!("软件 {}（{}）：{}{}", a.display, a.name, if a.running { "运行中" } else { "未运行" }, if a.startup { "，开机启动" } else { "" });
                }
            }
        }
        Cmd::Inspect { installer } => {
            let seven = cfg.sevenzip.clone().unwrap_or_else(|| unpack::DEFAULT_7Z.into());
            let kind = unpack::installer_kind(&seven, &installer);
            println!("{}: {kind:?}", installer.display());
        }
        Cmd::ThemeSync => {
            let pw = cred::load(&cfg.sandbox_user)?;
            theme::sync(&cfg.sandbox_user, &pw)?;
            println!("已同步主题到 {}，并通知正在运行的软件刷新", cfg.sandbox_user);
            println!("后台监视进程运行中: {}", theme::watcher_running());
        }
        Cmd::Shortcut { app, name, startup, remove } => {
            let a = get_app(&cfg, &app)?;
            let name = name.or_else(|| a.display.clone()).unwrap_or_else(|| app.clone());
            if remove {
                for p in shortcut::remove(&name)? {
                    println!("已删除 {}", p.display());
                }
            } else {
                let exe = match resolve_exe(&cfg, &app, a, a.mode(&cfg))? {
                    Exe::Found(p) => Some(p),
                    _ => None,
                };
                for p in shortcut::create(&app, &name, startup, exe.as_deref())? {
                    println!("已创建 {}", p.display());
                }
            }
        }
        Cmd::Unpack { app, installer, force } => {
            let a = get_app(&cfg, &app)?;
            if !a.portable {
                bail!("{app} 在配置里不是绿色版，请先加上 portable = true，并把 exe 改成只写文件名");
            }
            if a.mode(&cfg) == Mode::User {
                bail!(
                    "{app} 使用账户模式，程序要放进库里。\n\
                     请在管理员终端里运行: isobox-setup add {app} \"{}\"",
                    installer.display()
                );
            }
            let c = prepare_container(&cfg, &appcontainer::container_name(&app), &a.extra_read)?;
            let seven = cfg.sevenzip.clone().unwrap_or_else(|| unpack::DEFAULT_7Z.into());
            let exe = unpack::unpack(&seven, &installer, &c.folder()?.join("app"), Some(&a.exe_name()?), force, &|_| Ok(0))?;
            println!("完成，主程序: {}", exe.display());
            println!("现在可以运行 isobox run {app}");
        }
        Cmd::Exec { name, exe, args } => {
            let workdir = exe.parent().map(PathBuf::from).unwrap_or_else(|| ".".into());
            let c = prepare_container(&cfg, &appcontainer::container_name(&name), &[])?;
            let child = c.launch(&default_capabilities(), &exe, &join_args(&args), &workdir, None)?;
            println!("已在容器 {} 中启动 {}，PID {}", c.name, exe.display(), child.pid);
        }
        Cmd::Info { app } => {
            let a = get_app(&cfg, &app)?;
            let name = appcontainer::container_name(&app);
            match Container::open_existing(&name)? {
                Some(c) => {
                    println!("容器名      {}", c.name);
                    println!("SID         {}", c.sid_string);
                    println!("数据文件夹  {}", c.folder()?.display());
                    if a.portable {
                        println!("程序目录    {}", c.folder()?.join("app").display());
                    }
                }
                None => println!("容器 {name} 还没有创建，第一次 run 或 unpack 时会自动创建"),
            }
            println!("能力:");
            for (cap, sids) in appcontainer::capability_sids(&a.capabilities)? {
                println!("  {cap:28} {}", sids.join(", "));
            }
        }
        Cmd::Reset { app } => {
            let name = appcontainer::container_name(&app);
            match Container::open_existing(&name)? {
                Some(c) => {
                    c.delete()?;
                    println!("已删除容器 {name}，包括其中的程序、全部数据和授权。");
                }
                None => println!("容器 {name} 不存在，无需清理。"),
            }
        }
    }
    Ok(())
}

fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}
