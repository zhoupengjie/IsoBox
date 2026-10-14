# Run the full IsoBox lifecycle test inside Windows Sandbox: installer, add, isolation, libraries,
# migrate, restore, GUI commands, upgrade, and the system uninstaller.
#   powershell -ExecutionPolicy Bypass -File tests\sandbox\run.ps1
# Requires: Windows Sandbox enabled, 7-Zip installed in C:\Program Files\7-Zip (bundled into the
# installer and used to build the test package; the sandbox itself has no 7-Zip), no other sandbox running.
# Exit code 0 when every check passed. Results: target\sandbox-test\logs\result.txt
#   -UiTour   instead click through the installer and uninstaller pages with the default choices and
#             save a screenshot of each page to target\sandbox-test\logs\shots (to look at the pages)
#   -UpgradeFromDeploy  instead set up the old deploy.ps1 version (commit 5eb21d1, built once into
#             target\old-src), upgrade it with the new installer and uninstall (upgrade-from-deploy.ps1)
#   -NoBuild  reuse the newest installer in target\installer
#   -StageOnly  only prepare target\sandbox-test\stage (installer and test package) and stop.
#               CI does this, then runs run-in-sandbox.ps1 directly on its throwaway runner
# Keep this file ASCII.

param([int]$TimeoutMinutes = 20, [switch]$UiTour, [switch]$UpgradeFromDeploy, [switch]$NoBuild, [switch]$StageOnly)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$work = Join-Path $root 'target\sandbox-test'
$stage = Join-Path $work 'stage'
$logs = Join-Path $work 'logs'
$sevenDir = 'C:\Program Files\7-Zip'
# Last commit that was installed with deploy.ps1 instead of the installer
$deployCommit = '5eb21d1'

if (-not $StageOnly -and (Get-Process WindowsSandboxRemoteSession, WindowsSandboxServer -ErrorAction SilentlyContinue)) {
    throw 'Another Windows Sandbox is running. Close it first.'
}

if (-not $NoBuild) {
    Write-Host 'Building the installer and the probe...'
    & powershell -NoProfile -ExecutionPolicy Bypass -File "$root\installer\build.ps1"
    if ($LASTEXITCODE -ne 0) { throw 'installer build failed' }
}
cargo build --release --manifest-path "$root\Cargo.toml" --example probe
if ($LASTEXITCODE -ne 0) { throw 'cargo build --example probe failed' }
$installer = Get-ChildItem "$root\target\installer\IsoBox-*-setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
if (-not $installer) { throw 'installer not found in target\installer' }

$oldSrc = Join-Path $root 'target\old-src'
if ($UpgradeFromDeploy -and -not (Test-Path "$oldSrc\target\release\isobox-gui.exe")) {
    Write-Host "Building the old deploy.ps1 version ($deployCommit) in $oldSrc ..."
    if (-not (Test-Path $oldSrc)) {
        # After target\ was deleted git still remembers the worktree; forget it before adding it again
        git -C $root worktree prune
        git -C $root worktree add --detach $oldSrc $deployCommit
        if ($LASTEXITCODE -ne 0) { throw 'git worktree add failed' }
    }
    Push-Location $oldSrc
    cargo build --release --workspace --bins
    $code = $LASTEXITCODE
    Pop-Location
    if ($code -ne 0) { throw 'building the old version failed' }
}

if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\test", $logs, "$work\pkg" | Out-Null
Copy-Item $installer.FullName "$stage\IsoBox-setup.exe"
$script = if ($UiTour) { 'ui-tour.ps1' } elseif ($UpgradeFromDeploy) { 'upgrade-from-deploy.ps1' } else { 'run-in-sandbox.ps1' }
Copy-Item "$PSScriptRoot\$script" "$stage\test"
if ($UpgradeFromDeploy) {
    New-Item -ItemType Directory -Force "$stage\old\bin", "$stage\old\tools" | Out-Null
    foreach ($f in 'isobox.exe', 'isobox-setup.exe', 'isobox-launch.exe', 'isobox-gui.exe') { Copy-Item "$oldSrc\target\release\$f" "$stage\old\bin" }
    Copy-Item "$oldSrc\deploy.ps1", "$oldSrc\isobox.toml" "$stage\old"
    Copy-Item "$oldSrc\tools\*.ps1" "$stage\old\tools"
}

# A 7-Zip self-extracting "installer" with the probe as main program plus a helper exe
Copy-Item "$root\target\release\examples\probe.exe" "$work\pkg\IsoProbe.exe"
Copy-Item "$root\target\release\examples\probe.exe" "$work\pkg\unins000.exe"
& "$sevenDir\7z.exe" a -bso0 -bsp0 "-sfx$sevenDir\7z.sfx" "$stage\test\probe-setup.exe" "$work\pkg\*"
if ($LASTEXITCODE -ne 0) { throw '7z sfx failed' }
if ($StageOnly) { Write-Host "Staged in $stage"; exit 0 }

# The old version has no bundled 7-Zip and needs the system one to add an app
$sevenMap = if ($UpgradeFromDeploy) {
    "<MappedFolder><HostFolder>$sevenDir</HostFolder><SandboxFolder>C:\Program Files\7-Zip</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>"
} else { '' }
$wsb = @"
<Configuration>
  <VGpu>Disable</VGpu>
  <Networking>Disable</Networking>
  <AudioInput>Disable</AudioInput>
  <VideoInput>Disable</VideoInput>
  <PrinterRedirection>Disable</PrinterRedirection>
  <MemoryInMB>4096</MemoryInMB>
  <MappedFolders>
    <MappedFolder><HostFolder>$stage</HostFolder><SandboxFolder>C:\Stage</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
    <MappedFolder><HostFolder>$logs</HostFolder><SandboxFolder>C:\Logs</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
    $sevenMap
  </MappedFolders>
  <LogonCommand>
    <Command>powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\Stage\test\$script</Command>
  </LogonCommand>
</Configuration>
"@
$wsbPath = Join-Path $work 'test.wsb'
[IO.File]::WriteAllText($wsbPath, $wsb, (New-Object Text.UTF8Encoding $false))

# Start through WMI so the sandbox client is not a child of whatever launched this script
Write-Host 'Starting Windows Sandbox...'
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = "C:\Windows\System32\WindowsSandbox.exe `"$wsbPath`"" }
if ($r.ReturnValue -ne 0) { throw "Could not start Windows Sandbox: $($r.ReturnValue)" }

$sw = [Diagnostics.Stopwatch]::StartNew()
while (-not (Test-Path "$logs\done.flag") -and $sw.Elapsed.TotalMinutes -lt $TimeoutMinutes) { Start-Sleep -Seconds 3 }
$finished = Test-Path "$logs\done.flag"
Write-Host ("Sandbox run took {0:N0}s, finished={1}" -f $sw.Elapsed.TotalSeconds, $finished)

Get-Process WindowsSandboxRemoteSession, WindowsSandboxClient -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

if (Test-Path "$logs\result.txt") { Get-Content "$logs\result.txt" -Encoding UTF8 }
if (-not $finished) { Write-Host 'TIMEOUT: the test did not finish'; exit 2 }
$summary = Select-String -Path "$logs\result.txt" -Pattern 'SUMMARY passed=(\d+) failed=(\d+)'
if ($summary -and [int]$summary.Matches[0].Groups[2].Value -eq 0) { exit 0 } else { exit 1 }
