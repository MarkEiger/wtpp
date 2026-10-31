//! Key event → PTY byte encoding.
//!
//! The encoder is deliberately independent of any UI toolkit: the app maps
//! its native key events onto [`Key`] + [`Mods`] and gets back the bytes to
//! write to the shell.

/// A logical key, after keyboard layout has been applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable character. For shifted keys this is already the shifted
    /// character (e.g. `'A'`, `'!'`).
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
    /// Function key F1..=F12.
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { shift: false, alt: false, ctrl: false, meta: false };
    pub const SHIFT: Mods = Mods { shift: true, ..Mods::NONE };
    pub const ALT: Mods = Mods { alt: true, ..Mods::NONE };
    pub const CTRL: Mods = Mods { ctrl: true, ..Mods::NONE };

    pub fn is_empty(self) -> bool {
        self == Mods::NONE
    }

    /// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4) + meta(8).
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8 + 8 * self.meta as u8
    }
}

/// Which family of shells a profile talks to. This only changes the handful
/// of keys whose "edit word" behavior differs between readline-style shells
/// and PSReadLine/cmd.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyStyle {
    /// bash, zsh, fish, sh… (readline/zle defaults).
    #[default]
    Unix,
    /// cmd, Windows PowerShell, pwsh (matches Windows Terminal's bytes).
    Windows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeOptions {
    pub style: KeyStyle,
    /// DECCKM: cursor keys send SS3 (`ESC O x`) instead of CSI.
    pub app_cursor: bool,
    /// The app enabled kitty keyboard protocol "disambiguate escape codes".
    pub kitty_disambiguate: bool,
    /// Bytes for Shift+Enter when the app hasn't asked for kitty encoding.
    pub shift_enter: Vec<u8>,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            style: KeyStyle::Unix,
            app_cursor: false,
            kitty_disambiguate: false,
            // ESC CR: what most TUIs (editors, chat-style CLIs) treat as
            // "insert newline" and what shells ignore harmlessly.
            shift_enter: b"\x1b\r".to_vec(),
        }
    }
}

/// Encode a key press. Returns `None` for combinations that produce no input.
pub fn encode(key: Key, mods: Mods, opts: &EncodeOptions) -> Option<Vec<u8>> {
    // cmd/PowerShell read Windows key events, not VT; give ConPTY the exact
    // key (like Windows Terminal) rather than a sequence it has to guess at.
    if opts.style == KeyStyle::Windows
        && !matches!(key, Key::Char(_) if !mods.ctrl && !mods.alt)
        && let Some(bytes) = encode_win32(key, mods)
    {
        return Some(bytes);
    }
    if opts.kitty_disambiguate
        && let Some(bytes) = encode_kitty(key, mods)
    {
        return Some(bytes);
    }

    let bytes = match key {
        Key::Char(c) => return encode_char(c, mods),

        Key::Enter => match (mods.shift, mods.ctrl, mods.alt) {
            (true, false, false) => opts.shift_enter.clone(),
            (_, _, true) => b"\x1b\r".to_vec(),
            _ => b"\r".to_vec(),
        },

        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),

        Key::Backspace => match (mods.ctrl, mods.alt, opts.style) {
            // Delete word backwards: ^W is unix-word-rubout in bash and zsh;
            // ^H is BackwardKillWord in PSReadLine (and what WT sends).
            (true, _, KeyStyle::Unix) => vec![0x17],
            (true, _, KeyStyle::Windows) => vec![0x08],
            (false, true, _) => b"\x1b\x7f".to_vec(),
            (false, false, _) => vec![0x7f],
        },

        Key::Escape => vec![0x1b],

        // Delete word forwards: ESC d is readline/zle kill-word; PSReadLine
        // understands the xterm-style CSI 3;5~ through ConPTY.
        Key::Delete if mods == Mods::CTRL && opts.style == KeyStyle::Unix => b"\x1bd".to_vec(),

        Key::Insert => tilde(2, mods),
        Key::Delete => tilde(3, mods),
        Key::PageUp => tilde(5, mods),
        Key::PageDown => tilde(6, mods),

        Key::Up => cursor(b'A', mods, opts.app_cursor),
        Key::Down => cursor(b'B', mods, opts.app_cursor),
        Key::Right => cursor(b'C', mods, opts.app_cursor),
        Key::Left => cursor(b'D', mods, opts.app_cursor),
        Key::Home => cursor(b'H', mods, opts.app_cursor),
        Key::End => cursor(b'F', mods, opts.app_cursor),

        Key::F(n @ 1..=4) => {
            let c = b"PQRS"[n as usize - 1];
            if mods.is_empty() { vec![0x1b, b'O', c] } else { csi_mod(1, mods, c) }
        }
        Key::F(n @ 5..=12) => {
            let code = [15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5];
            tilde(code, mods)
        }
        Key::F(_) => return None,
    };
    Some(bytes)
}

fn encode_char(c: char, mods: Mods) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(4);
    if mods.alt {
        out.push(0x1b);
    }
    if mods.ctrl {
        out.push(ctrl_byte(c)?);
    } else {
        let mut buf = [0; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    Some(out)
}

/// The C0 control byte for Ctrl+`c`, following xterm.
fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        ' ' | '@' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        '/' => 0x1f,
        _ => return None,
    })
}

fn csi_mod(n: u32, mods: Mods, final_byte: u8) -> Vec<u8> {
    let mut v = format!("\x1b[{n};{}", mods.param()).into_bytes();
    v.push(final_byte);
    v
}

fn tilde(n: u32, mods: Mods) -> Vec<u8> {
    if mods.is_empty() {
        format!("\x1b[{n}~").into_bytes()
    } else {
        format!("\x1b[{n};{}~", mods.param()).into_bytes()
    }
}

fn cursor(c: u8, mods: Mods, app_cursor: bool) -> Vec<u8> {
    match (mods.is_empty(), app_cursor) {
        (true, true) => vec![0x1b, b'O', c],
        (true, false) => vec![0x1b, b'[', c],
        (false, _) => csi_mod(1, mods, c),
    }
}

/// Kitty keyboard protocol, "disambiguate escape codes" level: keys that are
/// ambiguous in legacy encoding are sent as `CSI codepoint ; mods u`.
/// Returns `None` to fall back to the legacy encoding (which the protocol
/// specifies for unmodified text and for the cursor/function keys).
fn encode_kitty(key: Key, mods: Mods) -> Option<Vec<u8>> {
    let code = match key {
        Key::Escape => 27,
        Key::Enter | Key::Tab | Key::Backspace if mods.is_empty() => return None,
        Key::Enter => 13,
        Key::Tab => 9,
        Key::Backspace => 127,
        Key::Char(c) if mods.ctrl || mods.alt || mods.meta => c.to_lowercase().next()? as u32,
        _ => return None,
    };
    let mods = if let Key::Char(c) = key {
        // Shift is already folded into the character itself.
        Mods { shift: mods.shift && !c.is_uppercase(), ..mods }
    } else {
        mods
    };
    Some(if mods.is_empty() {
        format!("\x1b[{code}u").into_bytes()
    } else {
        format!("\x1b[{code};{}u", mods.param()).into_bytes()
    })
}

/// win32-input-mode: `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _` for key down then
/// key up, understood by ConPTY. Returns `None` for keys without a mapping.
pub fn encode_win32(key: Key, mods: Mods) -> Option<Vec<u8>> {
    const SHIFT: u32 = 0x10;
    const LEFT_CTRL: u32 = 0x08;
    const LEFT_ALT: u32 = 0x02;
    const ENHANCED: u32 = 0x100;

    // (virtual key, scan code, unicode char, enhanced)
    let (vk, sc, mut uc, enhanced): (u32, u32, u32, bool) = match key {
        Key::Backspace => (0x08, 0x0E, 0x08, false),
        Key::Tab => (0x09, 0x0F, 0x09, false),
        Key::Enter => (0x0D, 0x1C, 0x0D, false),
        Key::Escape => (0x1B, 0x01, 0x1B, false),
        Key::PageUp => (0x21, 0x49, 0, true),
        Key::PageDown => (0x22, 0x51, 0, true),
        Key::End => (0x23, 0x4F, 0, true),
        Key::Home => (0x24, 0x47, 0, true),
        Key::Left => (0x25, 0x4B, 0, true),
        Key::Up => (0x26, 0x48, 0, true),
        Key::Right => (0x27, 0x4D, 0, true),
        Key::Down => (0x28, 0x50, 0, true),
        Key::Insert => (0x2D, 0x52, 0, true),
        Key::Delete => (0x2E, 0x53, 0, true),
        Key::F(n @ 1..=10) => (0x6F + n as u32, 0x3A + n as u32, 0, false),
        Key::F(11) => (0x7A, 0x57, 0, false),
        Key::F(12) => (0x7B, 0x58, 0, false),
        Key::F(_) => return None,
        Key::Char(' ') => (0x20, 0x39, 0x20, false),
        Key::Char(c) => {
            let lower = c.to_ascii_lowercase();
            let (vk, sc) = char_vk_scan(lower)?;
            (vk, sc, c as u32, false)
        }
    };

    if mods.ctrl {
        uc = match key {
            Key::Char(c) if c.is_ascii_alphabetic() => (c.to_ascii_lowercase() as u32) - 0x60,
            Key::Backspace => 0x7F,
            Key::Char(' ') => 0,
            Key::Char(_) => 0,
            _ => uc,
        };
    }

    let mut state = 0;
    if mods.shift {
        state |= SHIFT;
    }
    if mods.ctrl {
        state |= LEFT_CTRL;
    }
    if mods.alt {
        state |= LEFT_ALT;
    }
    if enhanced {
        state |= ENHANCED;
    }
    Some(format!("\x1b[{vk};{sc};{uc};1;{state};1_\x1b[{vk};{sc};{uc};0;{state};1_").into_bytes())
}

/// Virtual key and (US layout) scan code for letters and digits.
fn char_vk_scan(c: char) -> Option<(u32, u32)> {
    const ROWS: [(&str, u32); 3] = [("qwertyuiop", 0x10), ("asdfghjkl", 0x1E), ("zxcvbnm", 0x2C)];
    if c.is_ascii_lowercase() {
        let sc = ROWS.iter().find_map(|(row, base)| row.find(c).map(|i| base + i as u32))?;
        return Some((c.to_ascii_uppercase() as u32, sc));
    }
    if c.is_ascii_digit() {
        let sc = if c == '0' { 0x0B } else { 0x01 + (c as u32 - '0' as u32) };
        return Some((c as u32, sc));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(key: Key, mods: Mods) -> Vec<u8> {
        encode(key, mods, &EncodeOptions::default()).unwrap()
    }

    fn enc_with(key: Key, mods: Mods, f: impl FnOnce(&mut EncodeOptions)) -> Vec<u8> {
        let mut opts = EncodeOptions::default();
        f(&mut opts);
        encode(key, mods, &opts).unwrap()
    }

    #[test]
    fn plain_text() {
        assert_eq!(enc(Key::Char('a'), Mods::NONE), b"a");
        assert_eq!(enc(Key::Char('A'), Mods::SHIFT), b"A");
        assert_eq!(enc(Key::Char('é'), Mods::NONE), "é".as_bytes());
    }

    #[test]
    fn ctrl_and_alt_chars() {
        assert_eq!(enc(Key::Char('c'), Mods::CTRL), [0x03]);
        assert_eq!(enc(Key::Char(' '), Mods::CTRL), [0x00]);
        assert_eq!(enc(Key::Char('['), Mods::CTRL), [0x1b]);
        assert_eq!(enc(Key::Char('b'), Mods::ALT), b"\x1bb");
        assert_eq!(enc(Key::Char('w'), Mods { ctrl: true, alt: true, ..Mods::NONE }), b"\x1b\x17");
    }

    #[test]
    fn word_editing_unix() {
        assert_eq!(enc(Key::Backspace, Mods::NONE), [0x7f]);
        assert_eq!(enc(Key::Backspace, Mods::CTRL), [0x17]);
        assert_eq!(enc(Key::Backspace, Mods::ALT), b"\x1b\x7f");
        assert_eq!(enc(Key::Delete, Mods::NONE), b"\x1b[3~");
        assert_eq!(enc(Key::Delete, Mods::CTRL), b"\x1bd");
    }

    #[test]
    fn windows_style_sends_key_events() {
        let win = |o: &mut EncodeOptions| o.style = KeyStyle::Windows;
        assert_eq!(enc_with(Key::Backspace, Mods::CTRL, win), b"\x1b[8;14;127;1;8;1_\x1b[8;14;127;0;8;1_");
        assert_eq!(enc_with(Key::Delete, Mods::CTRL, win), b"\x1b[46;83;0;1;264;1_\x1b[46;83;0;0;264;1_");
        assert_eq!(enc_with(Key::Enter, Mods::SHIFT, win), b"\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;16;1_");
        assert_eq!(enc_with(Key::Char('c'), Mods::CTRL, win), b"\x1b[67;46;3;1;8;1_\x1b[67;46;3;0;8;1_");
        // Plain text stays plain.
        assert_eq!(enc_with(Key::Char('a'), Mods::NONE, win), b"a");
    }

    #[test]
    fn enter_variants() {
        assert_eq!(enc(Key::Enter, Mods::NONE), b"\r");
        assert_eq!(enc(Key::Enter, Mods::SHIFT), b"\x1b\r");
        assert_eq!(enc(Key::Enter, Mods::ALT), b"\x1b\r");
        assert_eq!(enc_with(Key::Enter, Mods::SHIFT, |o| o.shift_enter = b"\n".to_vec()), b"\n");
    }

    #[test]
    fn cursor_keys() {
        assert_eq!(enc(Key::Up, Mods::NONE), b"\x1b[A");
        assert_eq!(enc_with(Key::Up, Mods::NONE, |o| o.app_cursor = true), b"\x1bOA");
        assert_eq!(enc(Key::Left, Mods::CTRL), b"\x1b[1;5D");
        assert_eq!(enc(Key::End, Mods::SHIFT), b"\x1b[1;2F");
        assert_eq!(enc(Key::Tab, Mods::SHIFT), b"\x1b[Z");
    }

    #[test]
    fn function_keys() {
        assert_eq!(enc(Key::F(1), Mods::NONE), b"\x1bOP");
        assert_eq!(enc(Key::F(1), Mods::SHIFT), b"\x1b[1;2P");
        assert_eq!(enc(Key::F(5), Mods::NONE), b"\x1b[15~");
        assert_eq!(enc(Key::F(12), Mods::CTRL), b"\x1b[24;5~");
    }

    #[test]
    fn kitty_disambiguate() {
        let kitty = |o: &mut EncodeOptions| o.kitty_disambiguate = true;
        assert_eq!(enc_with(Key::Enter, Mods::SHIFT, kitty), b"\x1b[13;2u");
        assert_eq!(enc_with(Key::Enter, Mods::NONE, kitty), b"\r");
        assert_eq!(enc_with(Key::Escape, Mods::NONE, kitty), b"\x1b[27u");
        assert_eq!(enc_with(Key::Backspace, Mods::CTRL, kitty), b"\x1b[127;5u");
        assert_eq!(enc_with(Key::Char('c'), Mods::CTRL, kitty), b"\x1b[99;5u");
        assert_eq!(enc_with(Key::Char('a'), Mods::NONE, kitty), b"a");
        assert_eq!(enc_with(Key::Up, Mods::NONE, kitty), b"\x1b[A");
    }
}
