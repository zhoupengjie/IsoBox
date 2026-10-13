# 编译 IsoBox 并安装到固定位置：系统盘的 Program Files\IsoBox。
# 需要在管理员终端运行。
# 本文件必须保存为带 BOM 的 UTF-8，否则 Windows PowerShell 5.1 会把中文读成乱码。
#
#   .\deploy.ps1                      编译并安装
#   .\deploy.ps1 -NoBuild -BinDir X   不编译，直接用 X 里现成的程序（自动测试时使用）

param(
    [switch]$NoBuild,
    [string]$BinDir
)
$ErrorActionPreference = 'Stop'
$src = $PSScriptRoot
$pf = if ($env:ProgramW6432) { $env:ProgramW6432 } else { $env:ProgramFiles }
$Target = Join-Path $pf 'IsoBox'
if (-not $BinDir) { $BinDir = Join-Path $src 'target\release' }

$me = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $me.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw '需要管理员权限，请在“以管理员身份运行”的终端里执行'
}

if (-not $NoBuild) {
    cargo build --release --workspace --manifest-path "$src\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败' }
}

# 安全检查：安装目录已存在时，里面只能有 IsoBox 自己的东西，避免锁定权限时影响别的文件
$ours = @('isobox.exe', 'isobox-setup.exe', 'isobox-launch.exe', 'isobox-gui.exe', 'add-app.ps1', 'remove-app.ps1',
          'isobox.toml', 'isobox.toml.template', 'library.toml', 'apps', 'backup', 'icons')
if (Test-Path $Target) {
    $foreign = Get-ChildItem $Target -Force | Where-Object { $_.Name -notin $ours }
    if ($foreign) {
        throw "$Target 里有不属于 IsoBox 的文件：$(($foreign.Name) -join '、')。为避免改动它们的权限，停止安装"
    }
}

# 后台的主题监视进程和 IsoBox 界面会占用程序文件，先停掉，下次启动软件或打开界面时会重新运行
Get-Process isobox-launch, isobox-gui -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

New-Item -ItemType Directory -Force $Target | Out-Null
foreach ($f in 'isobox.exe', 'isobox-setup.exe', 'isobox-launch.exe', 'isobox-gui.exe') {
    Copy-Item (Join-Path $BinDir $f) $Target -Force
}
foreach ($f in 'add-app.ps1', 'remove-app.ps1') {
    Copy-Item "$src\tools\$f" $Target -Force
}

# 安装目录里的配置才是实际生效的，更新时绝不覆盖
if (Test-Path "$Target\isobox.toml") {
    Write-Host "保留已有的配置 $Target\isobox.toml"
} else {
    Copy-Item "$src\isobox.toml" $Target
}

# isobox-setup.exe 以管理员身份运行。如果普通用户能替换它，任何程序都可能借此拿到管理员权限。
# 所以和系统的 Program Files 一样：管理员和 SYSTEM 完全控制，普通用户只能读取和运行。
# 库的 apps 目录关闭了继承，有自己的权限，不受这里影响
icacls $Target /inheritance:r /grant:r '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX' /q | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'icacls 失败' }

# 开始菜单 IsoBox 分组里的管理入口
$group = Join-Path ([Environment]::GetFolderPath('Programs')) 'IsoBox'
New-Item -ItemType Directory -Force $group | Out-Null
$ws = New-Object -ComObject WScript.Shell
$main = $ws.CreateShortcut((Join-Path $group 'IsoBox.lnk'))
$main.TargetPath = "$Target\isobox-gui.exe"
$main.WorkingDirectory = $Target
$main.Description = 'IsoBox：管理隔离软件'
$main.Save()
foreach ($item in @(@('IsoBox 添加软件', 'add-app.ps1'), @('IsoBox 移除软件', 'remove-app.ps1'))) {
    $l = $ws.CreateShortcut((Join-Path $group "$($item[0]).lnk"))
    $l.TargetPath = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
    $l.Arguments = "-NoProfile -ExecutionPolicy Bypass -File `"$Target\$($item[1])`""
    $l.WorkingDirectory = $Target
    $l.Description = $item[0]
    $l.Save()
}

Write-Host "已安装到 $Target"
Get-ChildItem $Target -File | Format-Table Name, Length, LastWriteTime -AutoSize | Out-String | Write-Host
