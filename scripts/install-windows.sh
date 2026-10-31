#!/usr/bin/env bash
# Build the native Windows wt++ from WSL and install it for the current
# Windows user: %LOCALAPPDATA%\Programs\wtpp\wtpp.exe + a Start Menu shortcut.
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
target=x86_64-pc-windows-gnu

cargo build --release --target "$target"

win_env() { cmd.exe /c "echo %$1%" 2>/dev/null | tr -d '\r'; }
install_dir_win="$(win_env LOCALAPPDATA)\\Programs\\wtpp"
install_dir="$(wslpath "$install_dir_win")"
mkdir -p "$install_dir"
cp "target/$target/release/wtpp.exe" "$install_dir/wtpp.exe"
cp assets/wtpp.ico "$install_dir/wtpp.ico"

powershell.exe -NoProfile -Command "
  \$lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'wt++.lnk'
  \$s = (New-Object -ComObject WScript.Shell).CreateShortcut(\$lnk)
  \$s.TargetPath = '$install_dir_win\\wtpp.exe'
  \$s.WorkingDirectory = [Environment]::GetFolderPath('UserProfile')
  \$s.IconLocation = '$install_dir_win\\wtpp.ico'
  \$s.Description = 'wt++ terminal'
  \$s.Save()
  Write-Output \"Start Menu shortcut: \$lnk\"
"
echo "Installed to $install_dir_win\\wtpp.exe"

# "Open in wt++" in Explorer's folder and folder-background right-click menus
# (Windows 11: under "Show more options").
exe_win="$install_dir_win\\wtpp.exe"
for key in 'HKCU\Software\Classes\Directory\Background\shell\wtpp' 'HKCU\Software\Classes\Directory\shell\wtpp'; do
    reg.exe add "$key" /ve /d "Open in wt++" /f >/dev/null
    reg.exe add "$key" /v Icon /d "$install_dir_win\\wtpp.ico" /f >/dev/null
    reg.exe add "$key\\command" /ve /d "\"$exe_win\" -d \"%V\"" /f >/dev/null
done
echo "Added \"Open in wt++\" to Explorer's context menu"
