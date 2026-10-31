# Changelog

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
