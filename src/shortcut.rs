//! 在当前用户的桌面、开始菜单和启动文件夹里创建或删除快捷方式。
//! 快捷方式指向无控制台窗口的 isobox-launch.exe。
//! 图标：库里的程序你的账户可以读，直接用主程序；旧位置（隔离账户个人文件夹）读不到，用复制到 icons 的副本。

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

pub const LAUNCHER_EXE: &str = "isobox-launch.exe";

/// 开始菜单里的分组文件夹名
const START_MENU_GROUP: &str = "IsoBox";

/// 和 isobox.exe、isobox-launch.exe 同一个目录，也就是安装目录
pub fn install_dir() -> Result<PathBuf> {
    crate::paths::install_dir()
}

/// 旧位置的软件用的图标副本：复制到安装目录 icons 下的主程序
pub fn icon_path(app: &str) -> Result<PathBuf> {
    Ok(install_dir()?.join("icons").join(format!("{app}.exe")))
}

/// 给软件挑一个图标：优先用传入的主程序（你的账户能读时），其次是 icons 下的副本，最后用启动器
fn choose_icon(app: &str, exe: Option<&Path>, launcher: &Path) -> PathBuf {
    if let Some(e) = exe.filter(|e| std::fs::metadata(e).is_ok()) {
        return e.to_path_buf();
    }
    match icon_path(app) {
        Ok(p) if p.exists() => p,
        _ => launcher.to_path_buf(),
    }
}

/// 创建快捷方式。exe 是主程序路径，你的账户能读到时直接用它的图标
pub fn create(app: &str, name: &str, startup: bool, exe: Option<&Path>) -> Result<Vec<PathBuf>> {
    create_with(app, name, if startup { "all+startup" } else { "all" }, exe)
}

/// 只在开机启动文件夹里创建或删除某个软件的快捷方式
pub fn set_app_startup(app: &str, name: &str, enable: bool, exe: Option<&Path>) -> Result<()> {
    if enable {
        if !startup_apps().iter().any(|a| a == app) {
            create_with(app, name, "startup", exe)?;
        }
    } else {
        for (lnk, _, args) in startup_links() {
            if args == app {
                std::fs::remove_file(&lnk).with_context(|| format!("删除 {} 失败", lnk.display()))?;
            }
        }
    }
    Ok(())
}

/// where: "all" 桌面和开始菜单；"all+startup" 再加开机启动；"startup" 只放开机启动
fn create_with(app: &str, name: &str, where_: &str, exe: Option<&Path>) -> Result<Vec<PathBuf>> {
    let dir = install_dir()?;
    let target = dir.join(LAUNCHER_EXE);
    if !target.exists() {
        bail!("找不到 {}，请先用 deploy.ps1 部署最新版本", target.display());
    }
    let icon = choose_icon(app, exe, &target);
    let startup = where_.contains("startup");
    let script = r#"
        [Console]::OutputEncoding = [Text.Encoding]::UTF8
        $ws = New-Object -ComObject WScript.Shell
        $dirs = @()
        if ($env:ISOBOX_WHERE -ne 'startup') {
            $dirs += [Environment]::GetFolderPath('Desktop')
            $dirs += (Join-Path ([Environment]::GetFolderPath('Programs')) $env:ISOBOX_GROUP)
        }
        if ($env:ISOBOX_STARTUP -eq '1') { $dirs += [Environment]::GetFolderPath('Startup') }
        foreach ($d in $dirs) {
            New-Item -ItemType Directory -Force $d | Out-Null
            $p = Join-Path $d ($env:ISOBOX_NAME + '.lnk')
            $l = $ws.CreateShortcut($p)
            $l.TargetPath = $env:ISOBOX_TARGET
            $l.Arguments = $env:ISOBOX_APP
            $l.WorkingDirectory = $env:ISOBOX_DIR
            $l.IconLocation = $env:ISOBOX_ICON + ',0'
            $l.Description = $env:ISOBOX_DESC
            $l.Save()
            $p
        }
    "#;
    run_ps(
        script,
        &[
            ("ISOBOX_APP", app.to_string()),
            ("ISOBOX_NAME", name.to_string()),
            ("ISOBOX_TARGET", target.display().to_string()),
            ("ISOBOX_DIR", dir.display().to_string()),
            ("ISOBOX_ICON", icon.display().to_string()),
            ("ISOBOX_GROUP", START_MENU_GROUP.to_string()),
            ("ISOBOX_STARTUP", if startup { "1" } else { "0" }.to_string()),
            ("ISOBOX_WHERE", where_.to_string()),
            ("ISOBOX_DESC", format!("以独立账户隔离运行 {name}")),
        ],
    )
}

/// 读取 .lnk 快捷方式的目标和参数。直接用 Windows Shell 的 COM 接口，比启动 PowerShell 快得多
pub fn read_lnk(path: &Path) -> Option<(String, String)> {
    use windows::{
        Win32::{
            System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile, STGM_READ},
            UI::Shell::{IShellLinkW, SLGP_RAWPATH, ShellLink},
        },
        core::{HSTRING, Interface},
    };
    let wide = |b: &[u16]| {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf16_lossy(&b[..end])
    };
    unsafe {
        // 已经初始化过（包括不同模式）时返回错误码，但 COM 仍可用，忽略即可
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        pf.Load(&HSTRING::from(path.as_os_str()), STGM_READ).ok()?;
        let mut target = [0u16; 1024];
        link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32).ok()?;
        let mut args = [0u16; 1024];
        link.GetArguments(&mut args).ok()?;
        Some((wide(&target), wide(&args)))
    }
}

/// 开机启动文件夹里的所有快捷方式：(文件, 目标, 参数)
fn startup_links() -> Vec<(PathBuf, String, String)> {
    let Ok(dir) = crate::paths::startup_dir() else { return Vec::new() };
    std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
                .filter_map(|p| read_lnk(&p).map(|(t, a)| (p, t, a)))
                .collect()
        })
        .unwrap_or_default()
}

/// 开始菜单 IsoBox 分组和桌面上，启动各个软件的快捷方式名字。配置里没写显示名时用它
pub fn link_names() -> std::collections::HashMap<String, String> {
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Programs, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    let folder = |id| unsafe {
        SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok().and_then(|p| {
            let s = p.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
            s.map(PathBuf::from)
        })
    };
    let mut dirs = Vec::new();
    if let Some(p) = folder(&FOLDERID_Programs) {
        dirs.push(p.join(START_MENU_GROUP));
    }
    if let Some(d) = folder(&FOLDERID_Desktop) {
        dirs.push(d);
    }
    let mut map = std::collections::HashMap::new();
    for d in dirs {
        let Ok(it) = std::fs::read_dir(&d) else { continue };
        for e in it.flatten() {
            let p = e.path();
            if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")) {
                continue;
            }
            if let Some((t, a)) = read_lnk(&p)
                && t.to_lowercase().ends_with(LAUNCHER_EXE) && !a.is_empty() {
                    let name = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    map.entry(a).or_insert(name);
                }
        }
    }
    map
}

/// 设置了开机启动的隔离软件名
pub fn startup_apps() -> Vec<String> {
    startup_links()
        .into_iter()
        .filter(|(_, t, _)| t.to_lowercase().ends_with(LAUNCHER_EXE))
        .map(|(_, _, a)| a)
        .collect()
}

pub const GUI_EXE: &str = "isobox-gui.exe";

/// IsoBox 托盘是否开机启动
pub fn tray_autostart_enabled() -> bool {
    startup_links().iter().any(|(_, t, _)| t.to_lowercase().ends_with(GUI_EXE))
}

/// 开启或关闭 IsoBox 托盘的开机启动
pub fn set_tray_autostart(enable: bool) -> Result<()> {
    let startup = crate::paths::startup_dir()?;
    for (lnk, t, _) in startup_links() {
        if t.to_lowercase().ends_with(GUI_EXE) {
            std::fs::remove_file(&lnk)?;
        }
    }
    if enable {
        let gui = install_dir()?.join(GUI_EXE);
        let script = r#"
            $l = (New-Object -ComObject WScript.Shell).CreateShortcut($env:ISOBOX_LNK)
            $l.TargetPath = $env:ISOBOX_TARGET
            $l.Arguments = '--tray'
            $l.WorkingDirectory = $env:ISOBOX_DIR
            $l.Description = 'IsoBox 托盘'
            $l.Save()
        "#;
        run_ps(
            script,
            &[
                ("ISOBOX_LNK", startup.join("IsoBox.lnk").display().to_string()),
                ("ISOBOX_TARGET", gui.display().to_string()),
                ("ISOBOX_DIR", install_dir()?.display().to_string()),
            ],
        )?;
    }
    Ok(())
}

/// 删除某个名字的快捷方式
pub fn remove(name: &str) -> Result<Vec<PathBuf>> {
    let script = r#"
        [Console]::OutputEncoding = [Text.Encoding]::UTF8
        $group = Join-Path ([Environment]::GetFolderPath('Programs')) $env:ISOBOX_GROUP
        foreach ($d in @([Environment]::GetFolderPath('Desktop'), $group, [Environment]::GetFolderPath('Startup'))) {
            $p = Join-Path $d ($env:ISOBOX_NAME + '.lnk')
            if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force; $p }
        }
        if ((Test-Path $group) -and -not (Get-ChildItem $group)) { Remove-Item $group -Force }
    "#;
    run_ps(
        script,
        &[
            ("ISOBOX_NAME", name.to_string()),
            ("ISOBOX_GROUP", START_MENU_GROUP.to_string()),
        ],
    )
}

/// 删除指向 isobox-launch.exe 的快捷方式。
/// app 为 Some 时只删启动该软件的，为 None 时全部删除，并删掉开始菜单里的整个 IsoBox 分组（卸载时使用）。
fn remove_matching(app: Option<&str>) -> Result<Vec<PathBuf>> {
    let script = r#"
        [Console]::OutputEncoding = [Text.Encoding]::UTF8
        $ws = New-Object -ComObject WScript.Shell
        $group = Join-Path ([Environment]::GetFolderPath('Programs')) $env:ISOBOX_GROUP
        foreach ($d in @([Environment]::GetFolderPath('Desktop'), $group, [Environment]::GetFolderPath('Startup'))) {
            if (-not (Test-Path $d)) { continue }
            foreach ($f in Get-ChildItem $d -Filter *.lnk) {
                $l = $ws.CreateShortcut($f.FullName)
                $ours = $l.TargetPath -like ('*\' + $env:ISOBOX_EXE)
                $match = $ours -and ($env:ISOBOX_APP -eq '*' -or $l.Arguments -eq $env:ISOBOX_APP)
                # 卸载时，开始菜单 IsoBox 分组里的其他快捷方式（添加、移除软件）也一起删
                if ($match -or ($env:ISOBOX_APP -eq '*' -and $d -eq $group)) {
                    Remove-Item -LiteralPath $f.FullName -Force
                    $f.FullName
                }
            }
        }
        if ((Test-Path $group) -and -not (Get-ChildItem $group)) { Remove-Item $group -Force }
    "#;
    run_ps(
        script,
        &[
            ("ISOBOX_EXE", LAUNCHER_EXE.to_string()),
            ("ISOBOX_GROUP", START_MENU_GROUP.to_string()),
            ("ISOBOX_APP", app.unwrap_or("*").to_string()),
        ],
    )
}

/// 把启动某个软件的所有快捷方式改为指向当前安装目录的启动器，并更新图标。
/// 用于移动软件到其他库、或 IsoBox 换了安装位置之后。返回更新过的快捷方式
pub fn update_for_app(app: &str, exe: Option<&Path>) -> Result<Vec<PathBuf>> {
    let dir = install_dir()?;
    let target = dir.join(LAUNCHER_EXE);
    let icon = choose_icon(app, exe, &target);
    let script = r#"
        [Console]::OutputEncoding = [Text.Encoding]::UTF8
        $ws = New-Object -ComObject WScript.Shell
        $group = Join-Path ([Environment]::GetFolderPath('Programs')) $env:ISOBOX_GROUP
        foreach ($d in @([Environment]::GetFolderPath('Desktop'), $group, [Environment]::GetFolderPath('Startup'))) {
            if (-not (Test-Path $d)) { continue }
            foreach ($f in Get-ChildItem $d -Filter *.lnk) {
                $l = $ws.CreateShortcut($f.FullName)
                if ($l.TargetPath -like ('*\' + $env:ISOBOX_EXE) -and $l.Arguments -eq $env:ISOBOX_APP) {
                    $l.TargetPath = $env:ISOBOX_TARGET
                    $l.WorkingDirectory = $env:ISOBOX_DIR
                    $l.IconLocation = $env:ISOBOX_ICON + ',0'
                    $l.Save()
                    $f.FullName
                }
            }
        }
    "#;
    run_ps(
        script,
        &[
            ("ISOBOX_EXE", LAUNCHER_EXE.to_string()),
            ("ISOBOX_GROUP", START_MENU_GROUP.to_string()),
            ("ISOBOX_APP", app.to_string()),
            ("ISOBOX_TARGET", target.display().to_string()),
            ("ISOBOX_DIR", dir.display().to_string()),
            ("ISOBOX_ICON", icon.display().to_string()),
        ],
    )
}

/// 删除启动某个软件的所有快捷方式，不管它叫什么名字
pub fn remove_for_app(app: &str) -> Result<Vec<PathBuf>> {
    remove_matching(Some(app))
}

/// 删除所有 IsoBox 的快捷方式，卸载时使用
pub fn remove_all() -> Result<Vec<PathBuf>> {
    remove_matching(None)
}

fn run_ps(script: &str, env: &[(&str, String)]) -> Result<Vec<PathBuf>> {
    let mut cmd = Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().context("无法运行 powershell")?;
    if !out.status.success() {
        bail!("PowerShell 执行失败: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect())
}
