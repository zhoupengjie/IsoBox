# IsoBox：从隔离账户里移除一个软件。
# 从开始菜单的“IsoBox 移除软件”运行，会自动请求管理员权限。
# 本文件必须保存为带 BOM 的 UTF-8，否则 Windows PowerShell 5.1 会把中文读成乱码。

$ErrorActionPreference = 'Stop'
$setup = Join-Path $PSScriptRoot 'isobox-setup.exe'
$config = Join-Path $PSScriptRoot 'isobox.toml'

$me = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $me.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`""
    exit
}
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$Host.UI.RawUI.WindowTitle = 'IsoBox 移除软件'

try {
    # 从配置里读出软件列表和显示名
    $apps = @()
    $current = $null
    foreach ($line in [IO.File]::ReadAllLines($config, [Text.Encoding]::UTF8)) {
        if ($line -match '^\s*\[apps\.([a-z0-9_-]+)\]') {
            $current = [pscustomobject]@{ Name = $Matches[1]; Display = '' }
            $apps += $current
        } elseif ($line -match '^\s*\[') {
            $current = $null
        } elseif ($current -and $line -match '^\s*display\s*=\s*["''](.*)["'']') {
            $current.Display = $Matches[1]
        }
    }
    if (-not $apps) { Write-Host '配置里没有任何软件。'; Read-Host '按回车关闭窗口' | Out-Null; exit }

    Write-Host "当前的隔离软件：`n"
    for ($i = 0; $i -lt $apps.Count; $i++) {
        $d = if ($apps[$i].Display) { "  ($($apps[$i].Display))" } else { '' }
        Write-Host ("  {0,2}. {1}{2}" -f ($i + 1), $apps[$i].Name, $d)
    }
    Write-Host ''
    $n = Read-Host '输入要移除的序号，直接回车取消'
    if (-not $n) { exit }
    $app = $apps[[int]$n - 1]
    if (-not $app) { throw "没有序号 $n" }

    $ok = Read-Host "确定移除 $($app.Name)？程序文件、图标、快捷方式和配置都会删除。输入 y 确认"
    if ($ok -eq 'y') {
        Write-Host ''
        & $setup remove-app $app.Name
    }
} catch {
    Write-Host "出错了: $($_.Exception.Message)" -ForegroundColor Red
}
Write-Host ''
Read-Host '按回车关闭窗口' | Out-Null
