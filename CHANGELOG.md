# Changelog

## Unreleased

- Default terminal: Settings → Startup → "Make wt++ the default".
  - Windows: console programs started from the Start menu, Run or Explorer
    open in wt++ (styled with the matching profile, e.g. Command Prompt),
    through the same handoff Windows Terminal uses. Needs Windows Terminal
    installed (wt++ uses its console host); "Restore previous" undoes it.
  - Linux: sets the freedesktop terminal list (xdg-terminal-exec), GNOME /
    Cinnamon gsettings and KDE's terminal; shows the command for Debian's
    x-terminal-emulator.
  - macOS has no default-terminal setting.
- `wtpp -e COMMAND…` and `--working-directory DIR`, as Linux desktops call
  terminals.

## 0.2.0 — 2026-10-31

- Drag tabs between windows like a browser: a see-through preview follows
  the pointer, and a marker shows where the tab will land. Drop it outside
  any window for a new one; drag a window's only tab to move the window, or
  onto another window to merge them.
- All windows share one process: launching wt++ again opens a window in the
  running instance (so any two windows can exchange tabs).
- Drop files or folders onto a pane to paste their paths, quoted for its
  shell: `C:\...` for cmd and PowerShell, `/mnt/c/...` for WSL, `/c/...`
  for Git Bash.
- New windows open next to the current one, on the same monitor; torn-off
  windows stay clear of the taskbar.
- `wtpp --pos X,Y` opens the window at a screen position (as in `wt.exe`).
- Fixed: in a torn-off window the + button and dragging its tab didn't work
  until the window was resized.

## 0.1.0 — 2026-10-31

First stable release.

- Windows Terminal look: tabs in the title bar with native caption buttons and
  Snap Layouts, tab colors, renaming, drag to reorder, overflow scrolling.
- Panes: split, close, focus, resize (keys or dragging the divider), zoom, swap.
- Multiple windows; move a tab to a new window.
- GPU rendering (DX12 on Windows), ligatures, bundled Cascadia Mono/Code.
- Selection, mark mode, find (case/regex), links, OSC 52, bracketed paste,
  paste warnings, scrollback.
- Settings UI and WT-format `settings.json`: profiles, color schemes,
  appearance, keyboard shortcut editor, import from Windows Terminal,
  live reload.
- Shell discovery: PowerShell, cmd, WSL distros, Git Bash; `/etc/shells` on Unix.
- Ctrl+Backspace / Ctrl+Delete / Shift+Enter in every shell, including cmd.
- Shortcuts work on non-Latin keyboard layouts; defaults avoid OS hotkeys.
- Command line `wtpp [-d DIR] [-p PROFILE] [new-tab|split-pane …]`; "Open in
  wt++" in Explorer.

Known limitations: no background transparency/acrylic yet; tab progress
indicator (OSC 9;4) not shown; macOS untested; in cmd, the last echoed line
can be cut off when the shell exits.
