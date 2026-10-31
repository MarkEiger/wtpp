//! Color schemes, in Windows Terminal's `schemes` JSON format.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub const fn hex(v: u32) -> Color {
        Color((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Parse `#RRGGBB` or `#RGB`.
    pub fn parse(s: &str) -> Option<Color> {
        let h = s.trim().strip_prefix('#')?;
        let v = u32::from_str_radix(h, 16).ok()?;
        match h.len() {
            6 => Some(Color::hex(v)),
            3 => {
                let expand = |n: u32| ((n & 0xF) * 0x11) as u8;
                Some(Color(expand(v >> 8), expand(v >> 4), expand(v)))
            }
            _ => None,
        }
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid color {s:?}")))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scheme {
    pub name: String,
    pub foreground: Color,
    pub background: Color,
    #[serde(default = "white")]
    pub cursor_color: Color,
    #[serde(default = "white")]
    pub selection_background: Color,
    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub purple: Color,
    pub cyan: Color,
    pub white: Color,
    pub bright_black: Color,
    pub bright_red: Color,
    pub bright_green: Color,
    pub bright_yellow: Color,
    pub bright_blue: Color,
    pub bright_purple: Color,
    pub bright_cyan: Color,
    pub bright_white: Color,
}

fn white() -> Color {
    Color::hex(0xFFFFFF)
}

/// Names of the 20 editable colors, in [`Scheme::colors`] order.
pub const COLOR_NAMES: [&str; 20] = [
    "Foreground",
    "Background",
    "Cursor",
    "Selection",
    "Black",
    "Red",
    "Green",
    "Yellow",
    "Blue",
    "Purple",
    "Cyan",
    "White",
    "Bright black",
    "Bright red",
    "Bright green",
    "Bright yellow",
    "Bright blue",
    "Bright purple",
    "Bright cyan",
    "Bright white",
];

impl Scheme {
    /// ANSI colors 0..=15.
    pub fn ansi(&self) -> [Color; 16] {
        let c = self.colors();
        std::array::from_fn(|i| c[i + 4])
    }

    /// All colors, in [`COLOR_NAMES`] order.
    pub fn colors(&self) -> [Color; 20] {
        [
            self.foreground,
            self.background,
            self.cursor_color,
            self.selection_background,
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.purple,
            self.cyan,
            self.white,
            self.bright_black,
            self.bright_red,
            self.bright_green,
            self.bright_yellow,
            self.bright_blue,
            self.bright_purple,
            self.bright_cyan,
            self.bright_white,
        ]
    }

    pub fn set_color(&mut self, index: usize, color: Color) {
        let slot = match index {
            0 => &mut self.foreground,
            1 => &mut self.background,
            2 => &mut self.cursor_color,
            3 => &mut self.selection_background,
            4 => &mut self.black,
            5 => &mut self.red,
            6 => &mut self.green,
            7 => &mut self.yellow,
            8 => &mut self.blue,
            9 => &mut self.purple,
            10 => &mut self.cyan,
            11 => &mut self.white,
            12 => &mut self.bright_black,
            13 => &mut self.bright_red,
            14 => &mut self.bright_green,
            15 => &mut self.bright_yellow,
            16 => &mut self.bright_blue,
            17 => &mut self.bright_purple,
            18 => &mut self.bright_cyan,
            19 => &mut self.bright_white,
            _ => return,
        };
        *slot = color;
    }

    fn from_hex(name: &str, fg: u32, bg: u32, cursor: u32, sel: u32, ansi: [u32; 16]) -> Scheme {
        let a = ansi.map(Color::hex);
        Scheme {
            name: name.into(),
            foreground: Color::hex(fg),
            background: Color::hex(bg),
            cursor_color: Color::hex(cursor),
            selection_background: Color::hex(sel),
            black: a[0],
            red: a[1],
            green: a[2],
            yellow: a[3],
            blue: a[4],
            purple: a[5],
            cyan: a[6],
            white: a[7],
            bright_black: a[8],
            bright_red: a[9],
            bright_green: a[10],
            bright_yellow: a[11],
            bright_blue: a[12],
            bright_purple: a[13],
            bright_cyan: a[14],
            bright_white: a[15],
        }
    }
}

pub const DEFAULT_SCHEME: &str = "Campbell";

/// The schemes Windows Terminal ships with.
pub fn builtin_schemes() -> Vec<Scheme> {
    const CAMPBELL: [u32; 16] = [
        0x0C0C0C, 0xC50F1F, 0x13A10E, 0xC19C00, 0x0037DA, 0x881798, 0x3A96DD, 0xCCCCCC, 0x767676, 0xE74856,
        0x16C60C, 0xF9F1A5, 0x3B78FF, 0xB4009E, 0x61D6D6, 0xF2F2F2,
    ];
    const SOLARIZED: [u32; 16] = [
        0x002B36, 0xDC322F, 0x859900, 0xB58900, 0x268BD2, 0xD33682, 0x2AA198, 0xEEE8D5, 0x073642, 0xCB4B16,
        0x586E75, 0x657B83, 0x839496, 0x6C71C4, 0x93A1A1, 0xFDF6E3,
    ];
    const TANGO: [u32; 16] = [
        0x000000, 0xCC0000, 0x4E9A06, 0xC4A000, 0x3465A4, 0x75507B, 0x06989A, 0xD3D7CF, 0x555753, 0xEF2929,
        0x8AE234, 0xFCE94F, 0x729FCF, 0xAD7FA8, 0x34E2E2, 0xEEEEEC,
    ];
    vec![
        Scheme::from_hex("Campbell", 0xCCCCCC, 0x0C0C0C, 0xFFFFFF, 0xFFFFFF, CAMPBELL),
        Scheme::from_hex("Campbell Powershell", 0xCCCCCC, 0x012456, 0xFFFFFF, 0xFFFFFF, CAMPBELL),
        Scheme::from_hex("Dark+", 0xCCCCCC, 0x1E1E1E, 0x808080, 0xFFFFFF, [
            0x000000, 0xCD3131, 0x0DBC79, 0xE5E510, 0x2472C8, 0xBC3FBC, 0x11A8CD, 0xE5E5E5, 0x666666, 0xF14C4C,
            0x23D18B, 0xF5F543, 0x3B8EEA, 0xD670D6, 0x29B8DB, 0xE5E5E5,
        ]),
        Scheme::from_hex("One Half Dark", 0xDCDFE4, 0x282C34, 0xFFFFFF, 0xFFFFFF, [
            0x282C34, 0xE06C75, 0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xDCDFE4, 0x5A6374, 0xE06C75,
            0x98C379, 0xE5C07B, 0x61AFEF, 0xC678DD, 0x56B6C2, 0xDCDFE4,
        ]),
        Scheme::from_hex("One Half Light", 0x383A42, 0xFAFAFA, 0x4F525D, 0x4F525D, [
            0x383A42, 0xE45649, 0x50A14F, 0xC18301, 0x0184BC, 0xA626A4, 0x0997B3, 0xFAFAFA, 0x4F525D, 0xDF6C75,
            0x98C379, 0xE4C07A, 0x61AFEF, 0xC577DD, 0x56B5C1, 0xFFFFFF,
        ]),
        Scheme::from_hex("Solarized Dark", 0x839496, 0x002B36, 0xFFFFFF, 0xFFFFFF, SOLARIZED),
        Scheme::from_hex("Solarized Light", 0x657B83, 0xFDF6E3, 0x002B36, 0x002B36, SOLARIZED),
        Scheme::from_hex("Tango Dark", 0xD3D7CF, 0x000000, 0xFFFFFF, 0xFFFFFF, TANGO),
        Scheme::from_hex("Tango Light", 0x555753, 0xFFFFFF, 0x000000, 0x000000, TANGO),
        Scheme::from_hex("Vintage", 0xC0C0C0, 0x000000, 0xFFFFFF, 0xFFFFFF, [
            0x000000, 0x800000, 0x008000, 0x808000, 0x000080, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0xFF0000,
            0x00FF00, 0xFFFF00, 0x0000FF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_parsing() {
        assert_eq!(Color::parse("#0C0C0C"), Some(Color(12, 12, 12)));
        assert_eq!(Color::parse("#fff"), Some(Color(255, 255, 255)));
        assert_eq!(Color::parse("0C0C0C"), None);
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color(1, 171, 255).to_string(), "#01ABFF");
    }

    #[test]
    fn scheme_json_matches_wt() {
        let campbell = &builtin_schemes()[0];
        let json = serde_json::to_value(campbell).unwrap();
        assert_eq!(json["brightBlack"], "#767676");
        assert_eq!(json["cursorColor"], "#FFFFFF");
        let back: Scheme = serde_json::from_value(json).unwrap();
        assert_eq!(&back, campbell);
    }

    #[test]
    fn set_color_matches_colors_order() {
        let mut s = builtin_schemes()[0].clone();
        for i in 0..20 {
            s.set_color(i, Color(i as u8, 0, 0));
        }
        assert!(s.colors().iter().enumerate().all(|(i, c)| *c == Color(i as u8, 0, 0)));
    }
}
