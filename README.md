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

**Linux**: unpack `wtpp-<version>-linux-x64.tar.gz` and run `wtpp`.

## Highlights

- Tabs in the title bar, panes, multiple windows, command palette, find,
  mark mode — see [docs/PARITY.md](docs/PARITY.md) for the full list.
- A settings UI like WT's; settings live in WT-format `settings.json`
  (`%LOCALAPPDATA%\wtpp` on Windows, `~/.config/wtpp` on Linux), reload live,
  and can be imported from Windows Terminal.
- Ctrl+Backspace, Ctrl+Delete and Shift+Enter work in every shell, cmd included.
- Shortcuts work on any keyboard layout, and the defaults avoid OS hotkeys.

## Keyboard shortcuts

All shortcuts are editable in **⌄ → Keyboard shortcuts**.

| Keys | Action |
|---|---|
| Ctrl+Shift+T / Ctrl+Shift+1..9 | New tab (default / profile N) |
| Ctrl+Shift+N | New window |
| Ctrl+Shift+W | Close pane |
| Ctrl+Tab, Ctrl+Shift+Tab, Alt+1..9 | Switch tabs |
| Ctrl+Shift+Plus / Ctrl+Shift+Minus | Split pane right / down |
| Alt+Arrows | Move between panes |
| Ctrl+Shift+Arrows | Resize pane |
| Ctrl+Shift+P | Command palette |
| Ctrl+Shift+F | Find |
| Ctrl+Shift+M / Ctrl+Shift+A | Mark mode / select all |
| Ctrl+, | Settings |
| Ctrl+= / Ctrl+- / Ctrl+0 | Font size |
| Alt+Enter, F11 | Full screen |

They differ from Windows Terminal's in two places: WT's Alt+Shift pane keys
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
