//! 找到软件的主程序并按配置的隔离方式启动。命令行启动器和无窗口启动器共用。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{
    account,
    appcontainer::{self, Container},
    child::Child,
    config::{App, Config, Mode},
    cred, launch, library, shortcut, theme,
};

pub enum Exe {
    Found(PathBuf),
    Missing(PathBuf),
    /// 在沙盒用户的个人文件夹里，当前账户无权查看，只能直接尝试启动
    Unchecked(PathBuf),
    /// 还没安装或拆包，附带下一步该做什么
    NotReady(String),
}

pub fn get_app<'a>(cfg: &'a Config, app: &str) -> Result<&'a App> {
    match cfg.apps.get(app) {
        Some(a) => Ok(a),
        None => bail!(
            "配置里没有 {app}，可用的有: {:?}",
            cfg.apps.keys().collect::<Vec<_>>()
        ),
    }
}

/// 主程序的完整路径。
/// - 普通安装：配置里写的路径
/// - 绿色版 + appcontainer：容器文件夹的 app 目录
/// - 绿色版 + user + 指定了库：<盘>:\Program Files\IsoBox\apps\<app>
/// - 绿色版 + user + 没指定库：旧位置，沙盒用户个人文件夹的 Apps\<app>
pub fn resolve_exe(cfg: &Config, app_name: &str, a: &App, mode: Mode) -> Result<Exe> {
    if !a.portable {
        return Ok(check(a.exe.clone()));
    }
    let exe_name = a.exe_name()?;
    match mode {
        Mode::AppContainer => match Container::open_existing(&appcontainer::container_name(app_name))? {
            Some(c) => Ok(match check(c.folder()?.join("app").join(&exe_name)) {
                Exe::Missing(_) => Exe::NotReady(format!("还没有拆包，先运行 isobox unpack {app_name} <安装包>")),
                other => other,
            }),
            None => Ok(Exe::NotReady(format!("还没有拆包，先运行 isobox unpack {app_name} <安装包>"))),
        },
        Mode::User => {
            if let Some(d) = a.library_drive() {
                return Ok(match check(library::app_dir(d, app_name).join(&exe_name)) {
                    Exe::Missing(p) => Exe::NotReady(format!(
                        "{} 不存在。{d} 盘可能没有接上，或者 {app_name} 还没有装进 {d} 盘的库",
                        p.display()
                    )),
                    other => other,
                });
            }
            let user = &cfg.sandbox_user;
            let Ok(sid) = account::lookup_sid(user) else {
                return Ok(Exe::NotReady(format!(
                    "账户 {user} 不存在，先在管理员终端运行 isobox-setup install --apply"
                )));
            };
            let Some(profile) = account::profile_path(&sid)? else {
                return Ok(Exe::NotReady(format!(
                    "账户 {user} 还没有个人文件夹，先在管理员终端运行 isobox-setup install --apply"
                )));
            };
            Ok(check(profile.join("Apps").join(app_name).join(exe_name)))
        }
    }
}

fn check(p: PathBuf) -> Exe {
    match std::fs::metadata(&p) {
        Ok(_) => Exe::Found(p),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Exe::Unchecked(p),
        Err(_) => Exe::Missing(p),
    }
}

/// 打开或创建容器，并授权交换目录和额外的只读目录。
pub fn prepare_container(cfg: &Config, name: &str, extra_read: &[PathBuf]) -> Result<Container> {
    let c = Container::ensure(name)?;
    let exchange = cfg.exchange_dir()?;
    std::fs::create_dir_all(&exchange)?;
    c.grant(&exchange, "M")?;
    for p in extra_read {
        c.grant(p, "RX")?;
    }
    Ok(c)
}

/// 启动软件，返回子进程和一句给用户看的说明。
pub fn start(
    cfg: &Config,
    app: &str,
    mode: Option<Mode>,
    extra_args: &str,
    stdio: Option<&Path>,
    sync_theme: bool,
) -> Result<(Child, String)> {
    let a = get_app(cfg, app)?;
    let args = [a.args.as_str(), extra_args].join(" ").trim().to_string();
    let mode = mode.unwrap_or(a.mode(cfg));
    let exe = match resolve_exe(cfg, app, a, mode)? {
        Exe::Found(p) | Exe::Unchecked(p) => p,
        Exe::Missing(p) => bail!("找不到 {}", p.display()),
        Exe::NotReady(msg) => bail!(msg),
    };
    let workdir = a.workdir_for(&exe)?;
    match mode {
        Mode::User => {
            let pw = cred::load(&cfg.sandbox_user)?;
            // 先把深浅色模式和强调色同步过去，再启动软件，并确保后台有监视进程跟随后续切换。
            // 同步失败不影响启动
            if sync_theme {
                let _ = theme::sync(&cfg.sandbox_user, &pw);
                if let Ok(dir) = shortcut::install_dir() {
                    theme::ensure_watcher(&dir.join(shortcut::LAUNCHER_EXE));
                }
            }
            let child = launch::launch_as(&cfg.sandbox_user, &pw, &exe, &args, &workdir)
                .context("启动失败。如果提示找不到文件，可能还没有拆包")?;
            let msg = format!("已以 {} 身份启动 {app}，PID {}", cfg.sandbox_user, child.pid);
            Ok((child, msg))
        }
        Mode::AppContainer => {
            let c = prepare_container(cfg, &appcontainer::container_name(app), &a.extra_read)?;
            let child = c.launch(&a.capabilities, &exe, &args, &workdir, stdio)?;
            let msg = format!("已在容器 {} 中启动 {app}，PID {}", c.name, child.pid);
            Ok((child, msg))
        }
    }
}
