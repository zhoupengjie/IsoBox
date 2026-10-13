# IsoBox：一键把软件添加到隔离账户。
# 从开始菜单的“IsoBox 添加软件”运行，会自动请求管理员权限。
# 本文件必须保存为带 BOM 的 UTF-8，否则 Windows PowerShell 5.1 会把中文读成乱码。

$ErrorActionPreference = 'Stop'
$setup = Join-Path $PSScriptRoot 'isobox-setup.exe'

# 不是管理员就以管理员身份重新运行自己
$me = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
if (-not $me.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Start-Process powershell.exe -Verb RunAs -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`""
    exit
}
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$Host.UI.RawUI.WindowTitle = 'IsoBox 添加软件'

try {
    Add-Type -AssemblyName System.Windows.Forms
    $dlg = New-Object System.Windows.Forms.OpenFileDialog
    $dlg.Title = '选择要放进隔离账户的软件安装包'
    $dlg.Filter = '安装包 (*.exe;*.msi)|*.exe;*.msi|所有文件 (*.*)|*.*'
    $dlg.InitialDirectory = (New-Object -ComObject Shell.Application).NameSpace('shell:Downloads').Self.Path
    if ($dlg.ShowDialog() -ne [System.Windows.Forms.DialogResult]::OK) { exit }
    $installer = $dlg.FileName
    Write-Host "安装包: $installer`n"

    # 取文件名开头的第一段字母作为默认英文名，例如 BaiduNetdisk_7.50.0.exe 得到 baidunetdisk
    $guess = if ([IO.Path]::GetFileNameWithoutExtension($installer).ToLower() -match '[a-z]+') { $Matches[0] } else { '' }
    if (-not $guess) { $guess = 'app' }

    $name = Read-Host "英文名，用作文件夹名，只能用小写字母、数字、短横线（直接回车用 $guess）"
    if (-not $name) { $name = $guess }
    $display = Read-Host '快捷方式上显示的名字，例如 百度网盘（直接回车用英文名）'
    $startup = (Read-Host '开机自动启动吗？输入 y 表示是，直接回车表示否') -eq 'y'

    # 列出能建库的盘，默认系统盘
    Write-Host ''
    & $setup drives
    $sys = $env:SystemDrive.Substring(0, 1).ToUpper()
    $lib = Read-Host "装进哪个盘的库？输入盘符，直接回车用 $sys 盘"
    if (-not $lib) { $lib = $sys }
    $lib = $lib.Substring(0, 1).ToUpper()

    $method = Read-Host '安装方式：直接回车用快速安装（拆包，推荐）；输入 2 使用原版安装程序'
    $useInstaller = $method -eq '2'
    Write-Host ''

    $cmdArgs = @('add', $name, $installer, '--library', $lib)
    if ($display) { $cmdArgs += @('--display', $display) }
    if ($startup) { $cmdArgs += '--startup' }
    if ($useInstaller) { $cmdArgs += '--install' }

    & $setup @cmdArgs
    if ($LASTEXITCODE -ne 0 -and -not $useInstaller) {
        Write-Host ''
        $retry = Read-Host '没有成功。如果上面说的是拆包失败，可以改用原版安装程序。要试试吗？输入 y 表示是'
        if ($retry -eq 'y') {
            Write-Host ''
            & $setup @($cmdArgs + '--install')
        }
    }
} catch {
    Write-Host "出错了: $($_.Exception.Message)" -ForegroundColor Red
}
Write-Host ''
Read-Host '按回车关闭窗口' | Out-Null
