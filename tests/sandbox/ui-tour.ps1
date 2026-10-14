# Runs inside Windows Sandbox (LogonCommand). Clicks through the installer and the uninstaller with
# the default choices and saves a screenshot of every page to C:\Logs\shots, plus result.txt.
# Started by: powershell -ExecutionPolicy Bypass -File tests\sandbox\run.ps1 -UiTour
# Keep this file ASCII: Windows PowerShell 5.1 reads BOM-less files with the ANSI code page.

$ErrorActionPreference = 'Continue'
$script:failed = 0
$log = 'C:\Logs'
$shots = "$log\shots"
$res = "$log\result.txt"
New-Item -ItemType Directory -Force $shots | Out-Null
Set-Content -Encoding utf8 $res "IsoBox UI tour $(Get-Date -Format o)"
function Log([string]$s) { Add-Content -Encoding utf8 $res $s }

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class W {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr dlg, int id);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    // Visible top-level dialogs (#32770) of a process: the wizard and any message box
    public static IntPtr[] Dialogs(uint pid) {
        var list = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            var c = new StringBuilder(64); GetClassName(h, c, 64);
            if (p == pid && IsWindowVisible(h) && c.ToString() == "#32770") list.Add(h);
            return true;
        }, IntPtr.Zero);
        return list.ToArray();
    }
    public static string Text(IntPtr h) { var s = new StringBuilder(512); GetWindowText(h, s, 512); return s.ToString(); }
}
'@
[W]::SetProcessDPIAware() | Out-Null
$WM_COMMAND = 0x0111

$script:n = 0
function Shot([string]$name) {
    $script:n++
    $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
    $file = '{0}\{1:D2}-{2}.png' -f $shots, $script:n, $name
    $bmp.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Log "shot $file"
}

# First running process with one of these names that has a window
function WaitWindow([string[]]$procNames, [int]$sec = 30) {
    for ($i = 0; $i -lt $sec * 2; $i++) {
        $p = Get-Process -Name $procNames -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
        if ($p) { return $p }
        Start-Sleep -Milliseconds 500
    }
    return $null
}

# Press button $id of dialog $dlg the way a real click reports it: WM_COMMAND to the dialog.
# (BM_CLICK and UI Automation Invoke do not reliably reach NSIS pages.)
function Click([IntPtr]$dlg, [int]$id) { [W]::PostMessage($dlg, $WM_COMMAND, [IntPtr]$id, [W]::GetDlgItem($dlg, $id)) | Out-Null }

# Click the wizard's Next/Install/Finish button (control id 1) once it is enabled.
# A message box that pops up meanwhile is captured, logged and answered with OK / No.
function ClickNext($proc, [int]$waitSec = 15) {
    $main = $proc.MainWindowHandle
    for ($i = 0; $i -lt $waitSec * 2; $i++) {
        foreach ($d in [W]::Dialogs([uint32]$proc.Id)) {
            if ($d -eq $main) { continue }
            Shot 'messagebox'
            Log ('message box: ' + [W]::Text($d))
            foreach ($id in 1, 2, 7) {
                $b = [W]::GetDlgItem($d, $id)
                if ($b -ne [IntPtr]::Zero) { Click $d $id; break }
            }
            Start-Sleep -Seconds 1
        }
        $btn = [W]::GetDlgItem($main, 1)
        if ($btn -ne [IntPtr]::Zero -and [W]::IsWindowEnabled($btn) -and [W]::IsWindowEnabled($main)) {
            Click $main 1
            Start-Sleep -Milliseconds 1500
            return $true
        }
        Start-Sleep -Milliseconds 500
    }
    Log "next button not clickable after $waitSec s"
    return $false
}

function NextText($proc) { [W]::Text([W]::GetDlgItem($proc.MainWindowHandle, 1)) }

try {
    # A second fixed NTFS drive, so the protect page has a drive to show
    "create vdisk file=C:\testdrive.vhdx maximum=1024 type=expandable`r`nattach vdisk`r`ncreate partition primary`r`nformat fs=ntfs quick label=Data`r`nassign letter=T" |
        Set-Content -Encoding ascii C:\dp.txt
    diskpart /s C:\dp.txt | Out-Null

    Start-Process 'C:\Stage\IsoBox-setup.exe'
    $w = WaitWindow @('IsoBox-setup')
    if (-not $w) { throw 'installer window did not appear' }
    Start-Sleep -Seconds 2
    Shot 'install-welcome'
    ClickNext $w | Out-Null
    Shot 'install-notice'
    ClickNext $w | Out-Null
    Shot 'install-protect'
    ClickNext $w | Out-Null
    Start-Sleep -Seconds 2
    Shot 'install-progress'
    # The Next button stays disabled until the install section finishes
    ClickNext $w 600 | Out-Null
    Shot 'install-finish'
    Log ("finish button: " + (NextText $w))
    ClickNext $w | Out-Null
    $gui = WaitWindow @('isobox-gui') 30
    Start-Sleep -Seconds 4
    Shot 'gui-after-install'
    Log ("gui launched: " + [bool]$gui)
    Get-Process isobox-gui -ErrorAction SilentlyContinue | Stop-Process -Force

    Start-Process 'C:\Program Files\IsoBox\uninstall.exe'
    $u = WaitWindow @('Un', 'Un_A', 'Au_') 30
    if (-not $u) { Log ('processes: ' + ((Get-Process).Name -join ' ')) }
    if (-not $u) { throw 'uninstaller window did not appear' }
    Start-Sleep -Seconds 2
    Shot 'uninstall-choice'
    ClickNext $u | Out-Null
    Start-Sleep -Seconds 2
    Shot 'uninstall-progress'
    ClickNext $u 600 | Out-Null
    Shot 'uninstall-finish'
    ClickNext $u | Out-Null
    Log ("install dir left: " + (Test-Path 'C:\Program Files\IsoBox'))
    Log ("account left: " + [bool](Get-LocalUser isobox -ErrorAction SilentlyContinue))
} catch {
    $script:failed = 1
    Log "ERROR $($_.Exception.Message) at line $($_.InvocationInfo.ScriptLineNumber)"
    Shot 'error'
}
Log "SUMMARY passed=$(1 - $script:failed) failed=$($script:failed)"
New-Item -ItemType File -Force "$log\done.flag" | Out-Null
