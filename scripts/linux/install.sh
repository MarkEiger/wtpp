#!/usr/bin/env sh
# Install wt++ for the current user from an unpacked release (or a build):
#   ~/.local/bin/wtpp, an app-menu entry and its icon.
# Run:  sh install.sh
set -eu

here="$(cd "$(dirname "$0")" && pwd)"
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

src="$here/wtpp"
[ -x "$src" ] || src="$here/../../target/release/wtpp"
mkdir -p "$bin" "$data/applications" "$data/icons/hicolor/256x256/apps"
install -m 755 "$src" "$bin/wtpp"
icon="$here/wtpp.png"
[ -f "$icon" ] || icon="$here/../../assets/wtpp.png"
install -m 644 "$icon" "$data/icons/hicolor/256x256/apps/wtpp.png"

cat > "$data/applications/wtpp.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=wt++
GenericName=Terminal
Comment=GPU-accelerated terminal with tabs and panes
Exec=$bin/wtpp
TryExec=$bin/wtpp
Icon=wtpp
Terminal=false
Categories=System;TerminalEmulator;
Keywords=terminal;shell;console;command;prompt;
StartupWMClass=wtpp
X-TerminalArgExec=-e
X-TerminalArgDir=--working-directory
EOF
update-desktop-database "$data/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$data/icons/hicolor" 2>/dev/null || true
echo "Installed $bin/wtpp; wt++ is in your app menu."
case ":$PATH:" in *":$bin:"*) ;; *) echo "(Add $bin to your PATH to run wtpp from a shell.)";; esac
