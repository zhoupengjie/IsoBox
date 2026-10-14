# 生成 IsoBox 安装程序：target\installer\IsoBox-<版本>-setup.exe
# 本文件必须保存为带 BOM 的 UTF-8，否则 Windows PowerShell 5.1 会把中文读成乱码。
#
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1            编译并打包
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1 -NoBuild   用 target\release 里现成的程序
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1 -Preview   另外生成只看界面、不修改系统的预览版
#
# 需要：Rust、7-Zip（捆绑进安装包，默认取 C:\Program Files\7-Zip）。
# NSIS 找不到时自动下载 3.13 的绿色版到 target\tools，并核对 SHA-256。

param(
    [switch]$NoBuild,
    [switch]$Preview,
    [string]$SevenZipDir = 'C:\Program Files\7-Zip'
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$out = Join-Path $root 'target\installer'
$stage = Join-Path $out 'stage'

$NsisVersion = '3.13'
$NsisSha256 = 'ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8'

# 版本号取自 Cargo.toml 的 [package]
$m = Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"(\d+\.\d+\.\d+)"' | Select-Object -First 1
if (-not $m) { throw 'Cargo.toml 里找不到形如 1.2.3 的版本号' }
$version = $m.Matches[0].Groups[1].Value

if (-not $NoBuild) {
    cargo build --release --workspace --bins --manifest-path "$root\Cargo.toml"
    if ($LASTEXITCODE -ne 0) { throw 'cargo build 失败' }
}

function Find-MakeNsis {
    $candidates = @()
    if ($env:NSIS_HOME) { $candidates += Join-Path $env:NSIS_HOME 'makensis.exe' }
    $cmd = Get-Command makensis.exe -ErrorAction SilentlyContinue
    if ($cmd) { $candidates += $cmd.Source }
    $candidates += "${env:ProgramFiles(x86)}\NSIS\makensis.exe", "$env:ProgramFiles\NSIS\makensis.exe"
    $candidates += Join-Path $root "target\tools\nsis-$NsisVersion\makensis.exe"
    foreach ($c in $candidates) { if ($c -and (Test-Path $c)) { return $c } }

    $tools = Join-Path $root 'target\tools'
    New-Item -ItemType Directory -Force $tools | Out-Null
    $zip = Join-Path $tools "nsis-$NsisVersion.zip"
    $url = "https://downloads.sourceforge.net/project/nsis/NSIS%203/$NsisVersion/nsis-$NsisVersion.zip"
    Write-Host "没有找到 NSIS，下载 $url"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
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

$makensis = Find-MakeNsis
Write-Host "使用 $makensis"

function Invoke-MakeNsis([string]$outFile, [string[]]$extra) {
    $nsisArgs = @('/V2', '/INPUTCHARSET', 'UTF8', "/DVERSION=$version", "/DSTAGE=$stage", "/DOUTFILE=$outFile",
                  "/DICON=$root\gui\icons\icon.ico") + $extra + @("$PSScriptRoot\isobox.nsi")
    & $makensis @nsisArgs
    if ($LASTEXITCODE -ne 0) { throw "makensis 失败，返回码 $LASTEXITCODE" }
    Write-Host "已生成 $outFile"
}

Invoke-MakeNsis (Join-Path $out "IsoBox-$version-setup.exe") @()
if ($Preview) {
    Invoke-MakeNsis (Join-Path $out "IsoBox-$version-preview.exe") @('/DPREVIEW')
}
