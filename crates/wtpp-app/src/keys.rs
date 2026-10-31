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
}
