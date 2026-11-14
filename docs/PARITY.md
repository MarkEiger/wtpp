# Windows Terminal parity

`[x]` done and tested · `[ ]` open · `[~]` partial (note says what's missing)

## Bugs
- [x] Opening/closing tabs changes the window size (reported by user)
- [x] Colored inactive tabs had low-contrast text
- [x] Restored window came back one caption height taller
- [x] Default font silently fell back to Consolas (Cascadia isn't installed system-wide) — now bundled
- [x] Tab titles overflowed narrow tabs
- [x] Remembered window bounds weren't saved once multiple windows existed (event loop quit before the save)
- [ ] cmd: the last echoed line can be cut off when the shell exits (`exit 3` shows `e`) — ConPTY doesn't flush its final frame before the pipe closes

## Window & tabs
- [x] Tabs in the title bar, native caption buttons, Snap Layouts, drag/double-click caption
- [x] New tab button and profile dropdown with shortcuts
- [x] Rename (double-click / menu), duplicate, close, close others / to the right
- [x] Drag tabs to reorder
- [x] Close confirmation for multiple tabs
- [x] Tab color (context menu color picker, `tabColor` per profile)
- [x] Tabs shrink, then scroll (‹ › buttons) when they overflow the row
- [x] Multiple windows (`newWindow`, Ctrl+Shift+N), move tab to new window
- [x] One process for all windows (launching again opens a window in the running wt++)
- [x] Drag tabs between windows (drop marker), tear off into a new window, drag a lone tab to move/merge its window
- [x] New windows cascade from the current one, on its monitor
- [x] Unfocused window look (dimmed tab row)
- [~] Bell indicator on tab — done; progress indicator (OSC 9;4) **blocked**: alacritty_terminal's parser drops OSC 9 (needs a vendored patch)
- [x] Launch position (`initialPosition`) and window size/position persisted

## Panes
- [x] Split right/down/left/up/auto, duplicate profile, close, focus, resize, zoom
- [x] Drag the divider between panes with the mouse
- [x] Swap/move panes (`swapPane`)
- [x] Focus follows mouse (`focusFollowMouse`)

## Terminal
- [x] GPU rendering (DX12 on Windows), truecolor, bold/italic/underline/strikeout
- [x] Selection: drag, word, line, block (Alt), Shift-extend; copy on select
- [x] Copy/paste keys and right-click; bracketed paste
- [x] Scrollback, scrollbar, mouse wheel, mouse reporting for TUI apps
- [x] Find with match count
- [x] Ctrl+click links (text URLs and OSC 8)
- [x] Cursor shapes, blink; hollow cursor in unfocused panes
- [x] Font zoom (Ctrl+=/-/0, Ctrl+wheel)
- [x] Bell (system sound)
- [x] `closeOnExit` (graceful/always/never) and "[process exited with code N]"
- [x] Multi-line / large paste warning
- [x] Find: highlight all matches, case-sensitive and regex toggles
- [x] Underline links on Ctrl+hover, hand cursor
- [x] Terminal right-click menu option (`experimental.rightClickContextMenu`)
- [~] Background opacity and background image (`opacity`, `backgroundImage` incl. `desktopWallpaper`, image opacity, stretch mode, alignment) — done; acrylic open. Not see-through with OpenGL (WSLg), X11 without a compositor, or a Vulkan surface without a translucent alpha mode; on Windows always drawn through a DirectComposition swapchain, so it applies live
- [x] Font weight, line height, intense text style, ligatures (+ toggle); Cascadia Mono/Code bundled like WT
- [x] OSC 52 clipboard writes (setting-gated); visual bell
- [x] Mark mode / keyboard selection (Ctrl+Shift+M), select all (Ctrl+Shift+A)
- [x] Drag and drop files/folders onto a pane pastes their quoted paths (`/mnt/c/...` in WSL)

## Keyboard
- [x] Ctrl+Backspace, Ctrl+Delete (incl. cmd), Shift+Enter in every shell
- [x] Shortcuts and Ctrl+letter work on non-Latin keyboard layouts (Russian, Hebrew, …) by physical key, like WT
- [x] Defaults avoid OS hotkeys: panes on Ctrl+Shift+Plus/Minus and Ctrl+Shift+arrows (WT's Alt+Shift is the Windows layout switch), tabs on Alt+1..9 (Ctrl+Alt is AltGr)
- [x] win32-input-mode for cmd/PowerShell, kitty protocol for TUI apps
- [x] All WT default bindings; editable in Settings; sendInput

## Settings
- [x] Settings UI (Startup, Interaction, Appearance, Color schemes, Actions, profiles)
- [x] settings.json in WT format, live reload, Open JSON, import from WT
- [x] Profiles: name, command line, folder, hidden, scheme, font, size, cursor, padding, history, key encoding
- [x] Profile icon (dropdown + tab): program icons, image files, emoji
- [x] Profile: tab title, suppress app title, environment, bell style, close on exit, opacity, background image
- [x] Appearance: theme (dark/light/system), tab width mode (equal/title/compact), always show tabs
- [x] Rendering page (graphics API incl. software rendering)

## Platform & integration
- [x] Native Windows build + Start Menu shortcut; Linux build
- [x] Command line: `wtpp [-d DIR] [-p PROFILE] [new-tab|split-pane …]`
- [x] "Open in wt++" in Explorer's folder context menu
- [x] Default terminal: Windows (console handoff, like WT; needs WT's OpenConsole), Linux (xdg-terminals.list, gsettings, KDE); macOS has no such setting
- [x] macOS (Apple Silicon): builds, runs, `wt++.app`; ⌘ shortcuts, tabs in the title bar
- [x] Linux: tested on Ubuntu 26.04 GNOME (Wayland and X11); app-menu install
