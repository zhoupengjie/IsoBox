# Runs inside Windows Sandbox (LogonCommand) as the built-in admin user.
# Exercises the whole IsoBox lifecycle and writes PASS/FAIL lines to C:\Logs\result.txt.
# Keep this file ASCII: Windows PowerShell 5.1 reads BOM-less files with the ANSI code page.

$ErrorActionPreference = 'Continue'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$env:ISOBOX_NONINTERACTIVE = '1'

$log = 'C:\Logs'
$res = "$log\result.txt"
$cmdlog = "$log\commands.txt"
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
# icacls shows a full deny as (N) and partial denies as (DENY)(...)
function HasDeny([string]$p) { $a = Acl $p; ($a -match '\(N\)') -or ($a -match '\(DENY\)') }
function Lnk([string]$p) { (New-Object -ComObject WScript.Shell).CreateShortcut($p) }
function Section([string]$s) { Log ''; Log "== $s" }
function RegHas([string]$key, [string]$name) { reg query $key /v $name 2>$null | Out-Null; return ($LASTEXITCODE -eq 0) }
$hideKey = 'HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList'
$uninstKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\IsoBox'
$smLnk = "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\IsoBox\IsoBox.lnk"
# The installer shows its log only in its own window; in silent mode only the exit code is visible
function RunInstaller([string[]]$a) {
    $p = Start-Process 'C:\Stage\IsoBox-setup.exe' -ArgumentList $a -Wait -PassThru
    Add-Content -Encoding utf8 $cmdlog ("`r`n>>> IsoBox-setup.exe " + ($a -join ' ') + "`r`nexit=" + $p.ExitCode)
    return $p.ExitCode
}

Set-Content -Encoding utf8 $res "IsoBox sandbox test $(Get-Date -Format o)"
Set-Content -Encoding utf8 $cmdlog ''

try {
    Section 'prepare'
    # A second fixed NTFS drive T: backed by a virtual disk, to test drive-level protection and libraries
    "create vdisk file=C:\testdrive.vhdx maximum=1024 type=expandable`r`nattach vdisk`r`ncreate partition primary`r`nformat fs=ntfs quick label=TestT`r`nassign letter=T" |
        Set-Content -Encoding ascii C:\dp.txt
    diskpart /s C:\dp.txt | Out-File -Encoding utf8 "$log\diskpart.txt"
    $hasT = Test-Path 'T:\'
    Check 'virtual disk T: available' $hasT
    if ($hasT) { Set-Content T:\secret.txt 'secret' }
    New-Item -ItemType Directory -Force C:\TestData | Out-Null
    Set-Content C:\TestData\secret.txt 'secret'
    $desk = [Environment]::GetFolderPath('Desktop')
    Set-Content "$desk\secret.txt" 'secret'
    $x = Join-Path $env:USERPROFILE 'Downloads\IsoBox'

    Section 'installer (silent first install, creates the account)'
    Check 'sandbox has no system 7-Zip, so the bundled one is used' (-not (Test-Path 'C:\Program Files\7-Zip\7z.exe'))
    $deny = 'C:\TestData'
    if ($hasT) { $deny += ';T:\' }
    $code = RunInstaller @('/S', "/PROTECT=$deny")
    Check 'installer exits 0' ($code -eq 0) "exit=$code"
    if ($code -ne 0) { Run $setup @('install') | Out-Null }
    Check 'installer: program files' ((Test-Path $setup) -and (Test-Path $cli) -and (Test-Path "$IB\isobox-launch.exe") -and (Test-Path "$IB\isobox-gui.exe"))
    Check 'installer: bundled 7-Zip' ((Test-Path "$IB\7-Zip\7z.exe") -and (Test-Path "$IB\7-Zip\7z.dll") -and (Test-Path "$IB\7-Zip\License.txt"))
    Check 'installer: config and uninstaller' ((Test-Path "$IB\isobox.toml") -and (Test-Path "$IB\uninstall.exe"))
    Check 'installer: install dir read-only for Users' ((Acl $IB) -match 'BUILTIN\\Users:\(OI\)\(CI\)\(RX\)')
    Check 'installer: install dir does not inherit' (-not ((Acl $IB) -match '\(I\)')) (Acl $IB)
    $un = Get-ItemProperty $uninstKey -ErrorAction SilentlyContinue
    Check 'installer: Apps entry' ($un -and $un.DisplayName -eq 'IsoBox' -and $un.DisplayVersion -and ($un.UninstallString -like '*uninstall.exe*') -and $un.QuietUninstallString)
    Check 'installer: start menu shortcut for all users' ((Lnk $smLnk).TargetPath -like "$IB\isobox-gui.exe")
    $cfgText = Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8
    Check 'installer: protect choice saved in config' (($cfgText -match 'protect_auto = false') -and ($cfgText -match 'TestData'))

    Section 'account created by the installer'
    $u = Get-LocalUser isobox -ErrorAction SilentlyContinue
    Check 'account created' ([bool]$u)
    $sid = if ($u) { $u.SID.Value } else { 'none' }
    Check 'account hidden from login screen' (RegHas $hideKey 'isobox')
    Check 'profile created' (Test-Path 'C:\Users\isobox')
    Check 'C library marker' (Test-Path "$IB\library.toml")
    Check 'C apps dir writable by isobox' ((Acl "$IB\apps") -match 'isobox:\(OI\)\(CI\)\(M\)')
    Check 'deny on C:\TestData' ((Acl 'C:\TestData') -match 'isobox:\(OI\)\(CI\)\(N\)')
    if ($hasT) { Check 'deny on T:\' ((Acl 'T:\') -match 'isobox:\(OI\)\(CI\)\(N\)') }
    $backups = @(Get-ChildItem "$IB\backup" -Directory -ErrorAction SilentlyContinue)
    Check 'ACL backups created before deny' ($backups.Count -ge 1) "count=$($backups.Count)"
    Check 'exchange dir created' (Test-Path $x)
    Check 'exchange dir granted' ((Acl $x) -match 'isobox:\(OI\)\(CI\)\(M\)')
    Run $setup @('drives') | Out-Null
    Run $setup @('library', 'list') | Out-Null

    Section 'install can be re-run'
    $nb = @(Get-ChildItem "$IB\backup" -Directory -ErrorAction SilentlyContinue).Count
    Check 'install --apply again exits 0' ((Run $setup @('install', '--apply')) -eq 0)
    Check 'no new ACL backup on re-run' (@(Get-ChildItem "$IB\backup" -Directory).Count -eq $nb)
    Check 'still protected after re-run' ((Acl 'C:\TestData') -match 'isobox:\(OI\)\(CI\)\(N\)')
    Check 'exchange dir must not be a drive root' ((Run $setup @('set-exchange', 'C:\')) -ne 0)
    Check 'exchange dir must not be the user profile' ((Run $setup @('set-exchange', $env:USERPROFILE)) -ne 0)

    Section 'add (quick install = unpack)'
    Check 'add exits 0' ((Run $setup @('add', 'probe', 'C:\Stage\test\probe-setup.exe', '--display', 'Probe')) -eq 0)
    Check 'program unpacked into C library' (Test-Path "$IB\apps\probe\IsoProbe.exe")
    Check 'helper exe ranked below main program' ((Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8) -match "exe = `"IsoProbe.exe`"")
    Check 'config records library C' ((Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8) -match '(?s)\[apps\.probe\].*library = "C"')
    $lnk = "$desk\Probe.lnk"
    Check 'desktop shortcut created' (Test-Path $lnk)
    if (Test-Path $lnk) {
        Check 'shortcut icon is the program in the library' ((Lnk $lnk).IconLocation -like "*apps\probe\IsoProbe.exe*") (Lnk $lnk).IconLocation
        Check 'shortcut targets launcher' ((Lnk $lnk).TargetPath -like "*IsoBox\isobox-launch.exe")
    }

    Section 'installer again (update keeps everything)'
    Set-Content "$IB\foreign.txt" 'not ours'
    Check 'installer refuses a folder with foreign files' ((RunInstaller @('/S')) -eq 2)
    Remove-Item "$IB\foreign.txt" -Force
    $nb = @(Get-ChildItem "$IB\backup" -Directory).Count
    $code = RunInstaller @('/S')
    Check 'update exits 0' ($code -eq 0) "exit=$code"
    Check 'update keeps app config' ((Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8) -match '\[apps\.probe\]')
    Check 'update keeps the app in the library' (Test-Path "$IB\apps\probe\IsoProbe.exe")
    Check 'update keeps apps dir permissions' ((Acl "$IB\apps") -match 'isobox:\(OI\)\(CI\)\(M\)')
    Check 'update keeps protection, no new backup' (((Acl 'C:\TestData') -match 'isobox:\(OI\)\(CI\)\(N\)') -and (@(Get-ChildItem "$IB\backup" -Directory).Count -eq $nb))

    Section 'isolation probe (running as isobox)'
    Remove-Item "$x\probe-result.txt" -ErrorAction SilentlyContinue
    $a = @('run', 'probe', '--wait', '--', $x)
    if ($hasT) { $a += 'T:\secret.txt' }
    Run $cli $a | Out-Null
    $r = @{}
    Get-Content "$x\probe-result.txt" -ErrorAction SilentlyContinue | ForEach-Object { $k, $v = $_ -split '=', 2; $r[$k] = $v }
    Add-Content -Encoding utf8 $cmdlog ("probe result:`r`n" + ((Get-Content "$x\probe-result.txt" -ErrorAction SilentlyContinue) -join "`r`n"))
    Check 'probe ran as isobox' ($r['user'] -eq 'isobox') $r['user']
    Check 'protected file not readable' ($r['read_protected_file'] -like 'DENIED*') $r['read_protected_file']
    Check 'protected dir not listable' ($r['list_protected_dir'] -like 'DENIED*') $r['list_protected_dir']
    Check 'main user profile not readable' ($r['read_main_profile'] -like 'DENIED*') $r['read_main_profile']
    if ($hasT) { Check 'protected drive not readable' ($r['read_protected_drive'] -like 'DENIED*') $r['read_protected_drive'] }
    Check 'own app dir writable' ($r['write_own_app_dir'] -eq 'WRITE_OK') $r['write_own_app_dir']
    Check 'install root not writable' ($r['write_install_root'] -like 'DENIED*') $r['write_install_root']
    Check 'exchange writable' ($r['write_exchange'] -eq 'WRITE_OK') $r['write_exchange']
    Check 'ProgramData leftover created for cleanup test' ($r['create_programdata_leftover'] -eq 'OK') $r['create_programdata_leftover']

    if ($hasT) {
        Section 'second library on T: and moving apps'
        Check 'library add T' (((Run $setup @('library', 'add', 'T')) -eq 0) -and (Test-Path 'T:\Program Files\IsoBox\library.toml'))
        Check 'T library root has no inherited deny' (-not (HasDeny 'T:\Program Files\IsoBox'))
        Check 'move-app to T' (((Run $setup @('move-app', 'probe', '--to', 'T')) -eq 0) -and (Test-Path 'T:\Program Files\IsoBox\apps\probe\IsoProbe.exe') -and -not (Test-Path "$IB\apps\probe"))
        Check 'config records library T' ((Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8) -match '(?s)\[apps\.probe\].*library = "T"')
        if (Test-Path $lnk) { Check 'shortcut icon follows the move' ((Lnk $lnk).IconLocation -like 'T:\*') (Lnk $lnk).IconLocation }
        Remove-Item "$x\probe-result.txt" -ErrorAction SilentlyContinue
        Run $cli @('run', 'probe', '--wait', '--no-theme-sync', '--', $x) | Out-Null
        $r2 = @{}
        Get-Content "$x\probe-result.txt" -ErrorAction SilentlyContinue | ForEach-Object { $k, $v = $_ -split '=', 2; $r2[$k] = $v }
        Check 'probe runs from T library' ($r2['exe_dir'] -like 'T:\*') $r2['exe_dir']
        Check 'moved app dir writable by isobox' ($r2['write_own_app_dir'] -eq 'WRITE_OK') $r2['write_own_app_dir']
        Check 'library remove refused while not empty' ((Run $setup @('library', 'remove', 'T')) -ne 0)
        Check 'move-app back to C' (((Run $setup @('move-app', 'probe', '--to', 'C')) -eq 0) -and (Test-Path "$IB\apps\probe\IsoProbe.exe"))
        Check 'library remove T' (((Run $setup @('library', 'remove', 'T')) -eq 0) -and -not (Test-Path 'T:\Program Files\IsoBox'))
        Check 'Program Files created by IsoBox on T removed' (-not (Test-Path 'T:\Program Files'))
    }

    Section 'migrate from an old-style install'
    $old = 'C:\OldIsoBox'
    New-Item -ItemType Directory -Force "$old\icons", 'C:\Users\isobox\Apps\legacy' | Out-Null
    Copy-Item "$IB\apps\probe\IsoProbe.exe" 'C:\Users\isobox\Apps\legacy\IsoProbe.exe'
    Copy-Item "$IB\apps\probe\IsoProbe.exe" "$old\icons\legacy.exe"
    foreach ($f in 'isobox.exe', 'isobox-setup.exe', 'isobox-launch.exe') { Copy-Item "$IB\$f" $old }
    $oldCfg = (Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8) + "`r`n[apps.legacy]`r`nportable = true`r`nexe = 'IsoProbe.exe'`r`ndisplay = 'Legacy'`r`n"
    Set-Content -Encoding utf8 "$old\isobox.toml" $oldCfg
    $ll = Lnk "$desk\Legacy.lnk"; $ll.TargetPath = "$old\isobox-launch.exe"; $ll.Arguments = 'legacy'; $ll.IconLocation = "$old\icons\legacy.exe,0"; $ll.Save()
    # Reproduce a real-world case: a terminal is sitting in the old directory, so it cannot be removed
    $holder = Start-Process cmd.exe -ArgumentList '/k' -WorkingDirectory $old -WindowStyle Hidden -PassThru
    Start-Sleep -Seconds 1
    Check 'migrate exits 0 even if old dir is in use' ((Run $setup @('migrate', '--from', $old)) -eq 0)
    Check 'old dir emptied while in use' ((Test-Path $old) -and -not (Get-ChildItem $old -Force))
    Stop-Process -Id $holder.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    Check 'second migrate run only cleans up' ((Run $setup @('migrate', '--from', $old)) -eq 0)
    Check 'legacy app moved into C library' ((Test-Path "$IB\apps\legacy\IsoProbe.exe") -and -not (Test-Path 'C:\Users\isobox\Apps\legacy'))
    Check 'legacy config records library C' ((Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8) -match '(?s)\[apps\.legacy\].*library = "C"')
    Check 'old install dir removed' (-not (Test-Path $old))
    Check 'template config kept as backup' (Test-Path "$IB\isobox.toml.template")
    Check 'legacy shortcut retargeted' ((Lnk "$desk\Legacy.lnk").TargetPath -like "$IB\isobox-launch.exe") (Lnk "$desk\Legacy.lnk").TargetPath
    Check 'legacy shortcut icon from library' ((Lnk "$desk\Legacy.lnk").IconLocation -like "*apps\legacy\IsoProbe.exe*") (Lnk "$desk\Legacy.lnk").IconLocation
    if (Test-Path $lnk) { Check 'library app shortcut icon kept after migrate' ((Lnk $lnk).IconLocation -like "*apps\probe\IsoProbe.exe*") (Lnk $lnk).IconLocation }
    Remove-Item "$x\probe-result.txt" -ErrorAction SilentlyContinue
    Run $cli @('run', 'legacy', '--wait', '--no-theme-sync', '--', $x) | Out-Null
    $r3 = @{}
    Get-Content "$x\probe-result.txt" -ErrorAction SilentlyContinue | ForEach-Object { $k, $v = $_ -split '=', 2; $r3[$k] = $v }
    Check 'migrated app runs from C library' ($r3['exe_dir'] -like "$IB\apps\legacy*") $r3['exe_dir']

    Section 'restore ACL from backup'
    $b = Get-ChildItem "$IB\backup" -Directory | Where-Object { (Get-Content "$($_.FullName)\target.txt" -Raw).Trim() -eq 'C:\TestData' } | Select-Object -First 1
    Check 'backup for C:\TestData exists' ([bool]$b)
    if ($b) {
        Check 'restore-acl exits 0' ((Run $setup @('restore-acl', $b.FullName)) -eq 0)
        Check 'deny gone after restore' (-not (HasDeny 'C:\TestData'))
        Check 'children ACL intact after restore' ((Acl 'C:\TestData\secret.txt') -match 'BUILTIN\\Administrators')
    }

    Section 'commands used by the GUI'
    Check 'protect add exits 0' ((Run $setup @('protect', 'add', 'C:\TestData')) -eq 0)
    Check 'protect add applied deny' ((Acl 'C:\TestData') -match 'isobox:\(OI\)\(CI\)\(N\)')
    $cfgText = Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8
    Check 'protect add recorded in config' (($cfgText -match 'C:\\\\TestData|C:\\TestData') -and ($cfgText -match 'protect_auto = false'))
    Check 'protect remove exits 0' ((Run $setup @('protect', 'remove', 'C:\TestData')) -eq 0)
    Check 'protect remove lifted deny' (-not (HasDeny 'C:\TestData'))
    New-Item -ItemType Directory -Force C:\Exchange2 | Out-Null
    Check 'set-exchange exits 0' ((Run $setup @('set-exchange', 'C:\Exchange2')) -eq 0)
    Check 'new exchange granted' ((Acl 'C:\Exchange2') -match 'isobox:\(OI\)\(CI\)\(M\)')
    Check 'old exchange grant removed' (-not ((Acl $x) -match '\\isobox:'))
    Check 'config records new exchange' ((Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8) -match 'exchange_dir = .C:\\+Exchange2')
    Run $setup @('set-exchange', $x) | Out-Null
    $glog = "$log\setup-log-test.txt"
    & $setup --log $glog --non-interactive library list | Out-Null
    Check '--log writes output to file' ((Test-Path $glog) -and ((Get-Content $glog -Raw -Encoding UTF8) -match 'C '))
    $st = $null
    try { $st = (& $cli status --json) -join "`n" | ConvertFrom-Json } catch { }
    Check 'status --json parses' ([bool]$st)
    if ($st) {
        Check 'status reports account' ($st.account.exists -eq $true)
        Check 'status lists apps' (@($st.apps).Count -ge 2) "$(@($st.apps).Count)"
        Check 'status lists C library' (@($st.libraries | Where-Object { $_.drive -eq 'C' }).Count -eq 1)
    }
    $gui = Start-Process "$IB\isobox-gui.exe" -PassThru
    $title = ''
    for ($i = 0; $i -lt 30 -and -not $title; $i++) { Start-Sleep -Milliseconds 500; $gui.Refresh(); $title = $gui.MainWindowTitle }
    Check 'GUI window opens' ($title -eq 'IsoBox') "title='$title' exited=$($gui.HasExited)"
    Start-Sleep -Seconds 2
    Check 'GUI keeps running' (-not $gui.HasExited)
    Stop-Process -Id $gui.Id -Force -ErrorAction SilentlyContinue

    Section 'remove-app'
    Check 'remove-app exits 0' ((Run $setup @('remove-app', 'probe')) -eq 0)
    Check 'program dir removed' (-not (Test-Path "$IB\apps\probe"))
    Check 'shortcut removed' (-not (Test-Path $lnk))
    Check 'config entry removed' (-not ((Get-Content "$IB\isobox.toml" -Raw -Encoding UTF8) -match '\[apps\.probe\]'))

    Section 'uninstall (system uninstaller, keeping the account files)'
    # Stand-in for a chat history inside the isolated account's Documents
    $chat = 'C:\Users\isobox\Documents\xwechat_files\wxid_test'
    New-Item -ItemType Directory -Force $chat | Out-Null
    Set-Content "$chat\msg.db" 'chat'
    Start-Process "$IB\uninstall.exe" -ArgumentList '/S' -Wait | Out-Null
    # The uninstaller copies itself to %TEMP% and runs from there, so wait for that copy as well
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ((Get-Process -Name 'Un', 'Un_*', 'Au_*' -ErrorAction SilentlyContinue) -and $sw.Elapsed.TotalSeconds -lt 300) { Start-Sleep -Seconds 1 }
    Check 'account removed' (-not (Get-LocalUser isobox -ErrorAction SilentlyContinue))
    Check 'profile removed' (-not (Test-Path 'C:\Users\isobox'))
    Check 'profile list entry removed' (-not (Test-Path "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$sid"))
    Check 'install dir removed (library, backups, program files)' (-not (Test-Path $IB)) ((Get-ChildItem $IB -Recurse -Force -ErrorAction SilentlyContinue | Select-Object -First 8 -ExpandProperty FullName) -join ', ')
    Check 'Apps entry removed' (-not (Test-Path $uninstKey))
    Check 'start menu shortcut removed' (-not (Test-Path $smLnk))
    $kept = Get-ChildItem $x -Recurse -Filter msg.db -ErrorAction SilentlyContinue | Select-Object -First 1
    Check 'chat history kept in the exchange folder' ($kept -and ((Get-Content $kept.FullName) -eq 'chat')) "$($kept.FullName)"
    if ($kept) {
        Check 'kept files have no rule for the deleted account' (-not ((Acl $kept.FullName) -match [regex]::Escape($sid))) (Acl $kept.FullName)
        Check 'kept files inherit the exchange folder permissions' ((Acl $kept.FullName) -match [regex]::Escape("$($env:USERNAME):(I)(F)")) (Acl $kept.FullName)
        Check 'kept files owned by the user' ((Get-Acl $kept.FullName).Owner -like "*\$env:USERNAME") (Get-Acl $kept.FullName).Owner
    }
    Check 'no rule left on C:\TestData' (-not ((Acl 'C:\TestData') -match [regex]::Escape($sid)))
    if ($hasT) { Check 'no rule left on T:\' (-not (((Acl 'T:\') -match [regex]::Escape($sid)) -or (HasDeny 'T:\'))) (Acl 'T:\') }
    Check 'no rule left on exchange dir' (-not ((Acl $x) -match [regex]::Escape($sid)))
    Check 'ProgramData leftover removed' (-not (Test-Path 'C:\ProgramData\IsoProbeLeftover'))
    Check 'credential removed' (-not ((cmdkey /list | Out-String) -match 'isobox:isobox'))
    Check 'login-screen setting removed' (-not (RegHas $hideKey 'isobox'))
    $left = Get-ChildItem $desk -Filter *.lnk | Where-Object { (Lnk $_.FullName).TargetPath -like '*isobox-launch.exe' }
    Check 'no IsoBox shortcuts left on desktop' (-not $left) (($left.Name) -join ',')
    Check 'no theme watcher left running' (-not (Get-Process isobox-launch -ErrorAction SilentlyContinue))
} catch {
    $script:fails++
    Log "FAIL  unexpected error: $($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
}

Log ''
Log "SUMMARY passed=$($script:passes) failed=$($script:fails)"
New-Item -ItemType File -Force "$log\done.flag" | Out-Null
