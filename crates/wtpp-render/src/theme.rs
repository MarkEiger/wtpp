//! Color scheme resolution for terminal cells.

use wtpp_session::alacritty_terminal::term::color::Colors;
use wtpp_session::alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub selection_background: Rgb,
    /// ANSI 0..=15.
    pub ansi: [Rgb; 16],
    /// Bold text uses the bright ANSI colors.
    pub bold_is_bright: bool,
    /// Bold text uses a bold font.
    pub bold_font: bool,
}

const fn rgb(hex: u32) -> Rgb {
    Rgb { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
}

impl Default for Theme {
    /// Windows Terminal's default scheme, "Campbell".
    fn default() -> Self {
        Theme {
            foreground: rgb(0xCCCCCC),
            background: rgb(0x0C0C0C),
            cursor: rgb(0xFFFFFF),
            selection_background: rgb(0xFFFFFF),
            ansi: [
                rgb(0x0C0C0C), rgb(0xC50F1F), rgb(0x13A10E), rgb(0xC19C00),
                rgb(0x0037DA), rgb(0x881798), rgb(0x3A96DD), rgb(0xCCCCCC),
                rgb(0x767676), rgb(0xE74856), rgb(0x16C60C), rgb(0xF9F1A5),
                rgb(0x3B78FF), rgb(0xB4009E), rgb(0x61D6D6), rgb(0xF2F2F2),
            ],
            bold_is_bright: true,
            bold_font: false,
        }
    }
}

impl Theme {
    /// Resolve a cell color. `overrides` are colors the running app changed
    /// via OSC 4/10/11, which take precedence over the scheme.
    pub fn resolve(&self, color: Color, overrides: &Colors, bold_is_bright: bool) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(i) => {
                let i = if bold_is_bright && i < 8 { i + 8 } else { i };
                overrides[i as usize].unwrap_or_else(|| self.indexed(i))
            }
            Color::Named(named) => {
                let named = if bold_is_bright { named.to_bright() } else { named };
                overrides[named].unwrap_or_else(|| self.named(named))
            }
        }
    }

    fn named(&self, named: NamedColor) -> Rgb {
        let i = named as usize;
        if i < 16 {
            return self.ansi[i];
        }
        match named {
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground),
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            // Dim variants of the 8 ANSI colors.
            other => dim(self.ansi[other as usize - NamedColor::DimBlack as usize]),
        }
    }

    /// xterm 256-color palette: 16 scheme colors, a 6×6×6 cube, 24 grays.
    pub fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb { r: level(i / 36), g: level(i / 6 % 6), b: level(i % 6) }
            }
            232..=255 => {
                let v = 8 + (i - 232) * 10;
                Rgb { r: v, g: v, b: v }
            }
        }
    }
}

pub fn dim(c: Rgb) -> Rgb {
    let f = |v: u8| (v as u16 * 2 / 3) as u8;
    Rgb { r: f(c.r), g: f(c.g), b: f(c.b) }
}

pub fn to_f32(c: Rgb, alpha: f32) -> [f32; 4] {
    // The render target is non-sRGB and Slint composites it as-is, so these
    // stay in sRGB space (like every other terminal blends text).
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, alpha]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_256() {
        let t = Theme::default();
        assert_eq!(t.indexed(16), rgb(0x000000));
        assert_eq!(t.indexed(196), rgb(0xFF0000));
        assert_eq!(t.indexed(231), rgb(0xFFFFFF));
        assert_eq!(t.indexed(232), rgb(0x080808));
        assert_eq!(t.indexed(255), rgb(0xEEEEEE));
    }

    #[test]
    fn bold_brightens_ansi() {
        let t = Theme::default();
        let none = Colors::default();
        assert_eq!(t.resolve(Color::Named(NamedColor::Red), &none, true), t.ansi[9]);
        assert_eq!(t.resolve(Color::Indexed(1), &none, true), t.ansi[9]);
        assert_eq!(t.resolve(Color::Indexed(1), &none, false), t.ansi[1]);
    }
}
