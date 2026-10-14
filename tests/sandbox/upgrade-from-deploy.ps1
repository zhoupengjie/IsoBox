# Upgrade test: a machine set up with the old deploy.ps1 (commit 5eb21d1, before the installer existed)
# is updated by running the new installer, then uninstalled. Writes PASS/FAIL lines to <Logs>\result.txt.
# Started by: powershell -ExecutionPolicy Bypass -File tests\sandbox\run.ps1 -UpgradeFromDeploy
# Runs inside Windows Sandbox as administrator; never run it on a real machine.
#   -Stage  IsoBox-setup.exe, test\probe-setup.exe, old\deploy.ps1, old\isobox.toml, old\tools\*.ps1, old\bin\*.exe
#   -Logs   where result.txt, commands.txt and done.flag are written
# Keep this file ASCII: Windows PowerShell 5.1 reads BOM-less files with the ANSI code page.

param(
    [string]$Stage = 'C:\Stage',
    [string]$Logs = 'C:\Logs'
)
$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$env:ISOBOX_NONINTERACTIVE = '1'

New-Item -ItemType Directory -Force $Logs | Out-Null
$res = "$Logs\result.txt"
$cmdlog = "$Logs\commands.txt"
$IB = 'C:\Program Files\IsoBox'
$setup = "$IB\isobox-setup.exe"
$cli = "$IB\isobox.exe"
$script:fails = 0
$script:passes = 0

function Log([string]$s) { Add-Content -Encoding utf8 $res $s }
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    if ($ok) { $script:passes++; Log "PASS  $name" } else { $script:fails++; Log "FAIL  $name  $detail" }
}
function Run([string]$exe, [string[]]$a) {
    $o = & $exe @a 2>&1 | Out-String
    $code = $LASTEXITCODE
    Add-Content -Encoding utf8 $cmdlog ("`r`n>>> " + (Split-Path $exe -Leaf) + ' ' + ($a -join ' ') + "`r`n" + $o + "exit=$code")
    return $code
}
function Acl([string]$p) { (icacls $p 2>&1 | Out-String) }
function Lnk([string]$p) { (New-Object -ComObject WScript.Shell).CreateShortcut($p) }
function Section([string]$s) { Log ''; Log "== $s" }
function Cfg { Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8 }
# Shortcuts in the user's own start menu group made by the old deploy.ps1 for add-app.ps1 / remove-app.ps1
function LegacyScriptLinks([string]$group) {
    @(Get-ChildItem $group -Filter *.lnk -ErrorAction SilentlyContinue | Where-Object { (Lnk $_.FullName).Arguments -match '(add|remove)-app\.ps1' })
}
$uninstKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\IsoBox'
$smLnk = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\IsoBox\IsoBox.lnk"
$userGroup = Join-Path ([Environment]::GetFolderPath('Programs')) 'IsoBox'
$desk = [Environment]::GetFolderPath('Desktop')
$x = Join-Path $env:USERPROFILE 'Downloads\IsoBox'

Set-Content -Encoding utf8 $res "IsoBox upgrade-from-deploy test $(Get-Date -Format o)"
Set-Content -Encoding utf8 $cmdlog ''

try {
    Section 'old version, deployed with deploy.ps1'
    New-Item -ItemType Directory -Force C:\TestData | Out-Null
    Set-Content C:\TestData\secret.txt 'secret'
    $o = & powershell -NoProfile -ExecutionPolicy Bypass -File "$Stage\old\deploy.ps1" -NoBuild -BinDir "$Stage\old\bin" 2>&1 | Out-String
    Add-Content -Encoding utf8 $cmdlog ("`r`n>>> old deploy.ps1`r`n" + $o)
    Check 'old deploy: scripts and files' ((Test-Path $setup) -and (Test-Path "$IB\add-app.ps1") -and (Test-Path "$IB\remove-app.ps1"))
    Check 'old deploy: start menu entries in the user group' ((Test-Path "$userGroup\IsoBox.lnk") -and ((LegacyScriptLinks $userGroup).Count -eq 2))
    Set-Content -Encoding utf8 "$IB\isobox.toml" "sandbox_user = `"isobox`"`r`ndefault_mode = `"user`"`r`ndeny_paths = ['C:\TestData']`r`n"
    Check 'old install exits 0' ((Run $setup @('install', '--apply')) -eq 0)
    Check 'old add exits 0' ((Run $setup @('add', 'probe', "$Stage\test\probe-setup.exe", '--display', 'Probe')) -eq 0)
    $u = Get-LocalUser isobox -ErrorAction SilentlyContinue
    $sid = if ($u) { $u.SID.Value } else { 'none' }
    Check 'old version: account, protection, app' ($u -and ((Acl 'C:\TestData') -match 'isobox:\(OI\)\(CI\)\(N\)') -and (Test-Path "$IB\apps\probe\IsoProbe.exe"))
    $chat = 'C:\Users\isobox\Documents\xwechat_files\wxid_test'
    New-Item -ItemType Directory -Force $chat | Out-Null
    Set-Content "$chat\msg.db" 'chat'
    $nb = @(Get-ChildItem "$IB\backup" -Directory -ErrorAction SilentlyContinue).Count

    Section 'upgrade by running the new installer'
    $p = Start-Process "$Stage\IsoBox-setup.exe" -ArgumentList '/S' -Wait -PassThru
    Add-Content -Encoding utf8 $cmdlog ("`r`n>>> IsoBox-setup.exe /S`r`nexit=" + $p.ExitCode)
    Check 'installer exits 0' ($p.ExitCode -eq 0) "exit=$($p.ExitCode)"
    $info = (& $setup installer-info 2>&1 | Out-String)
    Add-Content -Encoding utf8 $cmdlog ("`r`n>>> isobox-setup installer-info`r`n" + $info)
    Check 'new programs in place and the account is recognised' ($info -match 'account=ready') $info
    Check 'same account kept' ((Get-LocalUser isobox -ErrorAction SilentlyContinue).SID.Value -eq $sid)
    Check 'config kept' (((Cfg) -match '\[apps\.probe\]') -and ((Cfg) -match 'TestData'))
    Check 'app kept in the library' (Test-Path "$IB\apps\probe\IsoProbe.exe")
    Check 'chat data untouched' ((Get-Content "$chat\msg.db" -ErrorAction SilentlyContinue) -eq 'chat')
    Check 'still protected, no new backup' (((Acl 'C:\TestData') -match 'isobox:\(OI\)\(CI\)\(N\)') -and (@(Get-ChildItem "$IB\backup" -Directory).Count -eq $nb))
    Check 'legacy scripts removed' (-not (Test-Path "$IB\add-app.ps1") -and -not (Test-Path "$IB\remove-app.ps1"))
    Check 'legacy start menu entries removed' (((LegacyScriptLinks $userGroup).Count -eq 0) -and -not (Test-Path "$userGroup\IsoBox.lnk"))
    Check 'app shortcuts kept' ((Test-Path "$userGroup\Probe.lnk") -and (Test-Path "$desk\Probe.lnk"))
    Check 'new start menu entry for all users' ((Lnk $smLnk).TargetPath -like "$IB\isobox-gui.exe")
    Check 'uninstaller, Apps entry, bundled 7-Zip' ((Test-Path "$IB\uninstall.exe") -and (Test-Path $uninstKey) -and (Test-Path "$IB\7-Zip\7z.exe"))
    Check 'install dir still read-only for Users' ((Acl $IB) -match 'BUILTIN\\Users:\(OI\)\(CI\)\(RX\)')
    Remove-Item "$x\probe-result.txt" -ErrorAction SilentlyContinue
    Run $cli @('run', 'probe', '--wait', '--no-theme-sync', '--', $x) | Out-Null
    $r = @{}
    Get-Content "$x\probe-result.txt" -ErrorAction SilentlyContinue | ForEach-Object { $k, $v = $_ -split '=', 2; $r[$k] = $v }
    Check 'app still runs isolated after the upgrade' (($r['user'] -eq 'isobox') -and ($r['read_protected_file'] -like 'DENIED*')) "$($r['user']) $($r['read_protected_file'])"

    Section 'uninstall after the upgrade'
    Start-Process "$IB\uninstall.exe" -ArgumentList '/S' -Wait | Out-Null
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ((Get-Process -Name 'Un', 'Un_*', 'Au_*' -ErrorAction SilentlyContinue) -and $sw.Elapsed.TotalSeconds -lt 300) { Start-Sleep -Seconds 1 }
    Check 'account removed' (-not (Get-LocalUser isobox -ErrorAction SilentlyContinue))
    Check 'install dir removed' (-not (Test-Path $IB)) ((Get-ChildItem $IB -Recurse -Force -ErrorAction SilentlyContinue | Select-Object -First 8 -ExpandProperty FullName) -join ', ')
    Check 'no rule left on C:\TestData' (-not ((Acl 'C:\TestData') -match [regex]::Escape($sid)))
    Check 'chat data kept in the exchange folder' ([bool](Get-ChildItem $x -Recurse -Filter msg.db -ErrorAction SilentlyContinue))
    Check 'user start menu group and desktop shortcut removed' (-not (Test-Path $userGroup) -and -not (Test-Path "$desk\Probe.lnk"))
    Check 'Apps entry and all-users shortcut removed' (-not (Test-Path $uninstKey) -and -not (Test-Path $smLnk))
} catch {
    $script:fails++
    Log "FAIL  unexpected error: $($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
}

Log ''
Log "SUMMARY passed=$($script:passes) failed=$($script:fails)"
New-Item -ItemType File -Force "$Logs\done.flag" | Out-Null
