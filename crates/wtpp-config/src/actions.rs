//! Actions, key chords, and their Windows Terminal JSON representation.

use serde_json::{Value, json};
use wtpp_input::{Key, Mods};

/// How a split arranges its two children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Axis {
    /// Side by side (a vertical divider).
    Horizontal,
    /// Stacked (a horizontal divider).
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Horizontal,
            Direction::Up | Direction::Down => Axis::Vertical,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }

    fn parse(s: &str) -> Option<Direction> {
        Some(match s {
            "left" => Direction::Left,
            "right" => Direction::Right,
            "up" => Direction::Up,
            "down" => Direction::Down,
            _ => return None,
        })
    }
}

/// Where a new split pane goes; `None` (auto) splits the longer side.
pub type SplitSide = Option<Direction>;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// `newTab`; `None` opens the default profile.
    NewTab(Option<usize>),
    DuplicateTab,
    CloseTab,
    CloseOtherTabs,
    ClosePane,
    NextTab,
    PrevTab,
    SwitchToTab(usize),
    SplitPane { side: SplitSide, duplicate: bool },
    MoveFocus(Direction),
    ResizePane(Direction),
    /// Swap the focused pane with its neighbor in this direction.
    SwapPane(Direction),
    TogglePaneZoom,
    Copy,
    Paste,
    ScrollUp,
    ScrollDown,
    ScrollUpPage,
    ScrollDownPage,
    ScrollToTop,
    ScrollToBottom,
    AdjustFontSize(i32),
    ResetFontSize,
    ToggleFullscreen,
    OpenNewTabDropdown,
    OpenSettings,
    NewWindow,
    CommandPalette,
    Find,
    RenameTab,
    /// Keyboard selection (WT's `markMode`).
    MarkMode,
    SelectAll,
    /// `sendInput`: write this text to the shell.
    SendInput(String),
}

impl Action {
    /// WT JSON for this action: a bare string, or an object with arguments.
    pub fn to_json(&self) -> Value {
        use Action::*;
        match self {
            NewTab(None) => json!("newTab"),
            NewTab(Some(i)) => json!({ "action": "newTab", "index": i }),
            DuplicateTab => json!("duplicateTab"),
            CloseTab => json!("closeTab"),
            CloseOtherTabs => json!("closeOtherTabs"),
            ClosePane => json!("closePane"),
            NextTab => json!("nextTab"),
            PrevTab => json!("prevTab"),
            SwitchToTab(i) => json!({ "action": "switchToTab", "index": i }),
            SplitPane { side, duplicate } => {
                let mut v = json!({ "action": "splitPane", "split": side.map_or("auto", Direction::name) });
                if *duplicate {
                    v["splitMode"] = json!("duplicate");
                }
                v
            }
            MoveFocus(d) => json!({ "action": "moveFocus", "direction": d.name() }),
            ResizePane(d) => json!({ "action": "resizePane", "direction": d.name() }),
            SwapPane(d) => json!({ "action": "swapPane", "direction": d.name() }),
            TogglePaneZoom => json!("togglePaneZoom"),
            Copy => json!({ "action": "copy", "singleLine": false }),
            Paste => json!("paste"),
            ScrollUp => json!("scrollUp"),
            ScrollDown => json!("scrollDown"),
            ScrollUpPage => json!("scrollUpPage"),
            ScrollDownPage => json!("scrollDownPage"),
            ScrollToTop => json!("scrollToTop"),
            ScrollToBottom => json!("scrollToBottom"),
            AdjustFontSize(d) => json!({ "action": "adjustFontSize", "delta": d }),
            ResetFontSize => json!("resetFontSize"),
            ToggleFullscreen => json!("toggleFullscreen"),
            OpenNewTabDropdown => json!("openNewTabDropdown"),
            OpenSettings => json!("openSettings"),
            NewWindow => json!("newWindow"),
            CommandPalette => json!("commandPalette"),
            Find => json!("find"),
            RenameTab => json!("openTabRenamer"),
            MarkMode => json!("markMode"),
            SelectAll => json!("selectAll"),
            SendInput(text) => json!({ "action": "sendInput", "input": text }),
        }
    }

    /// Parse WT JSON. `Ok(None)` is `"unbound"`; `Err` is an action we don't
    /// support (kept in the file, ignored at runtime).
    pub fn from_json(v: &Value) -> Result<Option<Action>, String> {
        use Action::*;
        let (name, args) = match v {
            Value::String(s) => (s.as_str(), &Value::Null),
            Value::Object(o) => (o.get("action").and_then(Value::as_str).ok_or("missing \"action\"")?, v),
            _ => return Err("expected a string or object".into()),
        };
        let index = || args.get("index").and_then(Value::as_u64).map(|i| i as usize);
        let direction = || {
            args.get("direction").and_then(Value::as_str).and_then(Direction::parse).ok_or("bad \"direction\"")
        };
        Ok(Some(match name {
            "unbound" => return Ok(None),
            "newTab" => NewTab(index()),
            "duplicateTab" => DuplicateTab,
            "closeTab" => CloseTab,
            "closeOtherTabs" => CloseOtherTabs,
            "closePane" => ClosePane,
            "nextTab" => NextTab,
            "prevTab" => PrevTab,
            "switchToTab" => SwitchToTab(index().unwrap_or(0)),
            "splitPane" => {
                let split = args.get("split").and_then(Value::as_str).unwrap_or("auto");
                let side = match split {
                    "auto" => None,
                    // WT's older names: "vertical" puts the new pane to the right.
                    "vertical" => Some(Direction::Right),
                    "horizontal" => Some(Direction::Down),
                    other => Some(Direction::parse(other).ok_or("bad \"split\"")?),
                };
                let duplicate = args.get("splitMode").and_then(Value::as_str) == Some("duplicate");
                SplitPane { side, duplicate }
            }
            "moveFocus" => MoveFocus(direction()?),
            "resizePane" => ResizePane(direction()?),
            "swapPane" => SwapPane(direction()?),
            "togglePaneZoom" => TogglePaneZoom,
            "copy" => Copy,
            "paste" => Paste,
            "scrollUp" => ScrollUp,
            "scrollDown" => ScrollDown,
            "scrollUpPage" => ScrollUpPage,
            "scrollDownPage" => ScrollDownPage,
            "scrollToTop" => ScrollToTop,
            "scrollToBottom" => ScrollToBottom,
            "adjustFontSize" => AdjustFontSize(args.get("delta").and_then(Value::as_f64).unwrap_or(1.0) as i32),
            "resetFontSize" => ResetFontSize,
            "toggleFullscreen" => ToggleFullscreen,
            "openNewTabDropdown" => OpenNewTabDropdown,
            "openSettings" => OpenSettings,
            "newWindow" => NewWindow,
            "commandPalette" | "toggleCommandPalette" => CommandPalette,
            "find" => Find,
            "openTabRenamer" => RenameTab,
            "markMode" => MarkMode,
            "selectAll" => SelectAll,
            "sendInput" => SendInput(args.get("input").and_then(Value::as_str).unwrap_or_default().to_string()),
            other => return Err(format!("unsupported action {other:?}")),
        }))
    }

    /// Human-readable name, like WT's command palette.
    pub fn label(&self, profile_names: &[String]) -> String {
        use Action::*;
        match self {
            NewTab(None) => "New tab".into(),
            NewTab(Some(i)) => match profile_names.get(*i) {
                Some(name) => format!("New tab: {name}"),
                None => format!("New tab, profile #{}", i + 1),
            },
            DuplicateTab => "Duplicate tab".into(),
            CloseTab => "Close tab".into(),
            CloseOtherTabs => "Close other tabs".into(),
            ClosePane => "Close pane".into(),
            NextTab => "Next tab".into(),
            PrevTab => "Previous tab".into(),
            SwitchToTab(i) => format!("Switch to tab {}", i + 1),
            SplitPane { side, duplicate } => {
                let side = match side {
                    None => "auto",
                    Some(Direction::Right) => "right",
                    Some(Direction::Down) => "down",
                    Some(Direction::Left) => "left",
                    Some(Direction::Up) => "up",
                };
                if *duplicate {
                    format!("Split pane ({side}), duplicate profile")
                } else {
                    format!("Split pane ({side})")
                }
            }
            MoveFocus(d) => format!("Move focus {}", d.name()),
            ResizePane(d) => format!("Resize pane {}", d.name()),
            SwapPane(d) => format!("Swap pane {}", d.name()),
            TogglePaneZoom => "Toggle pane zoom".into(),
            Copy => "Copy".into(),
            Paste => "Paste".into(),
            ScrollUp => "Scroll up".into(),
            ScrollDown => "Scroll down".into(),
            ScrollUpPage => "Scroll up one page".into(),
            ScrollDownPage => "Scroll down one page".into(),
            ScrollToTop => "Scroll to top".into(),
            ScrollToBottom => "Scroll to bottom".into(),
            AdjustFontSize(d) if *d >= 0 => "Increase font size".into(),
            AdjustFontSize(_) => "Decrease font size".into(),
            ResetFontSize => "Reset font size".into(),
            ToggleFullscreen => "Toggle fullscreen".into(),
            OpenNewTabDropdown => "Open new tab dropdown".into(),
            OpenSettings => "Open settings".into(),
            NewWindow => "New window".into(),
            CommandPalette => "Command palette".into(),
            Find => "Find".into(),
            RenameTab => "Rename tab".into(),
            MarkMode => "Toggle mark mode".into(),
            SelectAll => "Select all".into(),
            SendInput(text) => format!("Send input: {}", escape_input(text)),
        }
    }

    /// Every action a user can bind, for pickers and the command palette.
    pub fn catalog(profile_count: usize) -> Vec<Action> {
        use Action::*;
        let mut all = vec![NewTab(None), NewWindow];
        all.extend((0..profile_count).map(|i| NewTab(Some(i))));
        all.extend([DuplicateTab, CloseTab, CloseOtherTabs, ClosePane, NextTab, PrevTab, RenameTab]);
        all.extend((0..9).map(SwitchToTab));
        for side in [None, Some(Direction::Right), Some(Direction::Down), Some(Direction::Left), Some(Direction::Up)] {
            all.push(SplitPane { side, duplicate: false });
            all.push(SplitPane { side, duplicate: true });
        }
        for d in [Direction::Left, Direction::Right, Direction::Up, Direction::Down] {
            all.push(MoveFocus(d));
        }
        for d in [Direction::Left, Direction::Right, Direction::Up, Direction::Down] {
            all.push(ResizePane(d));
        }
        for d in [Direction::Left, Direction::Right, Direction::Up, Direction::Down] {
            all.push(SwapPane(d));
        }
        all.extend([
            TogglePaneZoom,
            Copy,
            Paste,
            Find,
            MarkMode,
            SelectAll,
            ScrollUp,
            ScrollDown,
            ScrollUpPage,
            ScrollDownPage,
            ScrollToTop,
            ScrollToBottom,
            AdjustFontSize(1),
            AdjustFontSize(-1),
            ResetFontSize,
            ToggleFullscreen,
            OpenNewTabDropdown,
            CommandPalette,
            OpenSettings,
            SendInput(String::new()),
        ]);
        all
    }
}

/// Show control characters in `sendInput` text the way users type them.
pub fn escape_input(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\x1b' => out.push_str("\\e"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 || c == '\x7f' => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape_input`]: `\e`, `\r`, `\n`, `\t`, `\\`, `\xHH`.
pub fn unescape_input(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('e') => out.push('\x1b'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                match u8::from_str_radix(&hex, 16) {
                    Ok(b) => out.push(b as char),
                    Err(_) => out.push_str(&format!("\\x{hex}")),
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// A key plus modifiers, e.g. Ctrl+Shift+T. Characters are stored unshifted
/// and lowercase ("ctrl+shift+1", not "ctrl+!").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub key: Key,
    pub mods: Mods,
}

impl Chord {
    pub fn new(key: Key, mods: Mods) -> Chord {
        Chord { key: normalize(key, mods), mods }
    }

    /// Parse WT key syntax: "ctrl+shift+t", "alt+shift+plus", "f11".
    pub fn parse(s: &str) -> Option<Chord> {
        let mut mods = Mods::NONE;
        let mut key = None;
        for part in s.split('+').map(|p| p.trim().to_ascii_lowercase()) {
            match part.as_str() {
                "ctrl" => mods.ctrl = true,
                "alt" => mods.alt = true,
                "shift" => mods.shift = true,
                "win" | "meta" | "cmd" => mods.meta = true,
                "" => return None,
                name => key = Some(parse_key(name)?),
            }
        }
        Some(Chord::new(key?, mods))
    }

    /// WT key syntax, the inverse of [`Chord::parse`].
    pub fn to_wt_string(self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.mods.meta {
            parts.push(if cfg!(target_os = "macos") { "cmd" } else { "win" }.into());
        }
        if self.mods.ctrl {
            parts.push("ctrl".into());
        }
        if self.mods.alt {
            parts.push("alt".into());
        }
        if self.mods.shift {
            parts.push("shift".into());
        }
        parts.push(key_name(self.key).to_string());
        parts.join("+")
    }

    /// Display form, e.g. "Ctrl+Shift+T", or "⇧⌘T" on macOS.
    pub fn label(self) -> String {
        if cfg!(target_os = "macos") { self.mac_label() } else { self.pc_label() }
    }

    /// Windows and Linux style: "Ctrl+Shift+T".
    pub fn pc_label(self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.mods.meta {
            parts.push("Win".into());
        }
        if self.mods.ctrl {
            parts.push("Ctrl".into());
        }
        if self.mods.alt {
            parts.push("Alt".into());
        }
        if self.mods.shift {
            parts.push("Shift".into());
        }
        parts.push(match self.key {
            Key::Char(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase().to_string(),
            Key::Char(' ') => "Space".into(),
            Key::Char('=') => "Plus".into(),
            Key::Char('-') => "Minus".into(),
            Key::Char(c) => c.to_string(),
            Key::PageUp => "PgUp".into(),
            Key::PageDown => "PgDn".into(),
            Key::Escape => "Esc".into(),
            Key::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        });
        parts.join("+")
    }

    /// macOS style: modifier symbols in Apple's order (⌃⌥⇧⌘), then the key.
    pub fn mac_label(self) -> String {
        let mut label = String::new();
        for (on, symbol) in [(self.mods.ctrl, '⌃'), (self.mods.alt, '⌥'), (self.mods.shift, '⇧'), (self.mods.meta, '⌘')] {
            if on {
                label.push(symbol);
            }
        }
        label += &match self.key {
            Key::Char(' ') => "Space".into(),
            Key::Char('=') => "+".into(),
            Key::Char(c) => c.to_uppercase().to_string(),
            Key::Enter => "↩".into(),
            Key::Tab => "⇥".into(),
            Key::Backspace => "⌫".into(),
            Key::Delete => "⌦".into(),
            Key::Escape => "⎋".into(),
            Key::Up => "↑".into(),
            Key::Down => "↓".into(),
            Key::Left => "←".into(),
            Key::Right => "→".into(),
            Key::PageUp => "⇞".into(),
            Key::PageDown => "⇟".into(),
            Key::Home => "↖".into(),
            Key::End => "↘".into(),
            Key::Insert => "Insert".into(),
            Key::F(n) => format!("F{n}"),
        };
        label
    }
}

fn key_name(key: Key) -> String {
    match key {
        Key::Char(' ') => "space".into(),
        Key::Char('+') | Key::Char('=') => "plus".into(),
        Key::Char('-') => "minus".into(),
        Key::Char(c) => match PUNCTUATION.iter().find(|(_, p)| *p == c) {
            Some((name, _)) => name.to_string(),
            None => c.to_string(),
        },
        Key::Enter => "enter".into(),
        Key::Tab => "tab".into(),
        Key::Backspace => "backspace".into(),
        Key::Escape => "esc".into(),
        Key::Insert => "insert".into(),
        Key::Delete => "delete".into(),
        Key::Home => "home".into(),
        Key::End => "end".into(),
        Key::PageUp => "pgup".into(),
        Key::PageDown => "pgdn".into(),
        Key::Up => "up".into(),
        Key::Down => "down".into(),
        Key::Left => "left".into(),
        Key::Right => "right".into(),
        Key::F(n) => format!("f{n}"),
    }
}

/// WT's names for punctuation keys.
const PUNCTUATION: &[(&str, char)] = &[
    ("comma", ','),
    ("period", '.'),
    ("backtick", '`'),
    ("semicolon", ';'),
    ("quote", '\''),
    ("slash", '/'),
    ("backslash", '\\'),
    ("openbracket", '['),
    ("closebracket", ']'),
];

fn parse_key(name: &str) -> Option<Key> {
    if let Some((_, c)) = PUNCTUATION.iter().find(|(n, _)| *n == name) {
        return Some(Key::Char(*c));
    }
    Some(match name {
        "space" => Key::Char(' '),
        // WT's "plus" is the =/+ key.
        "plus" => Key::Char('='),
        "minus" => Key::Char('-'),
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "esc" | "escape" => Key::Escape,
        "insert" | "ins" => Key::Insert,
        "delete" | "del" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pgup" | "pageup" => Key::PageUp,
        "pgdn" | "pagedown" => Key::PageDown,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
            Key::F(f[1..].parse().ok()?)
        }
        c if c.chars().count() == 1 => Key::Char(c.chars().next()?),
        _ => return None,
    })
}

/// Undo Shift's effect on the character so chords can be written with the
/// unshifted key, like WT does ("ctrl+shift+1", not "ctrl+!").
/// TODO(input): use physical key codes so this works beyond US layouts.
fn normalize(key: Key, mods: Mods) -> Key {
    const SHIFTED: &str = "!@#$%^&*()_+{}|:\"<>?~";
    const BASE: &str = "1234567890-=[]\\;',./`";
    match key {
        Key::Char(c) if mods.shift => match SHIFTED.find(c) {
            Some(i) => Key::Char(BASE.as_bytes()[i] as char),
            None => Key::Char(c.to_ascii_lowercase()),
        },
        Key::Char('+') => Key::Char('='),
        Key::Char(c) => Key::Char(c.to_ascii_lowercase()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_round_trip() {
        for s in ["ctrl+comma", "win+backtick", "ctrl+shift+t", "alt+shift+plus", "alt+shift+minus", "ctrl+shift+space", "f11", "ctrl+alt+1", "alt+enter", "ctrl+pgup"] {
            let chord = Chord::parse(s).unwrap();
            // macOS writes the ⌘ key as "cmd" ("win" elsewhere).
            if !(cfg!(target_os = "macos") && s.starts_with("win")) {
                assert_eq!(chord.to_wt_string(), s);
            }
            assert_eq!(Chord::parse(&chord.to_wt_string()), Some(chord));
        }
        assert_eq!(Chord::parse("Ctrl+Shift+T"), Chord::parse("ctrl+shift+t"));
        assert_eq!(Chord::parse("ctrl+"), None);
        assert_eq!(Chord::parse("ctrl+bogus"), None);
    }

    #[test]
    fn chord_from_shifted_text() {
        let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::NONE };
        assert_eq!(Chord::new(Key::Char('!'), ctrl_shift), Chord::parse("ctrl+shift+1").unwrap());
        assert_eq!(Chord::new(Key::Char('T'), ctrl_shift), Chord::parse("ctrl+shift+t").unwrap());
        let alt_shift = Mods { alt: true, shift: true, ..Mods::NONE };
        assert_eq!(Chord::new(Key::Char('+'), alt_shift), Chord::parse("alt+shift+plus").unwrap());
    }

    #[test]
    fn labels() {
        let pc = |s: &str| Chord::parse(s).unwrap().pc_label();
        let mac = |s: &str| Chord::parse(s).unwrap().mac_label();
        assert_eq!(pc("ctrl+shift+t"), "Ctrl+Shift+T");
        assert_eq!(pc("alt+shift+plus"), "Alt+Shift+Plus");
        assert_eq!(pc("f11"), "F11");
        assert_eq!(pc("ctrl+shift+pgup"), "Ctrl+Shift+PgUp");
        assert_eq!(pc("alt+left"), "Alt+Left");
        assert_eq!(mac("cmd+shift+p"), "⇧⌘P");
        assert_eq!(mac("cmd+ctrl+left"), "⌃⌘←");
        assert_eq!(mac("cmd+left"), "⌘←");
        assert_eq!(mac("cmd+backspace"), "⌘⌫");
        assert_eq!(mac("cmd+delete"), "⌘⌦");
        let ctrl_shift_t = Chord::parse("ctrl+shift+t").unwrap();
        let native = if cfg!(target_os = "macos") { "⌃⇧T" } else { "Ctrl+Shift+T" };
        assert_eq!(ctrl_shift_t.label(), native);
    }

    #[test]
    fn line_editing_chords_round_trip() {
        for s in ["cmd+left", "cmd+right", "cmd+backspace", "cmd+delete", "cmd+up", "cmd+down"] {
            let chord = Chord::parse(s).unwrap();
            assert_eq!(Chord::parse(&chord.to_wt_string()), Some(chord), "{s}");
        }
    }

    #[test]
    fn action_json_round_trip() {
        let inputs = ["\x1b\r", "\x01", "\x05", "\x15", "\x0b"].map(|s| Action::SendInput(s.into()));
        for action in Action::catalog(3).into_iter().chain(inputs) {
            let json = action.to_json();
            assert_eq!(Action::from_json(&json), Ok(Some(action.clone())), "{json}");
        }
    }

    #[test]
    fn action_json_wt_variants() {
        let parse = |v: Value| Action::from_json(&v).unwrap();
        assert_eq!(parse(json!("unbound")), None);
        assert_eq!(
            parse(json!({"action": "splitPane", "split": "vertical"})),
            Some(Action::SplitPane { side: Some(Direction::Right), duplicate: false })
        );
        assert_eq!(parse(json!({"action": "copy", "singleLine": true})), Some(Action::Copy));
        assert!(Action::from_json(&json!("quakeMode")).is_err());
    }

    #[test]
    fn input_escapes() {
        assert_eq!(unescape_input(r"\e\r\x7fa\\"), "\x1b\r\x7fa\\");
        assert_eq!(escape_input("\x1b\r\x7fa\\"), r"\e\r\x7fa\\");
    }
}
