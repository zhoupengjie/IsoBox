# 生成 IsoBox 安装程序：target\installer\IsoBox-<版本>-setup.exe
# 本文件必须保存为带 BOM 的 UTF-8，否则 Windows PowerShell 5.1 会把中文读成乱码。
#
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1            编译并打包
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1 -NoBuild   用 target\release 里现成的程序
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1 -Preview   另外生成只看界面、不修改系统的预览版
#
# 代码签名（可选）：-SignCommand 或环境变量 ISOBOX_SIGN_COMMAND 给出签名命令，%1 代表要签名的文件，例如
#   -SignCommand 'signtool sign /fd sha256 /tr http://timestamp.digicert.com /td sha256 /a "%1"'
# 会依次签四个程序、卸载程序和安装程序本身。
#
# 需要：Rust、7-Zip（捆绑进安装包，默认取 C:\Program Files\7-Zip）。
# 自动准备的工具：NSIS 3.13 绿色版（下载到 target\tools，核对 SHA-256）、cargo-about（生成第三方许可证声明）。

param(
    [switch]$NoBuild,
    [switch]$Preview,
    [string]$SevenZipDir = 'C:\Program Files\7-Zip',
    [string]$SignCommand = $env:ISOBOX_SIGN_COMMAND
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$out = Join-Path $root 'target\installer'
$stage = Join-Path $out 'stage'

$NsisVersion = '3.13'
$NsisSha256 = 'ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8'

# 版本号写在三处，必须一致。改版本用 scripts\set-version.ps1
function Get-Version([string]$file, [string]$pattern) {
    $m = Select-String -Path $file -Pattern $pattern | Select-Object -First 1
    if (-not $m) { throw "$file 里找不到版本号" }
    return $m.Matches[0].Groups[1].Value
}
$version = Get-Version "$root\Cargo.toml" '^version\s*=\s*"(\d+\.\d+\.\d+)"'
$others = @{
    'gui\Cargo.toml'       = Get-Version "$root\gui\Cargo.toml" '^version\s*=\s*"([^"]+)"'
    'gui\tauri.conf.json'  = Get-Version "$root\gui\tauri.conf.json" '^\s*"version"\s*:\s*"([^"]+)"'
}
foreach ($k in $others.Keys) {
    if ($others[$k] -ne $version) { throw "版本号不一致：Cargo.toml 是 $version，$k 是 $($others[$k])。用 scripts\set-version.ps1 统一" }
}

if (-not $NoBuild) {
    cargo build --release --workspace --bins --manifest-path "$root\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败' }
}

# 固定用 NSIS 3.13，不用电脑上装的其他版本，保证每台机器生成的安装包一样。
# 确实要用别的 NSIS 时设置环境变量 NSIS_HOME
function Find-MakeNsis {
    if ($env:NSIS_HOME) {
        $p = Join-Path $env:NSIS_HOME 'makensis.exe'
        if (-not (Test-Path $p)) { throw "NSIS_HOME 里没有 makensis.exe：$p" }
        return $p
    }
    $pinned = Join-Path $root "target\tools\nsis-$NsisVersion\makensis.exe"
    if (Test-Path $pinned) { return $pinned }

    $tools = Join-Path $root 'target\tools'
    New-Item -ItemType Directory -Force $tools | Out-Null
    $zip = Join-Path $tools "nsis-$NsisVersion.zip"
    $url = "https://downloads.sourceforge.net/project/nsis/NSIS%203/$NsisVersion/nsis-$NsisVersion.zip"
    Write-Host "没有找到 NSIS，下载 $url"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $ProgressPreference = 'SilentlyContinue'   # PowerShell 5.1 的进度条会让下载慢很多
    Invoke-WebRequest -Uri $url -OutFile $zip -UserAgent 'Wget' -UseBasicParsing
    $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if ($hash -ne $NsisSha256) {
        Remove-Item $zip -Force
        throw "下载的 NSIS 校验不符：$hash，应为 $NsisSha256"
    }
    Expand-Archive $zip -DestinationPath $tools -Force
    return Join-Path $tools "nsis-$NsisVersion\makensis.exe"
}

# 准备要打包的文件
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\7-Zip" | Out-Null
foreach ($f in 'isobox.exe', 'isobox-setup.exe', 'isobox-launch.exe', 'isobox-gui.exe') {
    $p = Join-Path $root "target\release\$f"
    if (-not (Test-Path $p)) { throw "找不到 $p，先去掉 -NoBuild 编译一次" }
    Copy-Item $p $stage
}
Copy-Item "$root\isobox.toml" $stage
# 须知页要求 Windows 换行；带 BOM 的 UTF-8 让 makensis 按 Unicode 读取
$notice = [IO.File]::ReadAllText("$PSScriptRoot\notice.txt") -replace "`r?`n", "`r`n"
[IO.File]::WriteAllText("$stage\notice.txt", $notice, (New-Object Text.UTF8Encoding $true))

# 捆绑 7-Zip：精简版 7zr.exe 解不开 NSIS 安装包，所以要完整的 7z.exe 和 7z.dll，附上它的许可证（LGPL 等）
foreach ($f in '7z.exe', '7z.dll', 'License.txt') {
    $p = Join-Path $SevenZipDir $f
    if (-not (Test-Path $p)) { throw "找不到 $p。请安装 7-Zip（winget install 7zip.7zip），或用 -SevenZipDir 指定位置" }
    Copy-Item $p "$stage\7-Zip"
}

# 第三方许可证声明：分发编译好的程序时，MIT 等许可证要求附上它们的版权和许可证文本
if (-not (Get-Command cargo-about -ErrorAction SilentlyContinue)) {
    Write-Host '没有找到 cargo-about，安装 0.9 版'
    if (Get-Command cargo-binstall -ErrorAction SilentlyContinue) {
        cargo binstall --no-confirm --locked 'cargo-about@0.9'
    } else {
        # 不加 cli 功能只编译库：cargo install 照样报告成功，但不会装上 cargo-about.exe
        cargo install --locked cargo-about --version '^0.9' --features cli
    }
    if ($LASTEXITCODE -ne 0 -or -not (Get-Command cargo-about -ErrorAction SilentlyContinue)) { throw '安装 cargo-about 失败' }
}
New-Item -ItemType Directory -Force "$stage\licenses" | Out-Null
# IsoBox 自己的许可证（GPL-3.0），换成 Windows 换行，记事本里也能正常显示
$license = [IO.File]::ReadAllText("$root\LICENSE") -replace "`r?`n", "`r`n"
[IO.File]::WriteAllText("$stage\licenses\LICENSE.txt", $license, (New-Object Text.UTF8Encoding $false))
cargo about generate --workspace --manifest-path "$root\Cargo.toml" -c "$root\about.toml" "$PSScriptRoot\notices.hbs" -o "$stage\licenses\THIRD-PARTY-NOTICES.txt"
if ($LASTEXITCODE -ne 0) { throw 'cargo about 失败：可能有依赖用了 about.toml 里没有列出的许可证' }

# 欢迎页和完成页左侧的图，按 2 倍大小画，高分屏上也清楚
function New-Sidebar([string]$path) {
    Add-Type -AssemblyName System.Drawing
    $w = 328; $h = 628
    $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'; $g.InterpolationMode = 'HighQualityBicubic'; $g.TextRenderingHint = 'AntiAliasGridFit'
    $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
    $top = [System.Drawing.Color]::FromArgb(37, 99, 235); $bottom = [System.Drawing.Color]::FromArgb(23, 37, 84)
    $g.FillRectangle((New-Object System.Drawing.Drawing2D.LinearGradientBrush $rect, $top, $bottom, 90), $rect)
    $icon = [System.Drawing.Image]::FromFile("$root\gui\icons\icon.png")
    $g.DrawImage($icon, [int](($w - 128) / 2), 150, 128, 128)
    $icon.Dispose()
    $center = New-Object System.Drawing.StringFormat
    $center.Alignment = 'Center'
    $white = [System.Drawing.Brushes]::White
    $soft = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(200, 255, 255, 255))
    $g.DrawString('IsoBox', (New-Object System.Drawing.Font 'Segoe UI Semibold', 30), $white, (New-Object System.Drawing.RectangleF 0, 300, $w, 60), $center)
    $tagline = "让不信任的软件`n读不到你的文件"
    $g.DrawString($tagline, (New-Object System.Drawing.Font 'Microsoft YaHei UI', 15), $soft, (New-Object System.Drawing.RectangleF 0, 372, $w, 100), $center)
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $bmp.Dispose()
}
New-Sidebar "$stage\sidebar.bmp"

# 代码签名：先签要打包的程序；卸载程序和安装程序在 makensis 生成后由 sign.nsh 里的命令签
$signArgs = @()
if ($SignCommand) {
    if ($SignCommand -notmatch '%1') { throw '签名命令里要用 %1 代表要签名的文件' }
    foreach ($f in 'isobox.exe', 'isobox-setup.exe', 'isobox-launch.exe', 'isobox-gui.exe') {
        $cmdLine = $SignCommand.Replace('%1', "$stage\$f")
        Write-Host "签名 $f"
        cmd.exe /d /c $cmdLine
        if ($LASTEXITCODE -ne 0) { throw "签名 $f 失败，返回码 $LASTEXITCODE" }
    }
    $nsh = "!finalize ``$SignCommand`` = 0`r`n!uninstfinalize ``$SignCommand`` = 0`r`n"
    [IO.File]::WriteAllText("$stage\sign.nsh", $nsh, (New-Object Text.UTF8Encoding $true))
    $signArgs = @("/DSIGN_NSH=$stage\sign.nsh")
}

$makensis = Find-MakeNsis
Write-Host "使用 $makensis"

function Invoke-MakeNsis([string]$outFile, [string[]]$extra) {
    $nsisArgs = @('/V2', '/INPUTCHARSET', 'UTF8', "/DVERSION=$version", "/DSTAGE=$stage", "/DOUTFILE=$outFile",
                  "/DICON=$root\gui\icons\icon.ico") + $extra + @("$PSScriptRoot\isobox.nsi")
    & $makensis @nsisArgs
    if ($LASTEXITCODE -ne 0) { throw "makensis 失败，返回码 $LASTEXITCODE" }
    Write-Host "已生成 $outFile"
}

$setupExe = Join-Path $out "IsoBox-$version-setup.exe"
Invoke-MakeNsis $setupExe $signArgs
# 发布时附上校验值，下载的人可以核对
$hash = (Get-FileHash $setupExe -Algorithm SHA256).Hash.ToLower()
[IO.File]::WriteAllText("$setupExe.sha256", "$hash  $(Split-Path $setupExe -Leaf)`n")
Write-Host "SHA-256 $hash"
if ($Preview) {
    Invoke-MakeNsis (Join-Path $out "IsoBox-$version-preview.exe") @('/DPREVIEW')
}
