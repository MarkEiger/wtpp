# wt++

A GPU-accelerated terminal with Windows Terminal's look, settings and
`settings.json` format, for Windows, Linux and macOS. It runs any shell: cmd,
PowerShell, WSL, bash, zsh, fish, …

## Install

**Windows** (native, no WSL needed): unzip `wtpp-<version>-windows-x64.zip`
and run

```powershell
powershell -ExecutionPolicy Bypass -File install.ps1
```

This installs to `%LOCALAPPDATA%\Programs\wtpp`, adds wt++ to the Start Menu
and "Open in wt++" to Explorer's folder right-click menu.

**macOS** (Apple Silicon): unzip `wtpp-<version>-macos-arm64.zip` and move
`wt++.app` to Applications. It isn't notarized by Apple, so macOS blocks the
first launch: either run

```sh
xattr -dr com.apple.quarantine /Applications/wt++.app
```

or try to open it once, then click **Open Anyway** in System Settings →
Privacy & Security.

**Linux** (x86-64, glibc 2.39+: Ubuntu 24.04, Debian 13, Fedora 40 or newer):
unpack `wtpp-<version>-linux-x64.tar.gz` and run `sh install.sh`: it installs
`~/.local/bin/wtpp` and adds wt++ to the app menu. On Wayland desktops wt++
runs through XWayland, which it needs for dropping files and dragging tabs
between windows; `WTPP_WAYLAND=1` runs it as a Wayland app without those.

## Highlights

- Tabs in the title bar, panes, multiple windows, command palette, find,
  mark mode — see [docs/PARITY.md](docs/PARITY.md) for the full list.
- A settings UI like WT's; settings live in WT-format `settings.json`
  (`%LOCALAPPDATA%\wtpp` on Windows, `~/.config/wtpp` on Linux), reload live,
  and can be imported from Windows Terminal.
- Drag tabs between windows (or out into a new one), like a browser.
- Drop files or folders onto the terminal to paste their paths, in the right
  form for the shell (cmd, PowerShell, WSL, Git Bash).
- Ctrl+Backspace, Ctrl+Delete and Shift+Enter work in every shell, cmd included.
- Shortcuts work on any keyboard layout, and the defaults avoid OS hotkeys.

## Default terminal

Settings → Startup → **Make wt++ the default**:

- **Windows**: console programs started outside a terminal (Start menu, Run,
  Explorer) open in wt++, like with Windows Terminal. Windows Terminal must be
  installed: wt++ uses its console host for the handoff. Elevated (admin)
  programs keep the classic console.
- **Linux**: wt++ becomes the desktop's terminal (freedesktop
  `xdg-terminals.list`, GNOME/Cinnamon, KDE); it accepts `-e COMMAND` and
  `--working-directory DIR`.
- **macOS** has no default-terminal setting.

## Keyboard shortcuts

All shortcuts are editable in **⌄ → Keyboard shortcuts**.

| Windows / Linux | macOS | Action |
|---|---|---|
| Ctrl+Shift+T / Ctrl+Shift+1..9 | ⌘T / ⇧⌘1..9 | New tab (default / profile N) |
| Ctrl+Shift+N | ⌘N | New window |
| Ctrl+Shift+W | ⌘W | Close pane |
| Ctrl+Tab, Ctrl+Shift+Tab, Alt+1..9 | ⌃Tab, ⇧⌘] / ⇧⌘[, ⌘1..9 | Switch tabs |
| Ctrl+Shift+Plus / Ctrl+Shift+Minus | ⌘D / ⇧⌘D | Split pane right / down |
| Alt+Arrows | ⌥⌘Arrows | Move between panes |
| Ctrl+Shift+Arrows | ⌃⌘Arrows | Resize pane |
| Ctrl+Shift+P | ⇧⌘P | Command palette |
| Ctrl+Shift+F | ⌘F | Find |
| Ctrl+Shift+M / Ctrl+Shift+A | ⇧⌘M / ⌘A | Mark mode / select all |
| Ctrl+Shift+C / Ctrl+Shift+V | ⌘C / ⌘V | Copy / paste |
| Ctrl+, | ⌘, | Settings |
| Ctrl+= / Ctrl+- / Ctrl+0 | ⌘= / ⌘- / ⌘0 | Font size |
| Alt+Enter, F11 | ⌃⌘F | Full screen |

On macOS, Ctrl always goes to the shell (Ctrl+C interrupts) and Option types
characters (⌥A → å), as in Terminal.

On Windows and Linux they differ from Windows Terminal's in two places: WT's Alt+Shift pane keys
are Windows' keyboard-layout switch, and its Ctrl+Alt+N tab keys are AltGr on
many layouts.

## Building

Rust (stable) workspace; `crates/wtpp-app` is the binary.

```sh
cargo build --release                     # Linux
scripts/install-windows.sh                # from WSL: native Windows build + install
scripts/package-release.sh                # release packages into dist/
```

The Windows build cross-compiles with MinGW (`x86_64-pc-windows-gnu`).
`vendor/i-slint-core` is a patched copy of Slint's core (see
[vendor/README.md](vendor/README.md)).

## License

wt++ is free software under the [GNU General Public License v3.0 or later](LICENSE).

It's built on [Slint](https://slint.dev) (used under its GPLv3 option;
`vendor/i-slint-core` is a patched copy), [alacritty_terminal](https://github.com/alacritty/alacritty),
[wgpu](https://wgpu.rs) and [cosmic-text](https://github.com/pop-os/cosmic-text).
It bundles the [Cascadia Code](https://github.com/microsoft/cascadia-code)
fonts (SIL Open Font License, see `assets/fonts/LICENSE-Cascadia.txt`).
