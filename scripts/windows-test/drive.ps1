# Launch wt++, measure startup, drive it with keystrokes and mouse, take
# screenshots. Steps are separated by "|":
#   <text>            SendKeys syntax (^ Ctrl, + Shift, % Alt, {ENTER}, {(} ...)
#   shot:NAME         screenshot of the window to NAME.png
#   click:X,Y  rclick:X,Y  dclick:X,Y  move:X,Y   (window-relative pixels)
#   drag:X1,Y1,X2,Y2  left-button drag
#   rect              print the window size (and position)
#   setrect:X,Y,W,H   move/resize the window
#   ctrl:down|up      hold/release Ctrl
#   close             Alt+F4 and wait for exit
#   windows           list the app's visible windows
#   clip:TEXT         put TEXT on the clipboard (\n = newline)
#   sleep:MS
param([string]$StepList, [string]$ArgList = "")
$Steps = $StepList -split "\|"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int n);
  // The visible frame, without the invisible resize borders (which would
  // capture whatever is behind the window).
  public static RECT Frame(IntPtr h) { RECT r; if (DwmGetWindowAttribute(h, 9, out r, 16) != 0) GetWindowRect(h, out r); return r; }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(int f, int x, int y, int d, int e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, int f, int e);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int ht, bool r);
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr LoadKeyboardLayout(string id, uint f);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
  public static System.Collections.Generic.List<IntPtr> Windows(uint pid) {
    var list = new System.Collections.Generic.List<IntPtr>();
    EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p);
      var t = new System.Text.StringBuilder(256); GetWindowText(h, t, 256);
      if (p == pid && IsWindowVisible(h) && t.Length > 0) list.Add(h); return true; }, IntPtr.Zero);
    return list;
  }
  public struct RECT { public int L, T, R, B; }
}
"@
[W]::SetProcessDPIAware() | Out-Null
$out = "$env:TEMP\wtpp-test"
$sw = [Diagnostics.Stopwatch]::StartNew()
$env:WTPP_LOG_FILE = "$out\wtpp.log"
# A separate wt++ instance: otherwise this launch would hand its window to
# the user's running wt++.
$env:WTPP_INSTANCE = 'test'

# WTPP_EXE runs another build (e.g. while the installed one is in use).
$exe = if ($env:WTPP_EXE) { $env:WTPP_EXE } else { "$env:LOCALAPPDATA\Programs\wtpp\wtpp.exe" }
if ($ArgList) { $p = Start-Process $exe -ArgumentList $ArgList -WorkingDirectory $env:USERPROFILE -PassThru }
else { $p = Start-Process $exe -WorkingDirectory $env:USERPROFILE -PassThru }
while ($p.MainWindowHandle -eq 0 -and $sw.ElapsedMilliseconds -lt 10000) { Start-Sleep -Milliseconds 20; $p.Refresh() }
"window visible after $($sw.ElapsedMilliseconds) ms"
while ($p.MainWindowTitle -in @('', 'wt++') -and $sw.ElapsedMilliseconds -lt 15000) { Start-Sleep -Milliseconds 20; $p.Refresh() }
"shell tab ready after $($sw.ElapsedMilliseconds) ms (title: $($p.MainWindowTitle))"
# Run on the secondary monitor if there is one, keeping the main one free.
# (Once started: wt++ sizes its window while it starts up.)
$screen = [Windows.Forms.Screen]::AllScreens | Sort-Object Primary | Select-Object -First 1
if (-not $screen.Primary) {
  for ($try = 0; $try -lt 5; $try++) {
    $r = New-Object W+RECT; [W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    if ($r.L -ge $screen.Bounds.X) { break }
    [W]::MoveWindow($p.MainWindowHandle, $screen.Bounds.X + 40, 100, [Math]::Min($r.R - $r.L, 1800), [Math]::Min($r.B - $r.T, 900), $true) | Out-Null
    Start-Sleep -Milliseconds 300
  }
}
$shell = New-Object -ComObject WScript.Shell
function ShotWindow($h, $name) {
  $r = [W]::Frame($h)
  $bmp = New-Object Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
  $g = [Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $bmp.Save("$out\$name.png"); $g.Dispose(); $bmp.Dispose()
}
function Shot($name) { ShotWindow $p.MainWindowHandle $name }
Start-Sleep -Milliseconds 800
[W]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 300
$i = 0
foreach ($s in $Steps) {
  # Keys, clicks and screenshots go to whatever is in front. If that isn't
  # wt++ (the user is in another app, a game is fullscreen), stop rather
  # than type into it or capture it.
  if ($s -notlike 'sleep:*' -and $s -notin @('rect', 'windows', 'tile', 'launch', 'sup') -and $s -notlike 'shotall:*') {
    $fg = [W]::GetForegroundWindow(); [uint32]$fgPid = 0; [W]::GetWindowThreadProcessId($fg, [ref]$fgPid) | Out-Null
    if ($fgPid -ne $p.Id) {
      "ABORTED at '$s': another app is in the foreground"
      if (-not $p.HasExited) { Stop-Process -Id $p.Id }
      exit 2
    }
  }
  if ($s -like 'shot:*') { Shot $s.Substring(5) }
  elseif ($s -like 'click:*') {
    # click:x,y in window-relative physical pixels
    $xy = $s.Substring(6).Split(','); $r = New-Object W+RECT; [W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    [W]::SetCursorPos($r.L + [int]$xy[0], $r.T + [int]$xy[1]) | Out-Null; Start-Sleep -Milliseconds 80
    [W]::mouse_event(2,0,0,0,0); [W]::mouse_event(4,0,0,0,0); Start-Sleep -Milliseconds 500
  }
  elseif ($s -like 'rclick:*') {
    $xy = $s.Substring(7).Split(','); $r = New-Object W+RECT; [W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    [W]::SetCursorPos($r.L + [int]$xy[0], $r.T + [int]$xy[1]) | Out-Null; Start-Sleep -Milliseconds 80
    [W]::mouse_event(8,0,0,0,0); [W]::mouse_event(16,0,0,0,0); Start-Sleep -Milliseconds 600
  }
  elseif ($s -like 'drag:*') {
    # drag:x1,y1,x2,y2 with the left button, in small steps
    $v = $s.Substring(5).Split(',') | ForEach-Object { [int]$_ }; $r = New-Object W+RECT; [W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    [W]::SetCursorPos($r.L + $v[0], $r.T + $v[1]) | Out-Null; Start-Sleep -Milliseconds 80; [W]::mouse_event(2,0,0,0,0)
    for ($k = 1; $k -le 20; $k++) { [W]::SetCursorPos($r.L + $v[0] + ($v[2]-$v[0])*$k/20, $r.T + $v[1] + ($v[3]-$v[1])*$k/20) | Out-Null; Start-Sleep -Milliseconds 25 }
    [W]::mouse_event(4,0,0,0,0); Start-Sleep -Milliseconds 400
  }
  elseif ($s -like 'move:*') {
    $xy = $s.Substring(5).Split(','); $r = New-Object W+RECT; [W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    [W]::SetCursorPos($r.L + [int]$xy[0], $r.T + [int]$xy[1]) | Out-Null; Start-Sleep -Milliseconds 300
  }
  elseif ($s -like 'dclick:*') {
    $xy = $s.Substring(7).Split(','); $r = New-Object W+RECT; [W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    [W]::SetCursorPos($r.L + [int]$xy[0], $r.T + [int]$xy[1]) | Out-Null; Start-Sleep -Milliseconds 80
    [W]::mouse_event(2,0,0,0,0); [W]::mouse_event(4,0,0,0,0); Start-Sleep -Milliseconds 60
    [W]::mouse_event(2,0,0,0,0); [W]::mouse_event(4,0,0,0,0); Start-Sleep -Milliseconds 700
  }
  elseif ($s -like 'clip:*') {
    # clip:TEXT puts TEXT on the clipboard (\n becomes a newline)
    Set-Clipboard -Value ($s.Substring(5) -replace '\\n', "`n")
  }
  elseif ($s -eq 'tile') {
    # Put this process's windows side by side, most recent on the left.
    # On the secondary monitor if there is one (the user may be using the
    # main one): left edge + 40, then + 940.
    $screen = [Windows.Forms.Screen]::AllScreens | Sort-Object Primary | Select-Object -First 1
    $ws = [W]::Windows([uint32]$p.Id); $k = 0
    foreach ($h in $ws) { [W]::MoveWindow($h, $screen.Bounds.X + 40 + $k * 940, 100, 900, 520, $true) | Out-Null; $k++ }
    Start-Sleep -Milliseconds 600
  }
  elseif ($s -like 'sdrag:*') {
    # sdrag:x1,y1,x2,y2 drags with the left button in screen coordinates
    $v = $s.Substring(6).Split(',') | ForEach-Object { [int]$_ }
    [W]::SetCursorPos($v[0], $v[1]) | Out-Null; Start-Sleep -Milliseconds 80; [W]::mouse_event(2,0,0,0,0)
    for ($k = 1; $k -le 30; $k++) { [W]::SetCursorPos($v[0] + ($v[2]-$v[0])*$k/30, $v[1] + ($v[3]-$v[1])*$k/30) | Out-Null; Start-Sleep -Milliseconds 25 }
    Start-Sleep -Milliseconds 300; [W]::mouse_event(4,0,0,0,0); Start-Sleep -Milliseconds 1200
  }
  elseif ($s -like 'shotall:*') {
    # A screenshot of each of this process's windows.
    $k = 0
    foreach ($h in [W]::Windows([uint32]$p.Id)) { ShotWindow $h "$($s.Substring(8))-$k"; $k++ }
  }
  elseif ($s -like 'shold:*') {
    # shold:x1,y1,x2,y2 like sdrag, but keeps the button down (until 'sup')
    $v = $s.Substring(6).Split(',') | ForEach-Object { [int]$_ }
    [W]::SetCursorPos($v[0], $v[1]) | Out-Null; Start-Sleep -Milliseconds 80; [W]::mouse_event(2,0,0,0,0)
    for ($k = 1; $k -le 30; $k++) { [W]::SetCursorPos($v[0] + ($v[2]-$v[0])*$k/30, $v[1] + ($v[3]-$v[1])*$k/30) | Out-Null; Start-Sleep -Milliseconds 25 }
    Start-Sleep -Milliseconds 400
  }
  elseif ($s -eq 'sup') { [W]::mouse_event(4,0,0,0,0); Start-Sleep -Milliseconds 1200 }
  elseif ($s -eq 'launch') {
    # Start wt++ again: it should hand over to this instance and exit.
    $q = Start-Process $exe -WorkingDirectory $env:USERPROFILE -PassThru
    if ($q.WaitForExit(5000)) { "second launch exited (code $($q.ExitCode))" } else { "second launch still running"; Stop-Process -Id $q.Id }
    Start-Sleep -Milliseconds 1500
  }
  elseif ($s -like 'layout:*') {
    # layout:040D switches the window's keyboard layout (0409 = US English)
    $hkl = [W]::LoadKeyboardLayout("0000" + $s.Substring(7), 0)
    [W]::PostMessage($p.MainWindowHandle, 0x50, [IntPtr]::Zero, $hkl) | Out-Null; Start-Sleep -Milliseconds 400
  }
  elseif ($s -like 'chord:*') {
    # chord:11,10,57 presses virtual keys (hex) in order and releases them in
    # reverse: physical keys, whatever the layout (here Ctrl+Shift+W)
    $vks = $s.Substring(6).Split(',') | ForEach-Object { [Convert]::ToByte($_, 16) }
    foreach ($vk in $vks) { [W]::keybd_event($vk, 0, 0, 0) }
    [array]::Reverse($vks); foreach ($vk in $vks) { [W]::keybd_event($vk, 0, 2, 0) }
    Start-Sleep -Milliseconds 400
  }
  elseif ($s -like 'ctrl:*') {
    # ctrl:down / ctrl:up holds or releases Ctrl (for Ctrl+hover)
    $f = if ($s -eq 'ctrl:down') { 0 } else { 2 }; [W]::keybd_event(0x11, 0, $f, 0); Start-Sleep -Milliseconds 100
  }
  elseif ($s -like 'setrect:*') {
    $v = $s.Substring(8).Split(',') | ForEach-Object { [int]$_ }
    [W]::MoveWindow($p.MainWindowHandle, $v[0], $v[1], $v[2], $v[3], $true) | Out-Null; Start-Sleep -Milliseconds 400
  }
  elseif ($s -eq 'close') {
    # Close like the user would (Alt+F4) and wait for the process to end.
    $shell.SendKeys('%{F4}'); $p.WaitForExit(5000) | Out-Null
  }
  elseif ($s -eq 'windows') {
    # List this process's visible, titled windows.
    $ws = [W]::Windows([uint32]$p.Id)
    "windows $($ws.Count)"
    foreach ($h in $ws) {
      $t = New-Object System.Text.StringBuilder 256; [W]::GetWindowText($h, $t, 256) | Out-Null
      $r = New-Object W+RECT; [W]::GetWindowRect($h, [ref]$r) | Out-Null
      "  '$t' $($r.R - $r.L)x$($r.B - $r.T) at $($r.L),$($r.T)"
    }
  }
  elseif ($s -eq 'rect') {
    # Print the window size, to catch unwanted resizes.
    $r = New-Object W+RECT; [W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
    "rect $($r.R - $r.L)x$($r.B - $r.T) at $($r.L),$($r.T)"
  }
  elseif ($s -like 'sleep:*') { Start-Sleep -Milliseconds ([int]$s.Substring(6)) }
  else { $shell.SendKeys($s); Start-Sleep -Milliseconds 700 }
}
$p.Refresh()
if ($p.HasExited -and $Steps[-1] -eq 'close') { "closed" }
elseif ($p.HasExited) { "EXITED code $($p.ExitCode)" } else { "still running; tabs title: $($p.MainWindowTitle)"; Stop-Process -Id $p.Id }
