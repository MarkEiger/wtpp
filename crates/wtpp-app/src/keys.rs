//! Mapping Slint key events onto `wtpp_input` keys.

use slint::platform::Key as SlintKey;
use wtpp_input::{Key, Mods};

/// Convert a Slint `KeyEvent.text` into a terminal key. Returns `None` for
/// modifier-only presses and keys a terminal doesn't send.
pub fn from_slint(text: &str, mut mods: Mods) -> Option<(Key, Mods)> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        // Multi-char text (IME commit, dead-key composition): only the first
        // char is a "key"; the app writes the whole string as text instead.
        return None;
    }

    let special = |k: SlintKey| char::from(k) == c;
    let key = if special(SlintKey::Backspace) {
        Key::Backspace
    } else if special(SlintKey::Tab) {
        Key::Tab
    } else if special(SlintKey::Backtab) {
        mods.shift = true;
        Key::Tab
    } else if special(SlintKey::Return) {
        Key::Enter
    } else if special(SlintKey::Escape) {
        Key::Escape
    } else if special(SlintKey::Delete) {
        Key::Delete
    } else if special(SlintKey::Insert) {
        Key::Insert
    } else if special(SlintKey::Home) {
        Key::Home
    } else if special(SlintKey::End) {
        Key::End
    } else if special(SlintKey::PageUp) {
        Key::PageUp
    } else if special(SlintKey::PageDown) {
        Key::PageDown
    } else if special(SlintKey::UpArrow) {
        Key::Up
    } else if special(SlintKey::DownArrow) {
        Key::Down
    } else if special(SlintKey::LeftArrow) {
        Key::Left
    } else if special(SlintKey::RightArrow) {
        Key::Right
    } else if let Some(n) = function_key(c) {
        Key::F(n)
    } else if is_private_use(c) || c.is_control() {
        // Modifier keys (Slint uses U+0010..U+0018 for them), media keys, etc.
        // Ctrl+letter arrives as the plain letter, so no control char here is text.
        return None;
    } else {
        Key::Char(c)
    };
    Some((key, mods))
}

/// The modifiers to act on: Slint's, less any the OS says are up.
///
/// Slint tracks modifiers from its own key events and never re-syncs them,
/// so a Shift release it missed would leave every later Enter a Shift+Enter
/// (a new line in Claude Code and PSReadLine) until the next Shift press.
/// A guard, not a proven cause of the reported stuck Shift+Enter: that one
/// ended with Ctrl+Enter, which doesn't clear Slint's Shift (see
/// `slint_state`). winit's state (`os`) comes from the OS on every key. This only ever clears a modifier:
/// Slint drops Ctrl+Alt for AltGr characters, and adding them back would
/// turn AltGr typing into shortcuts. `None` (no OS state seen yet) trusts
/// Slint.
pub fn reconcile(reported: Mods, os: Option<Mods>) -> Mods {
    let Some(os) = os else { return reported };
    Mods {
        shift: reported.shift && os.shift,
        alt: reported.alt && os.alt,
        ctrl: reported.ctrl && os.ctrl,
        meta: reported.meta && os.meta,
    }
}

fn function_key(c: char) -> Option<u8> {
    let f1 = char::from(SlintKey::F1) as u32;
    let n = (c as u32).checked_sub(f1)?;
    (n < 12).then_some(n as u8 + 1)
}

fn is_private_use(c: char) -> bool {
    ('\u{E000}'..='\u{F8FF}').contains(&c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: SlintKey) -> String {
        char::from(k).to_string()
    }

    #[test]
    fn special_keys() {
        assert_eq!(from_slint(&key(SlintKey::Backspace), Mods::CTRL), Some((Key::Backspace, Mods::CTRL)));
        assert_eq!(from_slint(&key(SlintKey::Return), Mods::SHIFT), Some((Key::Enter, Mods::SHIFT)));
        assert_eq!(from_slint(&key(SlintKey::Backtab), Mods::NONE), Some((Key::Tab, Mods::SHIFT)));
        assert_eq!(from_slint(&key(SlintKey::F5), Mods::NONE), Some((Key::F(5), Mods::NONE)));
        assert_eq!(from_slint(&key(SlintKey::UpArrow), Mods::NONE), Some((Key::Up, Mods::NONE)));
    }

    #[test]
    fn modifiers_alone_are_ignored() {
        assert_eq!(from_slint(&key(SlintKey::Shift), Mods::SHIFT), None);
        assert_eq!(from_slint(&key(SlintKey::Control), Mods::CTRL), None);
        assert_eq!(from_slint(&key(SlintKey::MetaR), Mods::NONE), None);
    }

    #[test]
    fn ctrl_letters() {
        assert_eq!(from_slint("c", Mods::CTRL), Some((Key::Char('c'), Mods::CTRL)));
    }

    #[test]
    fn reconcile_clears_what_the_os_says_is_up() {
        assert_eq!(reconcile(Mods::SHIFT, Some(Mods::NONE)), Mods::NONE);
        assert_eq!(reconcile(Mods::SHIFT, Some(Mods::SHIFT)), Mods::SHIFT);
        let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::NONE };
        assert_eq!(reconcile(ctrl_shift, Some(Mods::CTRL)), Mods::CTRL);
        assert_eq!(reconcile(Mods::SHIFT, None), Mods::SHIFT);
    }

    #[test]
    fn reconcile_never_adds_altgr_back() {
        let ctrl_alt = Mods { ctrl: true, alt: true, ..Mods::NONE };
        assert_eq!(reconcile(Mods::NONE, Some(ctrl_alt)), Mods::NONE);
    }

    /// With a Shift Slint thinks is held but the OS says is up, Enter
    /// submits (a CR, or the plain Enter key event) instead of a new line.
    #[test]
    fn enter_after_a_lost_shift_release_submits() {
        use wtpp_input::{EncodeOptions, KeyStyle, encode};
        let (key, mods) = from_slint(&key(SlintKey::Return), reconcile(Mods::SHIFT, Some(Mods::NONE))).unwrap();
        assert_eq!((key, mods), (Key::Enter, Mods::NONE));
        assert_eq!(encode(key, mods, &EncodeOptions::default()), Some(b"\r".to_vec()));
        let windows = EncodeOptions { style: KeyStyle::Windows, win32_input: true, ..EncodeOptions::default() };
        assert_eq!(encode(key, mods, &windows), encode(Key::Enter, Mods::NONE, &windows));
        assert_ne!(encode(key, mods, &windows), encode(Key::Enter, Mods::SHIFT, &windows));
    }
}

/// What Slint itself reports for a key, through a real (GPU-less) window.
#[cfg(test)]
mod slint_state {
    use slint::platform::software_renderer::MinimalSoftwareWindow;
    use slint::platform::{Key as SlintKey, Platform, PlatformError, WindowAdapter, WindowEvent};
    use std::rc::Rc;

    slint::slint! {
        export component Probe inherits Window {
            in-out property <string> mods;
            fs := FocusScope {
                key-pressed(e) => {
                    root.mods = (e.modifiers.shift ? "shift " : "") + (e.modifiers.control ? "ctrl" : "");
                    accept
                }
            }
            init => { fs.focus(); }
        }
    }

    struct Headless(Rc<MinimalSoftwareWindow>);

    impl Platform for Headless {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
            Ok(self.0.clone())
        }
    }

    /// Slint keeps modifiers from its own key events: a Shift whose release
    /// it never got stays held for every later key, and only a Shift event
    /// (or focus loss) clears it. Ctrl, or Ctrl+Enter, doesn't. So a lost
    /// release alone doesn't explain a stuck Shift+Enter that Ctrl+Enter
    /// ends; `reconcile` covers this case, whatever the cause of that one.
    #[test]
    fn slint_keeps_a_shift_it_never_saw_released() {
        let window = MinimalSoftwareWindow::new(Default::default());
        slint::platform::set_platform(Box::new(Headless(window.clone()))).unwrap();
        let probe = Probe::new().unwrap();
        probe.show().unwrap();
        let w = probe.window();
        let press = |k: SlintKey| w.dispatch_event(WindowEvent::KeyPressed { text: k.into() });
        let release = |k: SlintKey| w.dispatch_event(WindowEvent::KeyReleased { text: k.into() });
        let enter = || {
            press(SlintKey::Return);
            release(SlintKey::Return);
            probe.get_mods()
        };
        press(SlintKey::Shift);
        assert_eq!(enter(), "shift ");
        // The release is lost: Enter stays Shift+Enter.
        assert_eq!(enter(), "shift ");
        // Ctrl+Enter is Ctrl+Shift+Enter, and Shift is still held after it.
        press(SlintKey::Control);
        assert_eq!(enter(), "shift ctrl");
        release(SlintKey::Control);
        assert_eq!(enter(), "shift ");
        // A Shift press and release clears it.
        press(SlintKey::Shift);
        release(SlintKey::Shift);
        assert_eq!(enter(), "");
    }
}
