# 修改 IsoBox 的版本号：Cargo.toml、gui\Cargo.toml、gui\tauri.conf.json 三处一起改，再更新 Cargo.lock。
# 本文件必须保存为带 BOM 的 UTF-8，否则 Windows PowerShell 5.1 会把中文读成乱码。
#
#   powershell -ExecutionPolicy Bypass -File scripts\set-version.ps1 0.2.0
#
# 改完记得在 CHANGELOG.md 里写上这个版本的变化。发布：提交后打标签 v0.2.0 并推送，CI 会生成安装包和发布草稿。

param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "版本号要写成 1.2.3 的形式：$Version" }
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$utf8 = New-Object Text.UTF8Encoding $false

function Set-First([string]$file, [string]$pattern, [string]$replacement) {
    $text = [IO.File]::ReadAllText($file)
    $re = New-Object Text.RegularExpressions.Regex $pattern, 'Multiline'
    if (-not $re.IsMatch($text)) { throw "$file 里找不到版本号" }
    [IO.File]::WriteAllText($file, $re.Replace($text, $replacement, 1), $utf8)
    Write-Host "已修改 $file"
}

Set-First "$root\Cargo.toml" '^version\s*=\s*"[^"]*"' "version = `"$Version`""
Set-First "$root\gui\Cargo.toml" '^version\s*=\s*"[^"]*"' "version = `"$Version`""
Set-First "$root\gui\tauri.conf.json" '^(\s*)"version"\s*:\s*"[^"]*"' "`$1`"version`": `"$Version`""

cargo update --workspace --manifest-path "$root\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw '更新 Cargo.lock 失败' }
Write-Host "版本号已改为 $Version"
