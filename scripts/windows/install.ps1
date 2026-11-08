# Install wt++ for the current user from an unpacked release zip:
#   %LOCALAPPDATA%\Programs\wtpp\wtpp.exe, a Start Menu shortcut, and
#   "Open in wt++" in Explorer's folder right-click menu.
# Run:  powershell -ExecutionPolicy Bypass -File install.ps1
$ErrorActionPreference = 'Stop'
$src = $PSScriptRoot
$dir = Join-Path $env:LOCALAPPDATA 'Programs\wtpp'

Get-Process wtpp -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dir\*" } | ForEach-Object {
    Write-Host 'Close wt++ first, then run this again.'; exit 1
}
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item (Join-Path $src 'wtpp.exe'), (Join-Path $src 'wtpp.ico') $dir -Force

$lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'wt++.lnk'
$s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
$s.TargetPath = "$dir\wtpp.exe"
$s.WorkingDirectory = [Environment]::GetFolderPath('UserProfile')
$s.IconLocation = "$dir\wtpp.ico"
$s.Description = 'wt++ terminal'
$s.Save()

foreach ($key in 'HKCU:\Software\Classes\Directory\Background\shell\wtpp', 'HKCU:\Software\Classes\Directory\shell\wtpp') {
    New-Item -Force "$key\command" | Out-Null
    Set-ItemProperty $key '(default)' 'Open in wt++'
    Set-ItemProperty $key 'Icon' "$dir\wtpp.ico"
    Set-ItemProperty "$key\command" '(default)' "`"$dir\wtpp.exe`" -d `"%V`""
}
# On the user's PATH, so `wtpp --version` (and `wtpp -d DIR`) work in new
# shells. Read the raw value so %VARIABLES% in it stay unexpanded.
$envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
$path = [string]$envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$parts = @($path -split ';' | Where-Object { $_ })
if ($parts -notcontains $dir) {
    $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
    if ($path) { $kind = $envKey.GetValueKind('Path') }
    $envKey.SetValue('Path', (($parts + $dir) -join ';'), $kind)
    # Tell Explorer, so shells started from now on see it.
    [Environment]::SetEnvironmentVariable('WTPP_INSTALL_REFRESH', '1', 'User')
    [Environment]::SetEnvironmentVariable('WTPP_INSTALL_REFRESH', $null, 'User')
}
$envKey.Close()
Write-Host "Installed to $dir. Find wt++ in the Start Menu."
Write-Host "In a new terminal, 'wtpp --version' shows the installed version."
