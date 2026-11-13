# Changelog

## Unreleased

- A modifier wt++ thought was held but the keyboard says is up (a missed
  Shift release) no longer turns Enter into Shift+Enter, which adds lines in
  Claude Code and PowerShell. When this happens it's noted in the log, to
  help track down reports of Enter staying a new line after a Shift+Enter.

## 1.4.0 — 2026-11-13

- Linux and macOS: a profile's command line is read like a shell does (single
  quotes, backslashes), so `sh -c 'echo hello; exit 3'` runs; it failed with
  "Unterminated quoted string". Windows reads it as before.
- The "Paste anyway?" warning for multi-line or large pastes (and "Close all
  tabs?") now has the keyboard while it's open: Enter pastes, Esc cancels,
  Tab moves between its buttons (Enter then presses the one with the focus),
  and every other key, shortcuts included, does nothing until you answer it.
  Typing no longer goes to the shell behind it, and clicks there do nothing.
  Both paste warnings can be turned off in Settings → Interaction
  (`multiLinePasteWarning`, `largePasteWarning`): then pasting is immediate.
- A profile whose program can't be started (from the new-tab menu, a
  shortcut, a split) opens its tab anyway and says why, e.g.
  `[couldn't start "no-such.exe": The system cannot find the file
  specified.]`; Enter tries again, Ctrl+D closes it. Before, nothing
  happened.
- With no GPU that works (a VM, a remote session, a broken driver), wt++ uses
  its software renderer instead of exiting without a window. If it can't draw
  at all, Windows shows a message saying so and what to set.
- A program that prints and exits at once no longer loses the end of what it
  printed (`wsl.exe -d Nope` showed only "T", `exit 3` in cmd only "e").
- Windows 10: Ctrl+Backspace in Command Prompt right after very fast typing
  deletes the whole word; it waits a moment for the typing to show.
- Windows 10 1809: Shift+Enter adds a line in Claude Code; it submitted the
  prompt and left Alt stuck for the next key.
- Import from Windows Terminal adds its key bindings to yours instead of
  replacing them: your own bindings stay, also when WT has none.
- Settings: after an edit, the note at the bottom from before ("No
  changes.", "Saved …") goes away. The settings.json error note is shorter
  and the bar grows to fit it.
- Linux and macOS: a pane whose shell exited shows its exit code without the
  Windows-style hex form.

- Splitting a pane or duplicating a tab that's in ssh opens the new one in
  ssh to the same place. In bash and zsh (WSL, Linux, macOS) it shares the
  first one's connection: no new login or password. For that, wt++ gives
  them an `ssh` function that turns on OpenSSH's connection sharing where
  your ssh configuration has none. Windows' own ssh.exe can't share, so
  there you log in again. Settings → Interaction → New panes from an ssh
  session connect to the same place turns it off. See the README.

## 1.3.2 — 2026-11-12

- Linux and macOS: a pane whose starting directory doesn't exist (any more)
  opens in your home folder, as on Windows, not in the directory wt++ itself
  was started from.
- A split or duplicated tab shows the directory by the same name as the pane
  it came from: `/bin` stays `/bin` (not `/usr/bin`), in WSL too, and on macOS
  `/etc` stays `/etc` (not `/private/etc`). For that, bash (by
  `PROMPT_COMMAND`, as in WSL) and zsh (by startup files of wt++'s own, which
  run yours as before) now report their directory on Linux and macOS too.
  Settings → Interaction → New panes start in the current directory turns
  this off.

## 1.3.1 — 2026-11-11

- Windows: every key played the error sound in a window opened for a console
  program (wt++ as the default terminal, such as Win+R → `cmd`) after a recent
  Windows update: the window was in front without the keyboard focus. wt++
  now takes the focus.
- WSL: a pane split from a directory that is a symbolic link in Linux (`/bin`)
  opened in the home folder. The directory bash reports is now the real one.

## 1.3.0 — 2026-11-11

- Windows: new tabs and panes get the environment as it's set now. After you
  change PATH or another variable (in System Properties, or an installer
  does), new shells have it without restarting wt++, like in Windows Terminal
  (same setting: `compatibility.reloadEnvironmentVariables`, Settings →
  Interaction). Variables only wt++ was started with (by a launcher) are kept;
  PATH is the one Windows has, so to keep a developer prompt's PATH, start
  wt++ from it with the setting off. A program in a profile's command line is
  found in the new PATH too. Consoles handed to wt++ as the default terminal
  keep their own environment.
- A program that keeps adding keyboard modes without removing them no longer
  crashes the pane after 4096 of them.
- A new pane opens in the directory you're in also when its name has a `%`
  or starts with `~` (it was expanded like a setting). Split from a directory
  that's been deleted, it opens where its profile starts, not in the directory
  the pane before it started in.
- Linux: settings.json no longer reloads several times a second for as long
  as wt++ runs (each read of it counted as a change), and an error in it is
  logged once per change.

## 1.2.1 — 2026-11-10

- Splitting a pane or duplicating a tab opens the new one in the directory
  you're in, not where the profile starts. That works without setup for
  Command Prompt, PowerShell, bash in WSL, and shells on Linux and macOS;
  other shells in WSL need one line (see the README). A Windows drive carries
  over between WSL and Windows shells (`/mnt/c/src` ↔ `C:\src`); a directory
  that's only in Linux, to the same distro. It can be turned off in Settings →
  Interaction.
- An OSC 7 or OSC 9;9 (Windows Terminal's) current-directory report from any
  shell is understood.

## 1.2.0 — 2026-11-09

- Windows 10: Enter, Backspace, arrows, Tab and Ctrl+C did nothing in Command
  Prompt and PowerShell, because the console that comes with Windows 10 ignores
  the key events wt++ sent it. wt++ now sends those only to a console that asks
  for them (Windows 11 does, and works as before), and otherwise the keys'
  ordinary sequences. There, Ctrl+Backspace and Ctrl+Delete delete a word at
  the prompt, and Shift+Enter acts as Alt+Enter, which adds a line in Claude
  Code.
  Known limitations: on Windows 10 1809, Shift+Enter doesn't add a line in
  Claude Code. In Command Prompt, Ctrl+Backspace right after very fast typing
  can delete too little, as it goes by what's on screen so far.
- A WSL profile's starting directory is used; the Linux home won out over it.
  A path inside Linux (`/home/me`) opens the Linux home instead, as wsl.exe
  can't start there on all versions; before, such a profile opened no tab, or
  the wrong folder.
- A starting directory that doesn't exist no longer means no tab: it opens in
  your home folder.
- Windows: on some Windows 10 / WSL1 setups, WSL's usage text or an error
  message turned into one junk profile per line. wt++ now reads the list of
  distros from the registry, like Windows Terminal, and only accepts
  `wsl.exe -l -q` output that really is a list. Existing junk profiles are
  removed once, on the first launch of this version.
- wt++ looks for installed shells once, not on every launch: it no longer runs
  `wsl.exe` each time, and a shell profile you delete stays deleted (it could
  not be deleted before).
- Settings → Defaults → **Refresh shell list** finds shells again: use it after
  installing a new shell or WSL distro, or to repair the list. It adds what's
  new and removes what's gone, and says what changed; your changes to the
  shells' profiles, and profiles you made yourself, stay as they are.
- A `settings.json` with an error is no longer overwritten on startup. wt++
  opens with default settings and leaves the file alone. Saving from Settings
  replaces it only after keeping a copy as `settings.json.bak`.
- With no profile left to show (an empty list, or all hidden), wt++ opens the
  system's shell instead of closing, and also if the default profile's command
  can't be started.
- A hung `wsl.exe` no longer stays behind when wt++ closes during a refresh.
- WSL profiles on Windows 10's built-in WSL opened with only
  `[process exited with code -1]`. They now start with `wsl.exe ~ -d <distro>`
  instead of `--cd ~` (just `-d <distro>` before version 1903). Profiles wt++
  created are updated once; a command line you changed stays as it is.

## 1.1.2 — 2026-11-08

- Ctrl+Alt+Plus / Minus split a pane right / down with the default profile.
- Windows: Shift+Enter inserts a new line in Claude Code running in a
  PowerShell or Command Prompt tab; it submitted the prompt. Outside Claude
  Code, PowerShell still gets its own Shift+Enter.
- A tab dragged out of a maximized or fullscreen window opens at the default
  window size, not filling the screen again. A lone tab dragged out of
  fullscreen leaves fullscreen, as it already did for a maximized window.

## 1.1.1 — 2026-11-08

- Splitting a pane (Ctrl+Shift+Plus / Minus, ⌘D / ⇧⌘D) opens the same profile
  as the pane you split, like the right-click menu, instead of the default
  profile. "Split pane with default profile" is in the pane and tab right-click
  menus and the command palette, and can be given a shortcut.
- `wtpp --version` (or `-v`) prints the version. On Windows, `install.ps1` now
  puts wt++ on your PATH, so it works in any new terminal.
- macOS: ⌘← / ⌘→ go to the start / end of the line, ⌘⌫ deletes to the start
  of the line and ⌘⌦ to its end, as in iTerm; ⌘↑ / ⌘↓ scroll by a line.
  Before, these keys did nothing. They can be changed in Keyboard shortcuts.
- macOS: ⌥⌦ (⌥fn⌫) deletes the next word, as in Terminal; it printed
  `[3;3~` instead.

## 1.1.0 — 2026-11-01

- Linux and macOS: dropped files paste into the pane under the pointer
  (before, the focused pane), like on Windows.
- Linux and macOS: a tab dragged out of a window shows the same see-through
  preview of the window as on Windows. (Not on Wayland, which can't place
  windows at the pointer.)

## 1.0.0 — 2026-10-31

First public release, tested on Windows 11, Ubuntu 26.04 (GNOME, Wayland and
X11) and macOS 27 (Apple Silicon). Licensed GPL-3.0-or-later.

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
- macOS: a proper `wt++.app` (icon, ad-hoc signed); tabs in the title bar
  next to the window buttons (double-click to zoom); ⌘ shortcuts like
  Terminal and iTerm (⌘T, ⌘W, ⌘D, ⌘M…), Ctrl always reaches the shell,
  Option types characters and composes dead keys (⌥E E → é), ⌥←/⌥→ move by
  word.
- Linux: `install.sh` adds wt++ to the app menu with its icon; windows carry
  the `wtpp` app id so docks show the right icon.
- Linux: tabs in the title bar like Windows (wt++ draws the title bar, with
  minimize, maximize and close; drag the window by it, double-click to
  maximize, resize from the edges). Turn off "Show tabs in title bar" for the
  desktop's own title bar.
- Linux on Wayland desktops runs through XWayland, so files can be dropped
  onto panes and tabs dragged between windows (Wayland supports neither for
  wt++ yet). Set `WTPP_WAYLAND=1` to run as a Wayland app anyway.
- Heavy output (`cat` of a huge file) no longer delays typing: output is
  drawn at most once per frame. Ctrl+C under a flood used to take seconds on
  X11.
- Mouse wheel: 3 lines per notch on every font size, and mouse-mode apps
  (vim, htop) are told the cell under the pointer.
- Settings: shortcuts like Ctrl+Shift+W work after using a dropdown, and
  each page opens at its top.
- Linux default terminal: "Restore previous" puts back exactly what was
  there before (gsettings, KDE) and leaves an installed `wtpp.desktop` as it
  was.
- Tabs can be torn off by dragging straight down (before, only a sideways
  drag started dragging a tab).
- Escape with a selection clears the selection instead of reaching the shell.
- Default terminal opens handed-over consoles in front of other windows.

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
