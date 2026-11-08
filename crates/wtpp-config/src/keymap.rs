//! The effective key bindings: our defaults (Windows Terminal's, minus the
//! ones that collide with the OS), overlaid with the user's `actions` (where
//! `"unbound"` removes a default).

use serde_json::Value;
use wtpp_input::{Key, Mods};

use crate::actions::{Action, Chord, Direction};
use crate::settings::ActionEntry;

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub chord: Chord,
    pub action: Action,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
}

impl Keymap {
    /// The default bindings: Windows Terminal's (`defaults.json`), except
    /// where WT's collide with the OS:
    /// - Alt+Shift (WT's pane keys) is Windows' keyboard-layout hotkey, so
    ///   splitting and resizing panes kept switching layouts. Panes use
    ///   Ctrl+Shift+Plus/Minus and Ctrl+Shift+arrows instead.
    /// - Ctrl+Alt is AltGr on many layouts (Ctrl+Alt+7 types `{` in German),
    ///   so switching tabs is Alt+1..9. Only Plus/Minus use it, which few
    ///   layouts type anything on: split with the default profile.
    pub fn defaults() -> Keymap {
        if cfg!(target_os = "macos") { Keymap::mac_defaults() } else { Keymap::pc_defaults() }
    }

    /// macOS: app shortcuts on ⌘ as in Terminal and iTerm (⌘T, ⌘W, ⌘D to
    /// split, ⌥⌘arrows between panes); Ctrl always goes to the shell.
    pub fn mac_defaults() -> Keymap {
        use Action::*;
        use Direction::*;

        let mut b: Vec<(String, Action)> = vec![
            ("cmd+ctrl+f".into(), ToggleFullscreen),
            ("cmd+shift+space".into(), OpenNewTabDropdown),
            ("cmd+comma".into(), OpenSettings),
            ("cmd+f".into(), Find),
            ("cmd+shift+p".into(), CommandPalette),
            ("cmd+shift+m".into(), MarkMode),
            ("cmd+a".into(), SelectAll),
            ("cmd+t".into(), NewTab(None)),
            ("cmd+n".into(), NewWindow),
            ("cmd+w".into(), ClosePane),
            ("cmd+shift+]".into(), NextTab),
            ("cmd+shift+[".into(), PrevTab),
            ("ctrl+tab".into(), NextTab),
            ("ctrl+shift+tab".into(), PrevTab),
            ("cmd+d".into(), SplitPane { side: Some(Right), duplicate: true }),
            ("cmd+shift+d".into(), SplitPane { side: Some(Down), duplicate: true }),
            ("ctrl+alt+plus".into(), SplitPane { side: Some(Right), duplicate: false }),
            ("ctrl+alt+minus".into(), SplitPane { side: Some(Down), duplicate: false }),
            ("cmd+shift+enter".into(), TogglePaneZoom),
            ("cmd+c".into(), Copy),
            ("cmd+v".into(), Paste),
            ("cmd+pgup".into(), ScrollUpPage),
            ("cmd+pgdn".into(), ScrollDownPage),
            ("cmd+home".into(), ScrollToTop),
            ("cmd+end".into(), ScrollToBottom),
            ("cmd+plus".into(), AdjustFontSize(1)),
            ("cmd+minus".into(), AdjustFontSize(-1)),
            ("cmd+0".into(), ResetFontSize),
            // Line editing as in iTerm's "Natural Text Editing": ⌘←/→ go to
            // the start/end of the line (^A/^E), ⌘⌫ deletes to its start
            // (^U), ⌘⌦ to its end (^K).
            ("cmd+left".into(), SendInput("\x01".into())),
            ("cmd+right".into(), SendInput("\x05".into())),
            ("cmd+backspace".into(), SendInput("\x15".into())),
            ("cmd+delete".into(), SendInput("\x0b".into())),
            ("cmd+up".into(), ScrollUp),
            ("cmd+down".into(), ScrollDown),
        ];
        for (k, d) in [("left", Left), ("right", Right), ("up", Up), ("down", Down)] {
            b.push((format!("cmd+alt+{k}"), MoveFocus(d)));
            b.push((format!("cmd+ctrl+{k}"), ResizePane(d)));
        }
        for n in 1..=9 {
            b.push((format!("cmd+{n}"), SwitchToTab(n - 1)));
            b.push((format!("cmd+shift+{n}"), NewTab(Some(n - 1))));
        }
        Keymap::from_list(b.iter().map(|(k, a)| (k.as_str(), a.clone())))
    }

    /// Windows and Linux.
    pub fn pc_defaults() -> Keymap {
        use Action::*;
        use Direction::*;

        let mut b: Vec<(&str, Action)> = vec![
            ("alt+enter", ToggleFullscreen),
            ("f11", ToggleFullscreen),
            ("ctrl+shift+space", OpenNewTabDropdown),
            ("ctrl+comma", OpenSettings),
            ("ctrl+shift+f", Find),
            ("ctrl+shift+p", CommandPalette),
            ("ctrl+shift+m", MarkMode),
            ("ctrl+shift+a", SelectAll),
            ("ctrl+shift+t", NewTab(None)),
            ("ctrl+shift+n", NewWindow),
            ("ctrl+shift+d", DuplicateTab),
            ("ctrl+shift+w", ClosePane),
            ("ctrl+tab", NextTab),
            ("ctrl+shift+tab", PrevTab),
            ("ctrl+shift+plus", SplitPane { side: Some(Right), duplicate: true }),
            ("ctrl+shift+minus", SplitPane { side: Some(Down), duplicate: true }),
            ("ctrl+alt+plus", SplitPane { side: Some(Right), duplicate: false }),
            ("ctrl+alt+minus", SplitPane { side: Some(Down), duplicate: false }),
            ("ctrl+c", Copy),
            ("ctrl+shift+c", Copy),
            ("ctrl+insert", Copy),
            ("ctrl+v", Paste),
            ("ctrl+shift+v", Paste),
            ("shift+insert", Paste),
            ("ctrl+shift+pgup", ScrollUpPage),
            ("ctrl+shift+pgdn", ScrollDownPage),
            ("ctrl+shift+home", ScrollToTop),
            ("ctrl+shift+end", ScrollToBottom),
            ("ctrl+plus", AdjustFontSize(1)),
            ("ctrl+minus", AdjustFontSize(-1)),
            ("ctrl+0", ResetFontSize),
        ];
        let dirs = [("left", Left), ("right", Right), ("up", Up), ("down", Down)];
        let move_focus: Vec<_> = dirs.iter().map(|(k, d)| (format!("alt+{k}"), MoveFocus(*d))).collect();
        let resize: Vec<_> = dirs.iter().map(|(k, d)| (format!("ctrl+shift+{k}"), ResizePane(*d))).collect();
        let new_tabs: Vec<_> = (1..=9).map(|n| (format!("ctrl+shift+{n}"), NewTab(Some(n - 1)))).collect();
        let switch: Vec<_> = (1..=9).map(|n| (format!("alt+{n}"), SwitchToTab(n - 1))).collect();
        let owned: Vec<(String, Action)> = move_focus.into_iter().chain(resize).chain(new_tabs).chain(switch).collect();
        b.extend(owned.iter().map(|(k, a)| (k.as_str(), a.clone())));
        Keymap::from_list(b)
    }

    fn from_list<'a>(list: impl IntoIterator<Item = (&'a str, Action)>) -> Keymap {
        Keymap {
            bindings: list
                .into_iter()
                .map(|(keys, action)| Binding {
                    chord: Chord::parse(keys).unwrap_or_else(|| panic!("bad default chord {keys}")),
                    action,
                })
                .collect(),
        }
    }

    /// Defaults with the user's entries applied in order: an entry binds its
    /// keys to its command (replacing any previous binding of those keys), and
    /// `"unbound"` removes them.
    pub fn from_user_actions(entries: &[ActionEntry]) -> (Keymap, Vec<String>) {
        let mut keymap = Keymap::defaults();
        let mut warnings = Vec::new();
        for entry in entries {
            let action = match Action::from_json(&entry.command) {
                Ok(action) => action,
                Err(e) => {
                    warnings.push(format!("{}: {e}", entry.command));
                    continue;
                }
            };
            for keys in entry.keys.iter() {
                let Some(chord) = Chord::parse(keys) else {
                    warnings.push(format!("unknown key combination {keys:?}"));
                    continue;
                };
                keymap.bindings.retain(|b| b.chord != chord);
                if let Some(action) = &action {
                    keymap.bindings.push(Binding { chord, action: action.clone() });
                }
            }
        }
        (keymap, warnings)
    }

    /// The minimal user `actions` that turn the defaults into `self`:
    /// new/changed bindings, plus `"unbound"` for removed defaults.
    pub fn to_user_actions(&self) -> Vec<ActionEntry> {
        let defaults = Keymap::defaults();
        let mut entries = Vec::new();
        for d in &defaults.bindings {
            if !self.bindings.iter().any(|b| b.chord == d.chord) {
                entries.push(ActionEntry::new(Value::from("unbound"), d.chord));
            }
        }
        for b in &self.bindings {
            if !defaults.bindings.contains(b) {
                entries.push(ActionEntry::new(b.action.to_json(), b.chord));
            }
        }
        entries
    }

    pub fn lookup(&self, key: Key, mods: Mods) -> Option<&Action> {
        let chord = Chord::new(key, mods);
        self.bindings.iter().find(|b| b.chord == chord).map(|b| &b.action)
    }

    /// Label of the first chord bound to `action`, e.g. "Ctrl+Shift+1".
    pub fn chord_label(&self, action: &Action) -> Option<String> {
        self.bindings.iter().find(|b| &b.action == action).map(|b| b.chord.label())
    }

    /// Bind `chord` to `action`, replacing whatever used `chord` before.
    /// Returns the action that was displaced, if any.
    pub fn bind(&mut self, chord: Chord, action: Action) -> Option<Action> {
        let previous = self.bindings.iter().position(|b| b.chord == chord).map(|i| self.bindings.remove(i).action);
        self.bindings.push(Binding { chord, action });
        previous
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(command: Value, keys: &str) -> ActionEntry {
        ActionEntry::new(command, Chord::parse(keys).unwrap())
    }

    #[test]
    fn defaults_parse() {
        let km = Keymap::pc_defaults();
        let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::NONE };
        assert_eq!(km.lookup(Key::Char('T'), ctrl_shift), Some(&Action::NewTab(None)));
        assert_eq!(km.lookup(Key::Char('!'), ctrl_shift), Some(&Action::NewTab(Some(0))));
        assert_eq!(km.lookup(Key::Char('t'), Mods::CTRL), None);
        assert_eq!(km.lookup(Key::Backspace, Mods::CTRL), None);
        let right = Action::SplitPane { side: Some(Direction::Right), duplicate: true };
        let down = Action::SplitPane { side: Some(Direction::Down), duplicate: true };
        assert_eq!(km.lookup(Key::Char('+'), ctrl_shift), Some(&right));
        assert_eq!(km.lookup(Key::Char('_'), ctrl_shift), Some(&down));
        assert_eq!(km.lookup(Key::Char('='), Mods::CTRL), Some(&Action::AdjustFontSize(1)));
    }

    #[test]
    fn mac_defaults() {
        let km = Keymap::mac_defaults();
        let cmd = Mods { meta: true, ..Mods::NONE };
        let cmd_shift = Mods { meta: true, shift: true, ..Mods::NONE };
        assert_eq!(km.lookup(Key::Char('t'), cmd), Some(&Action::NewTab(None)));
        assert_eq!(km.lookup(Key::Char('c'), cmd), Some(&Action::Copy));
        assert_eq!(km.lookup(Key::Char('}'), cmd_shift), Some(&Action::NextTab));
        // Ctrl+C, Ctrl+V and friends belong to the shell.
        for c in 'a'..='z' {
            assert_eq!(km.lookup(Key::Char(c), Mods::CTRL), None, "ctrl+{c}");
        }
        let chords: std::collections::HashSet<_> = km.bindings.iter().map(|b| b.chord).collect();
        assert_eq!(chords.len(), km.bindings.len());
    }

    #[test]
    fn split_keys_pick_the_profile() {
        let split = |side, duplicate| Some(Action::SplitPane { side: Some(side), duplicate });
        let ctrl_alt = Mods { ctrl: true, alt: true, ..Mods::NONE };
        let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::NONE };
        let cmd = Mods { meta: true, ..Mods::NONE };
        for km in [Keymap::pc_defaults(), Keymap::mac_defaults()] {
            // Ctrl+Alt+Plus/Minus: the default profile, typed with or without Shift's "+".
            for plus in ['=', '+'] {
                assert_eq!(km.lookup(Key::Char(plus), ctrl_alt).cloned(), split(Direction::Right, false));
            }
            assert_eq!(km.lookup(Key::Char('-'), ctrl_alt).cloned(), split(Direction::Down, false));
        }
        // The other split shortcuts open the focused pane's profile.
        assert_eq!(Keymap::pc_defaults().lookup(Key::Char('='), ctrl_shift).cloned(), split(Direction::Right, true));
        assert_eq!(Keymap::pc_defaults().lookup(Key::Char('-'), ctrl_shift).cloned(), split(Direction::Down, true));
        assert_eq!(Keymap::mac_defaults().lookup(Key::Char('d'), cmd).cloned(), split(Direction::Right, true));
    }

    #[test]
    fn mac_line_editing() {
        let km = Keymap::mac_defaults();
        let cmd = Mods { meta: true, ..Mods::NONE };
        let send = |s: &str| Some(Action::SendInput(s.into()));
        assert_eq!(km.lookup(Key::Left, cmd).cloned(), send("\x01"));
        assert_eq!(km.lookup(Key::Right, cmd).cloned(), send("\x05"));
        assert_eq!(km.lookup(Key::Backspace, cmd).cloned(), send("\x15"));
        assert_eq!(km.lookup(Key::Delete, cmd).cloned(), send("\x0b"));
        assert_eq!(km.lookup(Key::Up, cmd), Some(&Action::ScrollUp));
        assert_eq!(km.lookup(Key::Down, cmd), Some(&Action::ScrollDown));
        // The ⌥⌘ and ⌃⌘ arrows still move between and resize panes.
        let cmd_alt = Mods { meta: true, alt: true, ..Mods::NONE };
        let cmd_ctrl = Mods { meta: true, ctrl: true, ..Mods::NONE };
        assert_eq!(km.lookup(Key::Left, cmd_alt), Some(&Action::MoveFocus(Direction::Left)));
        assert_eq!(km.lookup(Key::Left, cmd_ctrl), Some(&Action::ResizePane(Direction::Left)));
        // Windows and Linux keep these keys for the shell.
        let pc = Keymap::pc_defaults();
        for key in [Key::Left, Key::Right, Key::Backspace, Key::Delete] {
            assert_eq!(pc.lookup(key, cmd), None, "{key:?}");
        }
    }

    #[test]
    fn defaults_avoid_os_hotkeys() {
        for b in Keymap::pc_defaults().bindings {
            let m = b.chord.mods;
            assert!(!(m.alt && m.shift), "{} uses Alt+Shift (Windows layout switch)", b.chord.label());
            let plus_minus = matches!(b.chord.key, Key::Char('=' | '-'));
            assert!(!(m.ctrl && m.alt) || plus_minus, "{} uses Ctrl+Alt (AltGr)", b.chord.label());
        }
    }

    #[test]
    fn user_overrides_and_unbinds() {
        let (km, warnings) = Keymap::from_user_actions(&[
            entry(json!("unbound"), "ctrl+v"),
            entry(json!({"action": "sendInput", "input": "\u{1b}\r"}), "shift+enter"),
            entry(json!("closeTab"), "ctrl+shift+w"),
            entry(json!("quakeMode"), "win+backtick"),
        ]);
        assert_eq!(warnings.len(), 1);
        assert_eq!(km.lookup(Key::Char('v'), Mods::CTRL), None);
        assert_eq!(km.lookup(Key::Enter, Mods::SHIFT), Some(&Action::SendInput("\x1b\r".into())));
        let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::NONE };
        assert_eq!(km.lookup(Key::Char('W'), ctrl_shift), Some(&Action::CloseTab));
    }

    #[test]
    fn user_actions_round_trip() {
        let mut km = Keymap::defaults();
        assert!(km.to_user_actions().is_empty());

        // Defaults differ per platform: unbind whatever Paste is on here.
        let paste = km.bindings.iter().find(|b| b.action == Action::Paste).unwrap().chord;
        km.bind(Chord::parse("ctrl+shift+w").unwrap(), Action::CloseTab);
        km.bindings.retain(|b| b.chord != paste);
        km.bind(Chord::parse("ctrl+alt+s").unwrap(), Action::SplitPane { side: None, duplicate: false });

        let entries = km.to_user_actions();
        assert_eq!(entries.len(), 3);
        let (rebuilt, warnings) = Keymap::from_user_actions(&entries);
        assert!(warnings.is_empty());
        for b in &km.bindings {
            assert!(rebuilt.bindings.contains(b), "{b:?}");
        }
        assert_eq!(rebuilt.bindings.len(), km.bindings.len());
    }
}
