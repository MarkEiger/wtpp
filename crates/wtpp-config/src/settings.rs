//! `settings.json`: Windows Terminal's format, plus a few wt++ additions.
//!
//! Unknown keys are preserved through load/save so a WT settings file can be
//! used as-is and edited without losing anything.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::actions::Chord;
use crate::keymap::Keymap;
use crate::schemes::{DEFAULT_SCHEME, Scheme, builtin_schemes};

pub const DEFAULT_FONT_FACE: &str = "Cascadia Mono";
pub const DEFAULT_FONT_SIZE: f32 = 12.0;
pub const DEFAULT_PADDING: [f32; 4] = [8.0, 4.0, 8.0, 8.0];
pub const DEFAULT_HISTORY: u32 = 9001;
pub const DEFAULT_CELL_HEIGHT: f32 = 1.2;
/// Marks profiles wt++ found on the machine (shells, WSL distros).
pub const DISCOVERED_SOURCE: &str = "wtpp.discovered";
/// Marks the stand-in shell used when no profile is visible; never saved.
pub const FALLBACK_SOURCE: &str = "wtpp.fallback";

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LaunchMode {
    #[default]
    Default,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CursorShape {
    #[default]
    Bar,
    Vintage,
    Underscore,
    DoubleUnderscore,
    FilledBox,
    EmptyBox,
}

impl CursorShape {
    pub const ALL: [CursorShape; 6] = [
        CursorShape::Bar,
        CursorShape::Underscore,
        CursorShape::DoubleUnderscore,
        CursorShape::FilledBox,
        CursorShape::EmptyBox,
        CursorShape::Vintage,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CursorShape::Bar => "Bar ( ┃ )",
            CursorShape::Underscore => "Underscore ( ▁ )",
            CursorShape::DoubleUnderscore => "Double underscore ( ‗ )",
            CursorShape::FilledBox => "Filled box ( █ )",
            CursorShape::EmptyBox => "Empty box ( ▯ )",
            CursorShape::Vintage => "Vintage ( ▃ )",
        }
    }
}

/// What happens to a pane when its shell exits (WT's `closeOnExit`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseOnExit {
    /// Close when the shell exits successfully; otherwise keep the output.
    #[default]
    Graceful,
    Always,
    Never,
}

/// App chrome theme (WT's built-in `theme` names).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppTheme {
    #[default]
    Dark,
    Light,
    /// Follow the system's app mode.
    System,
}

/// How wide tabs are (WT's `tabWidthMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TabWidthMode {
    #[default]
    Equal,
    TitleLength,
    /// Inactive tabs show only their icon.
    Compact,
}

/// Graphics API used for rendering; applies on restart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphicsApi {
    #[default]
    Automatic,
    Direct3d12,
    Vulkan,
    Metal,
    #[serde(rename = "opengl")]
    OpenGl,
    /// CPU rendering, for broken GPU drivers or remote sessions.
    Software,
}

/// How the terminal bell is shown (WT's `bellStyle`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BellStyle {
    #[default]
    Audible,
    /// Flash the pane and mark the tab.
    Visual,
    All,
    None,
}

/// How "intense" (bold) text is drawn (WT's `intenseTextStyle`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IntenseTextStyle {
    /// Bright colors and bold font.
    All,
    #[default]
    Bright,
    Bold,
    None,
}

impl IntenseTextStyle {
    pub fn bright(self) -> bool {
        matches!(self, IntenseTextStyle::All | IntenseTextStyle::Bright)
    }

    pub fn bold_font(self) -> bool {
        matches!(self, IntenseTextStyle::All | IntenseTextStyle::Bold)
    }
}

/// How a background image is scaled to the pane (WT's `backgroundImageStretchMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundImageStretchMode {
    /// Natural size.
    None,
    /// Stretched to the pane, ignoring the aspect ratio.
    Fill,
    /// As large as fits, keeping the aspect ratio.
    Uniform,
    /// Covers the pane, keeping the aspect ratio (cropped).
    #[default]
    UniformToFill,
}

impl BackgroundImageStretchMode {
    pub const ALL: [Self; 4] = [Self::None, Self::Fill, Self::Uniform, Self::UniformToFill];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Fill => "Fill",
            Self::Uniform => "Uniform",
            Self::UniformToFill => "Uniform to fill",
        }
    }
}

/// Where a background image sits in the pane (WT's `backgroundImageAlignment`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundImageAlignment {
    #[default]
    Center,
    Left,
    Top,
    Right,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl BackgroundImageAlignment {
    pub const ALL: [Self; 9] = [
        Self::Center,
        Self::Left,
        Self::Top,
        Self::Right,
        Self::Bottom,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Center => "Center",
            Self::Left => "Left",
            Self::Top => "Top",
            Self::Right => "Right",
            Self::Bottom => "Bottom",
            Self::TopLeft => "Top left",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomRight => "Bottom right",
        }
    }

    /// -1 left, 0 centre, 1 right.
    pub fn horizontal(self) -> i8 {
        match self {
            Self::Left | Self::TopLeft | Self::BottomLeft => -1,
            Self::Right | Self::TopRight | Self::BottomRight => 1,
            Self::Center | Self::Top | Self::Bottom => 0,
        }
    }

    /// -1 top, 0 centre, 1 bottom.
    pub fn vertical(self) -> i8 {
        match self {
            Self::Top | Self::TopLeft | Self::TopRight => -1,
            Self::Bottom | Self::BottomLeft | Self::BottomRight => 1,
            Self::Center | Self::Left | Self::Right => 0,
        }
    }
}

/// How keys are encoded for this profile's shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyEncoding {
    /// Windows key events for cmd/PowerShell, VT sequences for everything else.
    #[default]
    Auto,
    Unix,
    Windows,
}

/// Terminal padding: left, top, right, bottom. WT writes it as a string
/// ("8, 8, 8, 8", "8, 4" or "8") or a number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Padding(pub [f32; 4]);

impl Padding {
    pub fn parse(s: &str) -> Option<Padding> {
        let parts: Vec<f32> = s.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
        Some(Padding(match parts[..] {
            [a] => [a; 4],
            [x, y] => [x, y, x, y],
            [l, t, r, b] => [l, t, r, b],
            _ => return None,
        }))
    }
}

impl std::fmt::Display for Padding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [l, t, r, b] = self.0;
        write!(f, "{l}, {t}, {r}, {b}")
    }
}

impl Serialize for Padding {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Padding {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::Number(n) => Ok(Padding([n.as_f64().unwrap_or(8.0) as f32; 4])),
            Value::String(s) => Padding::parse(&s).ok_or_else(|| serde::de::Error::custom("invalid padding")),
            _ => Err(serde::de::Error::custom("invalid padding")),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FontSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    /// "normal", "bold", … or a number (100–950).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<FontWeight>,
    /// Line height as a multiple of the font size, e.g. "1.2" (WT writes a string).
    #[serde(skip_serializing_if = "Option::is_none", with = "opt_number_string", default)]
    pub cell_height: Option<f32>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl FontSettings {
    /// `Some(false)` when `features` turns off contextual alternates or
    /// ligatures (WT: `"features": { "calt": 0, "liga": 0 }`).
    pub fn ligatures(&self) -> Option<bool> {
        let features = self.extra.get("features")?.as_object()?;
        let off = |k: &str| features.get(k).and_then(Value::as_i64) == Some(0);
        let on = |k: &str| features.get(k).and_then(Value::as_i64).is_some_and(|v| v != 0);
        if off("calt") || off("liga") {
            Some(false)
        } else if on("calt") || on("liga") {
            Some(true)
        } else {
            None
        }
    }

    pub fn set_ligatures(&mut self, enabled: bool) {
        let features = self.extra.entry("features").or_insert_with(|| Value::Object(Map::new()));
        if let Some(features) = features.as_object_mut() {
            for k in ["calt", "liga"] {
                if enabled {
                    features.remove(k);
                } else {
                    features.insert(k.into(), Value::from(0));
                }
            }
            if features.is_empty() {
                self.extra.remove("features");
            }
        }
    }
}

/// One profile entry (or `profiles.defaults`). Every field is optional:
/// unset fields inherit from the defaults, then from built-in values.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProfileSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commandline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starting_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<FontSettings>,
    /// Pre-1.10 WT spelling of `font.face`; read, never written.
    #[serde(skip_serializing)]
    pub font_face: Option<String>,
    /// Pre-1.10 WT spelling of `font.size`; read, never written.
    #[serde(skip_serializing)]
    pub font_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor_shape: Option<CursorShape>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Padding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_size: Option<u32>,
    #[serde(rename = "wtpp.keyEncoding", skip_serializing_if = "Option::is_none")]
    pub key_encoding: Option<KeyEncoding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_on_exit: Option<CloseOnExit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_color: Option<crate::schemes::Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_title: Option<String>,
    /// Image path, or an emoji / short text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_application_title: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bell_style: Option<BellStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intense_text_style: Option<IntenseTextStyle>,
    /// Background colour opacity, 0–100 (WT writes an integer).
    #[serde(skip_serializing_if = "Option::is_none", with = "opt_number", default)]
    pub opacity: Option<f32>,
    /// Image file drawn behind the text, or "desktopWallpaper".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image: Option<String>,
    /// 0.0–1.0.
    #[serde(skip_serializing_if = "Option::is_none", with = "opt_number", default)]
    pub background_image_opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image_stretch_mode: Option<BackgroundImageStretchMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_image_alignment: Option<BackgroundImageAlignment>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ProfileSettings {
    /// Move legacy `fontFace`/`fontSize` into `font`.
    fn migrate(&mut self) {
        if self.font_face.is_some() || self.font_size.is_some() {
            let font = self.font.get_or_insert_with(Default::default);
            font.face = font.face.take().or(self.font_face.take());
            font.size = font.size.or(self.font_size.take());
        }
    }

    pub fn font_face(&self) -> Option<&str> {
        self.font.as_ref()?.face.as_deref()
    }

    pub fn font_size(&self) -> Option<f32> {
        self.font.as_ref()?.size
    }

    pub fn set_font_face(&mut self, face: Option<String>) {
        self.font.get_or_insert_with(Default::default).face = face;
    }

    pub fn set_font_size(&mut self, size: Option<f32>) {
        self.font.get_or_insert_with(Default::default).size = size;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profiles {
    pub defaults: ProfileSettings,
    pub list: Vec<ProfileSettings>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for Profiles {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // WT accepts either `"profiles": [...]` or `{ "defaults", "list" }`.
        #[derive(Deserialize)]
        #[serde(untagged)]
        #[allow(clippy::large_enum_variant)] // short-lived, only while parsing
        enum Repr {
            List(Vec<ProfileSettings>),
            Object {
                #[serde(default)]
                defaults: ProfileSettings,
                #[serde(default)]
                list: Vec<ProfileSettings>,
                #[serde(flatten)]
                extra: Map<String, Value>,
            },
        }
        Ok(match Repr::deserialize(d)? {
            Repr::List(list) => Profiles { list, ..Default::default() },
            Repr::Object { defaults, list, extra } => Profiles { defaults, list, extra },
        })
    }
}

/// Font weight: WT accepts names or numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontWeight(pub u16);

impl FontWeight {
    pub const NAMES: [(&'static str, u16); 9] = [
        ("thin", 100),
        ("extra-light", 200),
        ("light", 300),
        ("semi-light", 350),
        ("normal", 400),
        ("medium", 500),
        ("semi-bold", 600),
        ("bold", 700),
        ("extra-bold", 800),
    ];
}

impl Serialize for FontWeight {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match FontWeight::NAMES.iter().find(|(_, w)| *w == self.0) {
            Some((name, _)) => s.serialize_str(name),
            None => s.serialize_u16(self.0),
        }
    }
}

impl<'de> Deserialize<'de> for FontWeight {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::Number(n) => Ok(FontWeight(n.as_u64().unwrap_or(400).clamp(100, 950) as u16)),
            Value::String(s) => {
                let s = s.to_ascii_lowercase();
                let named = FontWeight::NAMES.iter().find(|(n, _)| *n == s || n.replace('-', "") == s);
                match (named, s.parse::<u16>()) {
                    (Some((_, w)), _) => Ok(FontWeight(*w)),
                    (None, Ok(w)) => Ok(FontWeight(w.clamp(100, 950))),
                    _ => Err(serde::de::Error::custom(format!("unknown font weight {s:?}"))),
                }
            }
            _ => Err(serde::de::Error::custom("invalid font weight")),
        }
    }
}

/// `Option<f32>` stored as a string ("1.2"), as WT does for cell sizes;
/// numbers are accepted when reading.
mod opt_number_string {
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.collect_str(v),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Number(n) => n.as_f64().map(|v| v as f32),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        })
    }
}

/// `Option<f32>` stored as a JSON number (an integer when whole, as WT
/// writes `opacity`); numeric strings are accepted when reading.
mod opt_number {
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;

    pub fn serialize<S: Serializer>(v: &Option<f32>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) if v.fract() == 0.0 && v.abs() < 1e9 => s.serialize_i64(*v as i64),
            // Through f64 of the shortest f32 text, so 0.3 stays 0.3, not 0.30000001.
            Some(v) => s.serialize_f64(v.to_string().parse().unwrap_or(*v as f64)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
        let v: Option<f32> = match Value::deserialize(d)? {
            Value::Number(n) => n.as_f64().map(|v| v as f32),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        };
        Ok(v.filter(|v| v.is_finite()))
    }
}

/// One entry of `actions`: a command and the key(s) bound to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionEntry {
    pub command: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty", deserialize_with = "one_or_many", serialize_with = "one_or_many_out")]
    pub keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ActionEntry {
    pub fn new(command: Value, chord: Chord) -> ActionEntry {
        ActionEntry { command, keys: vec![chord.to_wt_string()], id: None, extra: Map::new() }
    }
}

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => vec![s],
        Value::Array(a) => a.into_iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => Vec::new(),
    })
}

/// A single key combination is written as a plain string, like WT does.
fn one_or_many_out<S: Serializer>(keys: &[String], s: S) -> Result<S::Ok, S::Error> {
    match keys {
        [one] => s.serialize_str(one),
        many => many.serialize(s),
    }
}

/// WT 1.21+ splits bindings out of `actions`: `{ "id": ..., "keys": ... }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeybindingEntry {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    #[serde(skip_serializing_if = "is_default")]
    pub launch_mode: LaunchMode,
    pub initial_cols: u16,
    pub initial_rows: u16,
    pub copy_on_select: bool,
    pub word_delimiters: String,
    pub confirm_close_all_tabs: bool,
    pub focus_follow_mouse: bool,
    /// Let apps set the clipboard with OSC 52.
    #[serde(rename = "compatibility.allowOSC52")]
    pub allow_osc52: bool,
    /// Windows: new shells get the environment as it's set now (in System
    /// Properties, or by an installer), not as it was when wt++ started.
    #[serde(rename = "compatibility.reloadEnvironmentVariables")]
    pub reload_environment_variables: bool,
    /// "x,y" in screen pixels; empty means let the system decide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_position: Option<String>,
    /// Restore the last window position and size (wt++ addition).
    #[serde(rename = "wtpp.rememberWindowBounds")]
    pub remember_window_bounds: bool,
    /// The machine's shells have been merged into `profiles` (wt++ addition).
    /// That happens once; afterwards only "Refresh shell list" changes them.
    #[serde(rename = "wtpp.profilesDiscovered")]
    pub profiles_discovered: bool,
    /// Split panes and duplicated tabs start in the current directory, and
    /// PowerShell and WSL's bash are set up to report it (wt++ addition).
    #[serde(rename = "wtpp.inheritWorkingDirectory")]
    pub inherit_working_directory: bool,
    /// Split panes and duplicated tabs of a pane that's in ssh open an ssh to
    /// the same place, and bash and zsh get an `ssh` function that shares
    /// the connection (wt++ addition, see `ssh`).
    #[serde(rename = "wtpp.splitDuplicatesSsh")]
    pub split_duplicates_ssh: bool,
    pub multi_line_paste_warning: bool,
    pub large_paste_warning: bool,
    pub show_tabs_in_titlebar: bool,
    pub theme: AppTheme,
    pub tab_width_mode: TabWidthMode,
    /// Show the tab row even with a single tab (only matters with the native
    /// title bar; tabs in the title bar are always shown).
    pub always_show_tabs: bool,
    /// Right-click in a terminal opens a menu instead of copy/paste.
    #[serde(rename = "experimental.rightClickContextMenu")]
    pub right_click_context_menu: bool,
    #[serde(rename = "rendering.graphicsAPI")]
    pub graphics_api: GraphicsApi,
    pub profiles: Profiles,
    pub schemes: Vec<Scheme>,
    pub actions: Vec<ActionEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub keybindings: Vec<KeybindingEntry>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            schema: None,
            default_profile: None,
            launch_mode: LaunchMode::Default,
            initial_cols: 120,
            initial_rows: 30,
            copy_on_select: false,
            word_delimiters: " /\\()\"'-.,:;<>~!@#$%^&*|+=[]{}~?\u{2502}".into(),
            confirm_close_all_tabs: true,
            focus_follow_mouse: false,
            allow_osc52: true,
            reload_environment_variables: true,
            initial_position: None,
            remember_window_bounds: true,
            profiles_discovered: false,
            inherit_working_directory: true,
            split_duplicates_ssh: true,
            multi_line_paste_warning: true,
            large_paste_warning: true,
            show_tabs_in_titlebar: true,
            theme: AppTheme::Dark,
            tab_width_mode: TabWidthMode::Equal,
            always_show_tabs: true,
            right_click_context_menu: false,
            graphics_api: GraphicsApi::Automatic,
            profiles: Profiles::default(),
            schemes: Vec::new(),
            actions: Vec::new(),
            keybindings: Vec::new(),
            extra: Map::new(),
        }
    }
}

/// A profile with everything inherited and defaulted, ready to use.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedProfile {
    pub guid: String,
    pub name: String,
    pub commandline: String,
    pub starting_directory: Option<String>,
    pub hidden: bool,
    pub source: Option<String>,
    pub color_scheme: String,
    pub font_face: String,
    pub font_size: f32,
    pub cursor_shape: CursorShape,
    pub padding: [f32; 4],
    pub history_size: u32,
    pub key_encoding: KeyEncoding,
    pub close_on_exit: CloseOnExit,
    pub tab_color: Option<crate::schemes::Color>,
    pub tab_title: Option<String>,
    pub icon: Option<String>,
    pub suppress_application_title: bool,
    pub environment: std::collections::BTreeMap<String, String>,
    pub bell_style: BellStyle,
    pub intense_text_style: IntenseTextStyle,
    pub font_weight: u16,
    pub cell_height: f32,
    /// False when `font.features` turns off `calt` or `liga`.
    pub ligatures: bool,
    /// Background colour alpha, 0.0–1.0 (WT's `opacity` / 100).
    pub opacity: f32,
    /// As written (a path, possibly with variables, or "desktopWallpaper").
    pub background_image: Option<String>,
    pub background_image_opacity: f32,
    pub background_image_stretch_mode: BackgroundImageStretchMode,
    pub background_image_alignment: BackgroundImageAlignment,
}

/// A shell found on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredProfile {
    pub name: String,
    pub commandline: String,
    pub icon: Option<String>,
}

/// What [`Settings::refresh_discovered`] changed: profile names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// WSL couldn't be read, and WSL profiles were kept because of that.
    pub wsl_kept: bool,
}

impl RefreshReport {
    /// Add what another refresh changed (the same names only count once).
    pub fn merge(&mut self, other: RefreshReport) {
        for (ours, theirs) in [(&mut self.added, other.added), (&mut self.removed, other.removed)] {
            for name in theirs {
                if !ours.contains(&name) {
                    ours.push(name);
                }
            }
        }
        self.wsl_kept |= other.wsl_kept;
    }

    /// One short paragraph for the user.
    pub fn summary(&self) -> String {
        // Junk names can be whole lines of text, and many of them.
        fn names(list: &[String]) -> String {
            const SHOWN: usize = 4;
            const LONGEST: usize = 32;
            let mut shown: Vec<String> = list
                .iter()
                .take(SHOWN)
                .map(|n| match n.char_indices().nth(LONGEST) {
                    Some((end, _)) => format!("{}…", &n[..end]),
                    None => n.clone(),
                })
                .collect();
            if list.len() > SHOWN {
                shown.push(format!("{} more", list.len() - SHOWN));
            }
            shown.join(", ")
        }
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!("Added {}.", names(&self.added)));
        }
        if !self.removed.is_empty() {
            parts.push(format!("Removed {}.", names(&self.removed)));
        }
        if parts.is_empty() {
            parts.push("No changes.".to_string());
        }
        if self.wsl_kept {
            parts.push("WSL couldn't be read, so existing WSL profiles were kept.".to_string());
        }
        parts.join(" ")
    }
}

/// Could this be the name of a WSL distro? WSL itself only allows these
/// characters, and no line of its usage or error texts passes all of this.
pub fn is_plausible_distro_name(name: &str) -> bool {
    const LONGEST: usize = 64;
    name.len() <= LONGEST
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub fn is_wsl_program(program: &str) -> bool {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
    file == "wsl.exe" || file == "wsl"
}

/// The distro a profile runs, if it runs `wsl.exe`: its `-d`, else its name.
fn wsl_distro_of(p: &ProfileSettings) -> Option<String> {
    let (program, args) = split_commandline_windows(p.commandline.as_deref()?);
    if !is_wsl_program(&program) {
        return None;
    }
    let distro = args.iter().position(|a| a == "-d" || a == "--distribution").and_then(|i| args.get(i + 1));
    distro.cloned().or_else(|| p.name.clone())
}

/// Every command line wt++ has written for a distro: 1.1.2's `--cd ~` (which
/// fails on Windows 10's inbox WSL), then `~` first, then neither.
fn generated_wsl_commandlines(distro: &str) -> [String; 3] {
    let line = |args: &[&str]| join_commandline_windows("wsl.exe", &args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
    [line(&["-d", distro, "--cd", "~"]), line(&["~", "-d", distro]), line(&["-d", distro])]
}

/// Does this entry stand for this shell? Discovered entries may have been
/// renamed or given another command line; the GUID still tells their origin.
/// An older version's command line for the same distro counts as its own.
fn is_same_shell(p: &ProfileSettings, d: &DiscoveredProfile) -> bool {
    if p.name.as_deref() == Some(&d.name)
        || p.commandline.as_deref() == Some(&d.commandline)
        || p.guid.as_deref() == Some(&guid_for(&d.name))
    {
        return true;
    }
    let forms = generated_wsl_commandlines(&d.name);
    forms.contains(&d.commandline) && p.commandline.as_ref().is_some_and(|c| forms.contains(c))
}

/// Is this starting directory a path inside the distro a WSL profile runs?
/// Windows can't start a process there (and `/tmp` could be `C:\tmp`), and
/// `--cd` with such a path fails on inbox wsl.exe: it opens in the Linux home.
pub fn is_wsl_linux_dir(commandline: &str, dir: &str) -> bool {
    let dir = dir.trim();
    is_wsl_program(&split_commandline_windows(commandline).0) && (dir == "~" || dir.starts_with("~/") || dir.starts_with('/'))
}

/// The command line to start a profile with. A starting directory takes over
/// from the Linux home a generated WSL profile opens in, unless it's a path
/// in Linux (see [`is_wsl_linux_dir`]).
pub fn commandline_for_start(commandline: &str, starting_directory: Option<&str>) -> String {
    match starting_directory.map(str::trim) {
        Some(dir) if !dir.is_empty() && !is_wsl_linux_dir(commandline, dir) => without_wsl_home(commandline),
        _ => commandline.to_string(),
    }
}

fn without_wsl_home(commandline: &str) -> String {
    let line = commandline.replace(" --cd ~", "");
    match line.strip_prefix("wsl.exe ~ ") {
        Some(rest) => format!("wsl.exe {rest}"),
        None => line,
    }
}

impl Settings {
    /// Parse settings JSON (comments and trailing commas allowed).
    pub fn parse(text: &str) -> Result<Settings, String> {
        let value: Value = serde_json::from_str(&strip_jsonc(text)).map_err(|e| e.to_string())?;
        let mut settings: Settings = serde_json::from_value(value).map_err(|e| e.to_string())?;
        settings.profiles.defaults.migrate();
        for p in &mut settings.profiles.list {
            p.migrate();
        }
        Ok(settings)
    }

    pub fn to_json(&self) -> String {
        let mut out = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut ser = serde_json::Serializer::with_formatter(&mut out, formatter);
        self.serialize(&mut ser).expect("settings always serialize");
        String::from_utf8(out).expect("serde_json writes UTF-8") + "\n"
    }

    /// Load from `path`. A missing file gives defaults; a broken file gives
    /// defaults plus the error, so the app still starts.
    pub fn load(path: &Path) -> (Settings, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match Settings::parse(&text) {
                Ok(s) => (s, None),
                Err(e) => (Settings::default(), Some(format!("{}: {e}", path.display()))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
            Err(e) => (Settings::default(), Some(format!("{}: {e}", path.display()))),
        }
    }

    /// Write atomically (temp file + rename) so a crash can't truncate it.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(&tmp, path)
    }

    /// If `path` holds something that doesn't parse, copy it aside: to
    /// `settings.json.bak`, or a numbered name so no older backup is lost.
    /// For just before the user replaces it; returns where the copy went.
    pub fn backup_if_broken(path: &Path) -> std::io::Result<Option<PathBuf>> {
        match std::fs::read_to_string(path) {
            Ok(text) if Settings::parse(&text).is_ok() => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            _ => {}
        }
        let backup = path.with_extension("json.bak");
        let free = std::iter::once(backup.clone())
            .chain((1..).map(|n| backup.with_extension(format!("bak.{n}"))))
            .find(|p| !p.exists())
            .expect("an endless list");
        std::fs::copy(path, &free)?;
        Ok(Some(free))
    }

    /// Bring the discovered profiles in line with the shells found on this
    /// machine, keeping the user's changes to them. Entries that aren't ours
    /// (made by the user, imported from WT) are never touched. `wsl_known` is
    /// false when WSL couldn't be asked: its distros may still be there.
    pub fn refresh_discovered(&mut self, found: &[DiscoveredProfile], wsl_known: bool) -> RefreshReport {
        let mut report = RefreshReport::default();
        let mut removed_ids = Vec::new();
        self.profiles.list.retain(|p| {
            if p.source.as_deref() != Some(DISCOVERED_SOURCE) || found.iter().any(|d| is_same_shell(p, d)) {
                return true;
            }
            if !wsl_known && wsl_distro_of(p).is_some_and(|d| is_plausible_distro_name(&d)) {
                report.wsl_kept = true;
                return true;
            }
            let name = p.name.clone().unwrap_or_else(|| "Profile".into());
            removed_ids.push(p.guid.clone().unwrap_or_else(|| guid_for(&name)));
            removed_ids.push(name.clone());
            report.removed.push(name);
            false
        });
        // Don't leave the default pointing at nothing: the first profile takes over.
        if let Some(wanted) = &self.default_profile
            && removed_ids.iter().any(|id| id.eq_ignore_ascii_case(wanted))
        {
            self.default_profile = None;
        }

        for d in found {
            if let Some(known) = self.profiles.list.iter_mut().find(|p| is_same_shell(p, d)) {
                if known.source.as_deref() == Some(DISCOVERED_SOURCE) {
                    // Profiles found by an older version may lack an icon.
                    if known.icon.is_none() {
                        known.icon = d.icon.clone();
                    }
                    // An older form of ours may not start here. Only those:
                    // anything else is the user's, and so is having removed
                    // the `~` (the plain form is never rewritten).
                    let forms = generated_wsl_commandlines(&d.name);
                    if forms.contains(&d.commandline) && known.commandline.as_ref().is_some_and(|c| forms[..2].contains(c)) {
                        known.commandline = Some(d.commandline.clone());
                    }
                }
            } else {
                self.profiles.list.push(ProfileSettings {
                    guid: Some(guid_for(&d.name)),
                    name: Some(d.name.clone()),
                    commandline: Some(d.commandline.clone()),
                    icon: d.icon.clone(),
                    source: Some(DISCOVERED_SOURCE.into()),
                    ..Default::default()
                });
                report.added.push(d.name.clone());
            }
        }
        report
    }

    /// All schemes: built-ins, overridden/extended by the file's.
    pub fn all_schemes(&self) -> Vec<Scheme> {
        let mut all = builtin_schemes();
        for s in &self.schemes {
            match all.iter_mut().find(|b| b.name == s.name) {
                Some(slot) => *slot = s.clone(),
                None => all.push(s.clone()),
            }
        }
        all
    }

    pub fn scheme(&self, name: &str) -> Scheme {
        let all = self.all_schemes();
        all.iter().find(|s| s.name == name).or_else(|| all.iter().find(|s| s.name == DEFAULT_SCHEME)).cloned().unwrap()
    }

    pub fn resolve(&self, p: &ProfileSettings) -> ResolvedProfile {
        let d = &self.profiles.defaults;
        let name = p.name.clone().unwrap_or_else(|| "Profile".into());
        ResolvedProfile {
            guid: p.guid.clone().unwrap_or_else(|| guid_for(&name)),
            commandline: p.commandline.clone().or_else(|| d.commandline.clone()).unwrap_or_else(default_shell),
            starting_directory: p.starting_directory.clone().or_else(|| d.starting_directory.clone()),
            hidden: p.hidden.unwrap_or(false),
            source: p.source.clone(),
            color_scheme: p.color_scheme.clone().or_else(|| d.color_scheme.clone()).unwrap_or_else(|| DEFAULT_SCHEME.into()),
            font_face: p.font_face().or(d.font_face()).unwrap_or(DEFAULT_FONT_FACE).to_string(),
            font_size: p.font_size().or(d.font_size()).unwrap_or(DEFAULT_FONT_SIZE),
            cursor_shape: p.cursor_shape.or(d.cursor_shape).unwrap_or_default(),
            padding: p.padding.or(d.padding).map_or(DEFAULT_PADDING, |p| p.0),
            history_size: p.history_size.or(d.history_size).unwrap_or(DEFAULT_HISTORY),
            key_encoding: p.key_encoding.or(d.key_encoding).unwrap_or_default(),
            close_on_exit: p.close_on_exit.or(d.close_on_exit).unwrap_or_default(),
            tab_color: p.tab_color.or(d.tab_color),
            tab_title: p.tab_title.clone().or_else(|| d.tab_title.clone()).filter(|t| !t.is_empty()),
            icon: p.icon.clone().or_else(|| d.icon.clone()).filter(|i| !i.trim().is_empty()),
            suppress_application_title: p.suppress_application_title.or(d.suppress_application_title).unwrap_or(false),
            // Defaults' variables, overridden per profile.
            environment: d.environment.iter().chain(p.environment.iter()).flatten().map(|(k, v)| (k.clone(), v.clone())).collect(),
            bell_style: p.bell_style.or(d.bell_style).unwrap_or_default(),
            intense_text_style: p.intense_text_style.or(d.intense_text_style).unwrap_or_default(),
            font_weight: p.font.as_ref().and_then(|f| f.weight).or(d.font.as_ref().and_then(|f| f.weight)).map_or(400, |w| w.0),
            cell_height: p.font.as_ref().and_then(|f| f.cell_height).or(d.font.as_ref().and_then(|f| f.cell_height)).unwrap_or(DEFAULT_CELL_HEIGHT),
            ligatures: p.font.as_ref().and_then(FontSettings::ligatures).or(d.font.as_ref().and_then(FontSettings::ligatures)).unwrap_or(true),
            opacity: p.opacity.or(d.opacity).map_or(1.0, |o| o.clamp(0.0, 100.0) / 100.0),
            background_image: p
                .background_image
                .as_ref()
                .or(d.background_image.as_ref())
                .map(|i| i.trim().to_string())
                .filter(|i| !i.is_empty()),
            background_image_opacity: p.background_image_opacity.or(d.background_image_opacity).map_or(1.0, |o| o.clamp(0.0, 1.0)),
            background_image_stretch_mode: p.background_image_stretch_mode.or(d.background_image_stretch_mode).unwrap_or_default(),
            background_image_alignment: p.background_image_alignment.or(d.background_image_alignment).unwrap_or_default(),
            name,
        }
    }

    /// Whether any profile (or the defaults) asks for a see-through
    /// background, which on some platforms must be decided at startup.
    pub fn wants_transparency(&self) -> bool {
        self.resolve(&self.profiles.defaults).opacity < 1.0 || self.profiles.list.iter().any(|p| self.resolve(p).opacity < 1.0)
    }

    /// Profiles shown in the new-tab menu, in order. Never empty: with
    /// nothing to show (an empty list, everything hidden) there's still the
    /// stand-in shell, so wt++ always opens something.
    pub fn visible_profiles(&self) -> Vec<ResolvedProfile> {
        let visible: Vec<_> = self.profiles.list.iter().map(|p| self.resolve(p)).filter(|p| !p.hidden).collect();
        if visible.is_empty() { vec![self.fallback_profile()] } else { visible }
    }

    /// The system's plain shell, as a profile that only exists in memory.
    pub fn fallback_profile(&self) -> ResolvedProfile {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let (name, program) = if cfg!(windows) {
            ("Command Prompt".to_string(), env("COMSPEC").unwrap_or_else(|| "cmd.exe".into()))
        } else {
            let shell = env("SHELL").unwrap_or_else(|| "/bin/sh".into());
            (shell.rsplit('/').next().unwrap_or("sh").to_string(), shell)
        };
        self.resolve(&ProfileSettings {
            commandline: Some(join_commandline(&program, &[])),
            name: Some(name),
            source: Some(FALLBACK_SOURCE.into()),
            ..Default::default()
        })
    }

    /// Index into [`Settings::visible_profiles`] of the default profile.
    pub fn default_profile_index(&self) -> usize {
        let Some(wanted) = &self.default_profile else { return 0 };
        self.visible_profiles()
            .iter()
            .position(|p| p.guid.eq_ignore_ascii_case(wanted) || &p.name == wanted)
            .unwrap_or(0)
    }

    pub fn keymap(&self) -> (Keymap, Vec<String>) {
        // WT 1.21+: commands carry an `id`; bindings live in `keybindings`.
        let mut entries = self.actions.clone();
        for kb in &self.keybindings {
            if let Some(entry) = kb.id.as_ref().and_then(|id| entries.iter_mut().find(|e| e.id.as_ref() == Some(id))) {
                entry.keys.extend(kb.keys.iter().cloned());
            }
        }
        Keymap::from_user_actions(&entries)
    }

    /// Store `keymap` as the user's bindings (as a diff from the defaults).
    pub fn set_keymap(&mut self, keymap: &Keymap) {
        // Keep commands without keys (palette-only entries) and anything we
        // don't understand; replace everything we generate.
        self.actions.retain(|a| a.keys.is_empty() && a.id.is_some());
        self.keybindings.clear();
        self.actions.extend(keymap.to_user_actions());
    }

    /// Import a Windows Terminal settings file: profiles (matched by name),
    /// color schemes, key bindings and the default profile. Returns how many
    /// key bindings it brought (see `merge_bindings`).
    pub fn import_windows_terminal(&mut self, wt: &Settings) -> usize {
        for p in &wt.profiles.list {
            let Some(name) = &p.name else { continue };
            match self.profiles.list.iter_mut().find(|q| q.name.as_ref() == Some(name)) {
                Some(existing) => {
                    let guid = existing.guid.clone();
                    let source = existing.source.clone();
                    *existing = p.clone();
                    existing.guid = guid.or(p.guid.clone());
                    existing.source = source;
                }
                None if p.commandline.is_some() => self.profiles.list.push(p.clone()),
                // WT's dynamic profiles (no commandline) only make sense with
                // WT's generators; skip ones we didn't discover ourselves.
                None => {}
            }
        }
        self.profiles.defaults = wt.profiles.defaults.clone();
        for s in &wt.schemes {
            self.schemes.retain(|x| x.name != s.name);
            self.schemes.push(s.clone());
        }
        let bindings = self.merge_bindings(wt);
        if let Some(default) = &wt.default_profile {
            let name = wt.profiles.list.iter().find(|p| p.guid.as_ref() == Some(default)).and_then(|p| p.name.clone());
            self.default_profile = name.or(Some(default.clone()));
        }
        self.copy_on_select = wt.copy_on_select;
        self.word_delimiters = wt.word_delimiters.clone();
        self.launch_mode = wt.launch_mode;
        self.initial_cols = wt.initial_cols;
        self.initial_rows = wt.initial_rows;
        bindings
    }

    /// Add `wt`'s key bindings to ours, like its profiles: what it binds (or
    /// unbinds) wins for those keys, and the keys it says nothing about keep
    /// whatever we have, our own bindings included. (Taking its keymap whole
    /// dropped ours, all of them when it had none.) Returns how many it had.
    fn merge_bindings(&mut self, wt: &Settings) -> usize {
        let defaults = Keymap::defaults();
        let (theirs, _) = wt.keymap();
        let (mut ours, _) = self.keymap();
        let mut count = 0;
        for d in &defaults.bindings {
            if !theirs.bindings.iter().any(|b| b.chord == d.chord) {
                ours.bindings.retain(|b| b.chord != d.chord);
                count += 1;
            }
        }
        for b in theirs.bindings.iter().filter(|b| !defaults.bindings.contains(b)) {
            ours.bind(b.chord, b.action.clone());
            count += 1;
        }
        self.set_keymap(&ours);
        count
    }
}

fn default_shell() -> String {
    if cfg!(windows) { "powershell.exe".into() } else { std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()) }
}

/// Stable GUID for a discovered profile, derived from its name (FNV-1a).
pub fn guid_for(name: &str) -> String {
    let fnv = |seed: u64| {
        name.bytes().fold(seed, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
    };
    let (a, b) = (fnv(0xcbf29ce484222325), fnv(0x84222325cbf29ce4));
    format!(
        "{{{:08x}-{:04x}-{:04x}-{:04x}-{:012x}}}",
        a >> 32,
        (a >> 16) & 0xFFFF,
        a & 0xFFFF,
        b >> 48,
        b & 0xFFFF_FFFF_FFFF
    )
}

/// Split a profile's command line into program and arguments, by the
/// rules of the platform's shells: Windows' on Windows (see
/// [`split_commandline_windows`]), POSIX shell words elsewhere (see
/// [`split_commandline_unix`]).
pub fn split_commandline(s: &str) -> (String, Vec<String>) {
    if cfg!(windows) { split_commandline_windows(s) } else { split_commandline_unix(s) }
}

/// Windows rules (as WT reads `commandline`): whitespace separates, double
/// quotes group, backslashes are ordinary characters.
pub fn split_commandline_windows(s: &str) -> (String, Vec<String>) {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut started = false;
    for c in s.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    parts.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        parts.push(cur);
    }
    let mut parts = parts.into_iter();
    (parts.next().unwrap_or_default(), parts.collect())
}

/// POSIX shell words, as `sh` would split them, without expanding anything
/// (no variables, `~` or globs): single quotes keep everything; in double
/// quotes a backslash escapes only `$`, `` ` ``, `"`, `\` and a newline;
/// outside quotes it escapes any character. An unterminated quote runs to
/// the end.
pub fn split_commandline_unix(s: &str) -> (String, Vec<String>) {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                started = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                started = true;
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.peek() {
                            Some('\n') => {
                                chars.next();
                            }
                            Some(&e @ ('$' | '`' | '"' | '\\')) => {
                                chars.next();
                                cur.push(e);
                            }
                            _ => cur.push('\\'),
                        },
                        c => cur.push(c),
                    }
                }
            }
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(e) => {
                    cur.push(e);
                    started = true;
                }
                None => {
                    cur.push('\\');
                    started = true;
                }
            },
            c if c.is_whitespace() => {
                if started {
                    parts.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        parts.push(cur);
    }
    let mut parts = parts.into_iter();
    (parts.next().unwrap_or_default(), parts.collect())
}

/// Quote program and arguments back into a command line that
/// [`split_commandline`] gives back.
pub fn join_commandline(program: &str, args: &[String]) -> String {
    if cfg!(windows) { join_commandline_windows(program, args) } else { join_commandline_unix(program, args) }
}

/// See [`split_commandline_windows`].
pub fn join_commandline_windows(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(|p| if p.contains(char::is_whitespace) || p.is_empty() { format!("\"{p}\"") } else { p.to_string() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// See [`split_commandline_unix`]: single quotes around anything but plain
/// words.
pub fn join_commandline_unix(program: &str, args: &[String]) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@%+=:,./-~".contains(c);
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(|p| if !p.is_empty() && p.chars().all(plain) { p.to_string() } else { format!("'{}'", p.replace('\'', r"'\''")) })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Expand Windows-style `%VAR%` references (and a leading `~` to the home
/// directory), as WT does for command lines and starting directories.
pub fn expand_env(s: &str) -> String {
    expand_env_with(s, |name| std::env::var(name).ok())
}

/// [`expand_env`] with the variables from `var`.
pub fn expand_env_with(s: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match var(name) {
                    Some(v) => out.push_str(&v),
                    None => out.push_str(&rest[start..start + end + 2]),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if let Some(tail) = out.strip_prefix('~')
        && (tail.is_empty() || tail.starts_with(['/', '\\']))
        && let Some(home) = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
    {
        return format!("{home}{tail}");
    }
    out
}

/// Remove `//` and `/* */` comments and trailing commas, outside strings.
pub fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            (',', _) => {
                // Drop the comma if the next non-space char closes a container.
                let rest: String = chars.clone().collect();
                let next = strip_leading_comments(&rest);
                if !next.starts_with(['}', ']']) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn strip_leading_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("//") {
            s = rest.split_once('\n').map_or("", |(_, r)| r);
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r);
        } else {
            return s;
        }
    }
}

/// Where wt++ keeps its settings.
pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

fn config_dir() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        env("LOCALAPPDATA").unwrap_or_else(|| PathBuf::from(".")).join("wtpp")
    } else if cfg!(target_os = "macos") {
        env("HOME").unwrap_or_default().join("Library/Application Support/wtpp")
    } else {
        env("XDG_CONFIG_HOME").unwrap_or_else(|| env("HOME").unwrap_or_default().join(".config")).join("wtpp")
    }
}

/// Windows Terminal's settings file (store, preview or unpackaged install).
pub fn find_windows_terminal_settings() -> Option<PathBuf> {
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    [
        "Packages/Microsoft.WindowsTerminal_8wekyb3d8bbwe/LocalState/settings.json",
        "Packages/Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe/LocalState/settings.json",
        "Microsoft/Windows Terminal/settings.json",
    ]
    .iter()
    .map(|p| local.join(p))
    .find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Action;

    fn found(name: &str, commandline: &str) -> DiscoveredProfile {
        DiscoveredProfile { name: name.into(), commandline: commandline.into(), icon: None }
    }

    fn names(s: &Settings) -> Vec<&str> {
        s.profiles.list.iter().filter_map(|p| p.name.as_deref()).collect()
    }

    const WT_SAMPLE: &str = r##"
    // This file was initially generated by Windows Terminal
    {
        "$schema": "https://aka.ms/terminal-profiles-schema",
        "defaultProfile": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}",
        "copyOnSelect": true,
        "theme": "light",
        "launchWindowAnimation": true, /* unknown to us, must survive */
        "profiles": {
            "defaults": { "font": { "face": "Cascadia Code", "size": 11 }, "padding": "4" },
            "list": [
                { "guid": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}", "name": "Windows PowerShell",
                  "commandline": "%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe", "hidden": false },
                { "guid": "{0caa0dad-35be-5f56-a8ff-afceeeaa6101}", "name": "Command Prompt",
                  "commandline": "%SystemRoot%\\System32\\cmd.exe", "fontFace": "Consolas", "colorScheme": "Vintage" },
                { "guid": "{2c4de342-38b7-51cf-b940-2309a097f518}", "name": "Ubuntu", "source": "Windows.Terminal.Wsl", "hidden": false },
            ],
        },
        "schemes": [
            { "name": "Mine", "foreground": "#FFFFFF", "background": "#000000",
              "black": "#000000", "red": "#FF0000", "green": "#00FF00", "yellow": "#FFFF00",
              "blue": "#0000FF", "purple": "#FF00FF", "cyan": "#00FFFF", "white": "#FFFFFF",
              "brightBlack": "#000000", "brightRed": "#FF0000", "brightGreen": "#00FF00", "brightYellow": "#FFFF00",
              "brightBlue": "#0000FF", "brightPurple": "#FF00FF", "brightCyan": "#00FFFF", "brightWhite": "#FFFFFF" }
        ],
        "actions": [
            { "command": { "action": "copy", "singleLine": false }, "id": "User.copy.644BA8F2" },
            { "command": "unbound", "keys": "ctrl+v" },
            { "command": { "action": "splitPane", "split": "auto", "splitMode": "duplicate" }, "id": "User.split" }
        ],
        "keybindings": [
            { "id": "User.copy.644BA8F2", "keys": "ctrl+c" },
            { "id": "User.split", "keys": "alt+shift+s" }
        ]
    }"##;

    #[test]
    fn parses_windows_terminal_file() {
        let s = Settings::parse(WT_SAMPLE).unwrap();
        assert!(s.copy_on_select);
        assert_eq!(s.theme, AppTheme::Light);
        assert_eq!(s.extra.get("launchWindowAnimation"), Some(&Value::from(true)));
        assert_eq!(s.profiles.list.len(), 3);
        assert_eq!(s.profiles.list[1].font_face(), Some("Consolas"));

        let cmd = s.resolve(&s.profiles.list[1]);
        assert_eq!(cmd.color_scheme, "Vintage");
        assert_eq!(cmd.font_size, 11.0);
        assert_eq!(cmd.padding, [4.0; 4]);

        let (keymap, warnings) = s.keymap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let alt_shift = wtpp_input::Mods { alt: true, shift: true, ..Default::default() };
        assert_eq!(
            keymap.lookup(wtpp_input::Key::Char('S'), alt_shift),
            Some(&Action::SplitPane { side: None, duplicate: true })
        );
        assert_eq!(keymap.lookup(wtpp_input::Key::Char('v'), wtpp_input::Mods::CTRL), None);
    }

    #[test]
    fn save_round_trip_keeps_unknown_keys() {
        let s = Settings::parse(WT_SAMPLE).unwrap();
        let again = Settings::parse(&s.to_json()).unwrap();
        assert_eq!(again, s);
        assert!(s.to_json().contains("\"launchWindowAnimation\": true"));
        assert!(Settings { actions: vec![ActionEntry::new(Value::from("paste"), Chord::parse("ctrl+v").unwrap())], ..Default::default() }
            .to_json()
            .contains("\"keys\": \"ctrl+v\""));
        // Legacy spelling is migrated, not duplicated.
        assert!(!s.to_json().contains("fontFace"));
    }

    #[test]
    fn import_from_windows_terminal() {
        let wt = Settings::parse(WT_SAMPLE).unwrap();
        let mut ours = Settings::default();
        ours.refresh_discovered(&[found("Windows PowerShell", "powershell.exe"), found("Command Prompt", "cmd.exe")], true);
        ours.import_windows_terminal(&wt);

        // Matched by name, our GUID kept, settings imported; WSL generator profile skipped.
        assert_eq!(ours.profiles.list.len(), 2);
        assert_eq!(ours.profiles.list[0].guid, Some(guid_for("Windows PowerShell")));
        assert_eq!(ours.profiles.list[1].color_scheme.as_deref(), Some("Vintage"));
        assert_eq!(ours.default_profile.as_deref(), Some("Windows PowerShell"));
        assert!(ours.all_schemes().iter().any(|s| s.name == "Mine"));
        let (keymap, _) = ours.keymap();
        assert_eq!(keymap.lookup(wtpp_input::Key::Char('v'), wtpp_input::Mods::CTRL), None);
    }

    /// Importing adds WT's key bindings to ours; it doesn't take ours away,
    /// least of all when WT has none. (The PC defaults' keys.)
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn import_keeps_our_key_bindings() {
        use crate::actions::Action;
        use wtpp_input::{Key, Mods};
        let ctrl_alt = Mods { ctrl: true, alt: true, ..Mods::NONE };
        let ctrl_shift = Mods { ctrl: true, shift: true, ..Mods::NONE };
        let mut ours = Settings::default();
        let (mut keymap, _) = ours.keymap();
        keymap.bind(Chord::parse("ctrl+alt+k").unwrap(), Action::ClosePane);
        keymap.bindings.retain(|b| b.chord != Chord::parse("ctrl+shift+f").unwrap());
        ours.set_keymap(&keymap);
        let saved = Settings::parse(&ours.to_json()).unwrap();
        // (Those keys are the defaults' to start with.)
        assert_eq!(Keymap::defaults().lookup(Key::Char('F'), ctrl_shift), Some(&Action::Find));
        assert!(Keymap::defaults().lookup(Key::Char('T'), ctrl_shift).is_some());
        assert!(Keymap::defaults().lookup(Key::Char('W'), ctrl_shift).is_some());

        // A WT file with an empty `actions` list, or none.
        for wt in [r#"{ "actions": [], "profiles": { "list": [] } }"#, r#"{ "profiles": [] }"#] {
            let mut s = saved.clone();
            assert_eq!(s.import_windows_terminal(&Settings::parse(wt).unwrap()), 0);
            let (keymap, _) = Settings::parse(&s.to_json()).unwrap().keymap();
            assert_eq!(keymap.lookup(Key::Char('k'), ctrl_alt), Some(&Action::ClosePane), "{wt}");
            assert_eq!(keymap.lookup(Key::Char('F'), ctrl_shift), None, "{wt}");
            assert_eq!(keymap, saved.keymap().0, "{wt}");
        }

        // WT's own: a new key, a key we use for something else, an unbind.
        let wt = Settings::parse(
            r#"{ "actions": [
                { "command": "paste", "keys": "ctrl+alt+v" },
                { "command": "find", "keys": "ctrl+alt+k" },
                { "command": "unbound", "keys": "ctrl+shift+t" }
            ] }"#,
        )
        .unwrap();
        let mut s = saved.clone();
        assert_eq!(s.import_windows_terminal(&wt), 3);
        let (keymap, _) = Settings::parse(&s.to_json()).unwrap().keymap();
        assert_eq!(keymap.lookup(Key::Char('v'), ctrl_alt), Some(&Action::Paste));
        assert_eq!(keymap.lookup(Key::Char('k'), ctrl_alt), Some(&Action::Find));
        assert_eq!(keymap.lookup(Key::Char('T'), ctrl_shift), None);
        // The rest as it was, ours and the defaults.
        assert_eq!(keymap.lookup(Key::Char('F'), ctrl_shift), None);
        assert_eq!(keymap.lookup(Key::Char('W'), ctrl_shift), Keymap::defaults().lookup(Key::Char('W'), ctrl_shift));
        assert_eq!(keymap.bindings.len(), saved.keymap().0.bindings.len() + 1 - 1);
    }

    /// What 1.1.2 wrote where `wsl -l -q` printed its usage: a profile per line.
    const JUNK_1_1_2: &str = r##"{
        "defaultProfile": "Windows PowerShell",
        "wtpp.rememberWindowBounds": false,
        "launchWindowAnimation": true,
        "profiles": { "defaults": {}, "list": [
            { "guid": "{1}", "name": "Windows PowerShell", "commandline": "powershell.exe -NoLogo", "source": "wtpp.discovered", "colorScheme": "Vintage" },
            { "guid": "{2}", "name": "Command Prompt", "commandline": "C:\\Windows\\system32\\cmd.exe", "source": "wtpp.discovered" },
            { "guid": "{3}", "name": "Usage: wsl.exe [Argument] [Options...] [CommandLine]", "commandline": "wsl.exe -d \"Usage: wsl.exe [Argument] [Options...] [CommandLine]\" --cd ~", "source": "wtpp.discovered" },
            { "guid": "{4}", "name": "Arguments for running Linux binaries:", "commandline": "wsl.exe -d \"Arguments for running Linux binaries:\" --cd ~", "source": "wtpp.discovered" },
            { "guid": "{5}", "name": "--exec, -e <CommandLine>", "commandline": "wsl.exe -d \"--exec, -e <CommandLine>\" --cd ~", "source": "wtpp.discovered" },
            { "guid": "{6}", "name": "--", "commandline": "wsl.exe -d -- --cd ~", "source": "wtpp.discovered" },
            { "guid": "{7}", "name": "Debian", "commandline": "wsl.exe -d Debian --cd ~", "source": "wtpp.discovered" },
            { "guid": "{8}", "name": "Usage: my own notes", "commandline": "wsl.exe -d \"not a distro\"" },
            { "guid": "{9}", "name": "Ubuntu", "source": "Windows.Terminal.Wsl" }
        ] }
    }"##;

    fn junk_names() -> [&'static str; 4] {
        [
            "Usage: wsl.exe [Argument] [Options...] [CommandLine]",
            "Arguments for running Linux binaries:",
            "--exec, -e <CommandLine>",
            "--",
        ]
    }

    #[test]
    fn upgrade_from_1_1_2_cleans_junk() {
        let mut s = Settings::parse(JUNK_1_1_2).unwrap();
        assert!(!s.profiles_discovered, "flag absent means not discovered yet");
        let done = JUNK_1_1_2.replacen('{', r#"{ "wtpp.profilesDiscovered": true,"#, 1);
        assert!(Settings::parse(&done).unwrap().profiles_discovered);
        let shells = [found("Windows PowerShell", "powershell.exe -NoLogo"), found("Command Prompt", "cmd.exe")];

        // WSL couldn't be read (it printed the usage again): junk goes, Debian stays.
        let report = s.refresh_discovered(&shells, false);
        assert_eq!(report.removed, junk_names());
        assert_eq!(report.added, Vec::<String>::new());
        assert_eq!(names(&s), ["Windows PowerShell", "Command Prompt", "Debian", "Usage: my own notes", "Ubuntu"]);
        // Customized and found by commandline or name: untouched.
        assert_eq!(s.profiles.list[0].color_scheme.as_deref(), Some("Vintage"));
        assert_eq!(s.profiles.list[1].commandline.as_deref(), Some(r"C:\Windows\system32\cmd.exe"));
        assert_eq!(s.extra.get("launchWindowAnimation"), Some(&Value::from(true)));
    }

    #[test]
    fn known_wsl_answer_removes_what_is_gone() {
        let mut s = Settings::parse(JUNK_1_1_2).unwrap();
        let report = s.refresh_discovered(&[found("Windows PowerShell", "powershell.exe -NoLogo"), found("Kali", "wsl.exe ~ -d Kali")], true);
        assert_eq!(report.added, ["Kali"]);
        assert!(report.removed.contains(&"Debian".to_string()));
        assert!(report.removed.contains(&"Command Prompt".to_string()));
        // Not ours: stays, however junk it looks.
        assert_eq!(names(&s), ["Windows PowerShell", "Usage: my own notes", "Ubuntu", "Kali"]);
    }

    #[test]
    fn unknown_wsl_only_protects_wsl_entries() {
        let mut s = Settings::default();
        let shells = [found("Git Bash", "bash.exe"), found("Debian", "wsl.exe ~ -d Debian")];
        s.refresh_discovered(&shells, true);
        let report = s.refresh_discovered(&[], false);
        assert_eq!(report.removed, ["Git Bash"]);
        assert!(report.wsl_kept);
        assert_eq!(names(&s), ["Debian"]);
    }

    #[test]
    fn wsl_note_only_when_a_wsl_profile_was_kept() {
        // Nothing of WSL here, and wsl.exe says so in its usage text.
        let mut s = Settings::default();
        s.refresh_discovered(&[found("Command Prompt", "cmd.exe")], true);
        let report = s.refresh_discovered(&[found("Command Prompt", "cmd.exe")], false);
        assert!(!report.wsl_kept);
        assert_eq!(report.summary(), "No changes.");
        // Only junk to remove: nothing kept either.
        s.profiles.list.push(old_entry("Options:", "wsl.exe -d Options: --cd ~"));
        let report = s.refresh_discovered(&[], false);
        assert!(!report.wsl_kept);
        assert!(!report.summary().contains("WSL"), "{}", report.summary());
    }

    #[test]
    fn renamed_wsl_entry_counts_by_its_distro() {
        let mut s = Settings::default();
        s.refresh_discovered(&[found("Debian", "wsl.exe ~ -d Debian")], true);
        s.profiles.list[0].name = Some("My Debian box".into());
        assert_eq!(s.refresh_discovered(&[], false), RefreshReport { wsl_kept: true, ..Default::default() });
        assert_eq!(names(&s), ["My Debian box"]);
        // A plain `wsl.exe` has no `-d`: its name is all there is.
        s.profiles.list[0].commandline = Some(r"C:\Windows\System32\WSL.EXE".into());
        assert_eq!(s.refresh_discovered(&[], false).removed, ["My Debian box"]);
    }

    #[test]
    fn refresh_keeps_customizations() {
        let shells = [found("bash", "/bin/bash -l"), found("zsh", "/bin/zsh -l"), found("fish", "/bin/fish -l")];
        let mut s = Settings::default();
        assert_eq!(s.refresh_discovered(&shells, true).added, ["bash", "zsh", "fish"]);
        assert_eq!(s.visible_profiles()[0].commandline, "/bin/bash -l");

        s.profiles.list[0].color_scheme = Some("Vintage".into());
        s.profiles.list[0].set_font_size(Some(20.0));
        s.profiles.list[1].hidden = Some(true);
        // Renamed, found by its commandline.
        s.profiles.list[2].name = Some("The fish".into());
        let before = s.clone();
        assert_eq!(s.refresh_discovered(&shells, true), RefreshReport::default());
        assert_eq!(s, before);

        // Renamed and given another command line: still the same shell.
        s.profiles.list[2].commandline = Some("/opt/fish".into());
        assert_eq!(s.refresh_discovered(&shells, true), RefreshReport::default());
    }

    #[test]
    fn refresh_brings_back_a_deleted_profile() {
        let shells = [found("bash", "/bin/bash -l"), found("zsh", "/bin/zsh -l")];
        let mut s = Settings::default();
        s.refresh_discovered(&shells, true);
        s.profiles_discovered = true;
        s.profiles.list.remove(1);
        // Relaunching doesn't run discovery at all (the flag is set)...
        let reloaded = Settings::parse(&s.to_json()).unwrap();
        assert!(reloaded.profiles_discovered);
        assert_eq!(names(&reloaded), ["bash"]);
        // ...refreshing does.
        assert_eq!(s.refresh_discovered(&shells, true).added, ["zsh"]);
    }

    #[test]
    fn refresh_fills_in_a_missing_icon() {
        let mut s = Settings::default();
        s.refresh_discovered(&[found("bash", "/bin/bash -l")], true);
        let with_icon = DiscoveredProfile { icon: Some("x.ico".into()), ..found("bash", "/bin/bash -l") };
        assert_eq!(s.refresh_discovered(&[with_icon], true), RefreshReport::default());
        assert_eq!(s.profiles.list[0].icon.as_deref(), Some("x.ico"));
    }

    /// A 1.1.2 entry, as it wrote them.
    fn old_entry(name: &str, commandline: &str) -> ProfileSettings {
        ProfileSettings {
            guid: Some("{from-1.1.2}".into()),
            name: Some(name.into()),
            commandline: Some(commandline.into()),
            source: Some(DISCOVERED_SOURCE.into()),
            color_scheme: Some("Vintage".into()),
            hidden: Some(true),
            ..Default::default()
        }
    }

    fn with_entries(entries: Vec<ProfileSettings>) -> Settings {
        let mut s = Settings::default();
        s.profiles.list = entries;
        s
    }

    #[test]
    fn old_commandlines_of_ours_are_rewritten() {
        for (old, name, new) in [
            ("wsl.exe -d Alpine --cd ~", "Alpine", "wsl.exe ~ -d Alpine"),
            (r#"wsl.exe -d "My Distro" --cd ~"#, "My Distro", r#"wsl.exe ~ -d "My Distro""#),
            // Below build 18362, for `~` as well.
            ("wsl.exe -d Alpine --cd ~", "Alpine", "wsl.exe -d Alpine"),
            ("wsl.exe ~ -d Alpine", "Alpine", "wsl.exe -d Alpine"),
        ] {
            let mut s = with_entries(vec![old_entry(name, old)]);
            assert_eq!(s.refresh_discovered(&[found(name, new)], true), RefreshReport::default());
            assert_eq!(s.profiles.list.len(), 1);
            let p = &s.profiles.list[0];
            assert_eq!(p.commandline.as_deref(), Some(new));
            // All the rest stays.
            assert_eq!(p.name.as_deref(), Some(name));
            assert_eq!(p.guid.as_deref(), Some("{from-1.1.2}"));
            assert_eq!(p.color_scheme.as_deref(), Some("Vintage"));
            assert_eq!(p.hidden, Some(true));
        }
    }

    #[test]
    fn renamed_old_entry_is_recognised_by_its_commandline() {
        // Neither the name nor the GUID tell: only the old form does.
        let mut s = with_entries(vec![old_entry("My box", "wsl.exe -d Alpine --cd ~")]);
        let report = s.refresh_discovered(&[found("Alpine", "wsl.exe ~ -d Alpine")], true);
        assert_eq!(report, RefreshReport::default());
        assert_eq!(names(&s), ["My box"]);
        assert_eq!(s.profiles.list[0].commandline.as_deref(), Some("wsl.exe ~ -d Alpine"));
    }

    #[test]
    fn customized_commandlines_are_left_alone() {
        for custom in [
            "wsl.exe -d Alpine --cd /srv",
            "wsl.exe -d Alpine --cd ~ -u bob",
            "wsl.exe -d Alpine -u bob",
            "wsl -d Alpine --cd ~",
            r"C:\Windows\System32\wsl.exe -d Alpine --cd ~",
            "WSL.EXE -d Alpine --cd ~",
            "wsl.exe --cd ~ -d Alpine",
            "wsl.exe -d Other --cd ~",
            // No `~`: someone may have removed it on purpose.
            "wsl.exe -d Alpine",
        ] {
            let mut s = with_entries(vec![old_entry("Alpine", custom)]);
            let report = s.refresh_discovered(&[found("Alpine", "wsl.exe ~ -d Alpine")], true);
            assert_eq!(report, RefreshReport::default(), "{custom}");
            assert_eq!(s.profiles.list[0].commandline.as_deref(), Some(custom));
        }
    }

    #[test]
    fn only_our_entries_and_distros_are_rewritten() {
        // Not discovered by us: not touched, and not added a second time.
        let mut theirs = old_entry("Mine", "wsl.exe -d Alpine --cd ~");
        theirs.source = None;
        let mut s = with_entries(vec![theirs.clone()]);
        assert_eq!(s.refresh_discovered(&[found("Alpine", "wsl.exe ~ -d Alpine")], true), RefreshReport::default());
        assert_eq!(s.profiles.list, [theirs]);
        // A shell that isn't a distro keeps whatever it was given.
        let mut s = with_entries(vec![old_entry("bash", "wsl.exe -d bash --cd ~")]);
        s.refresh_discovered(&[found("bash", "/bin/bash -l")], true);
        assert_eq!(s.profiles.list[0].commandline.as_deref(), Some("wsl.exe -d bash --cd ~"));
    }

    #[test]
    fn wsl_home_can_be_left_out() {
        assert_eq!(without_wsl_home("wsl.exe ~ -d Alpine"), "wsl.exe -d Alpine");
        assert_eq!(without_wsl_home(r#"wsl.exe ~ -d "My Distro""#), r#"wsl.exe -d "My Distro""#);
        assert_eq!(without_wsl_home("wsl.exe -d Alpine --cd ~"), "wsl.exe -d Alpine");
        assert_eq!(without_wsl_home("wsl.exe -d Alpine"), "wsl.exe -d Alpine");
        assert_eq!(without_wsl_home("cmd.exe /k echo ~ x"), "cmd.exe /k echo ~ x");
    }

    #[test]
    fn starting_directory_beats_the_wsl_home() {
        let start = commandline_for_start;
        assert_eq!(start("wsl.exe ~ -d Alpine", Some(r"C:\code")), "wsl.exe -d Alpine");
        assert_eq!(start("wsl.exe -d Alpine --cd ~", Some(r"C:\code")), "wsl.exe -d Alpine");
        assert_eq!(start("wsl.exe ~ -d Alpine", Some("%USERPROFILE%")), "wsl.exe -d Alpine");
        // No directory, or that same home.
        assert_eq!(start("wsl.exe ~ -d Alpine", None), "wsl.exe ~ -d Alpine");
        assert_eq!(start("wsl.exe ~ -d Alpine", Some("  ")), "wsl.exe ~ -d Alpine");
        assert_eq!(start("wsl.exe ~ -d Alpine", Some("~")), "wsl.exe ~ -d Alpine");
        // Other programs keep theirs.
        assert_eq!(start("cmd.exe /k echo ~ x", Some(r"C:\code")), "cmd.exe /k echo ~ x");
    }

    #[test]
    fn linux_paths_are_not_windows_directories() {
        for dir in ["/home/bob", "/tmp", "/", "~", "~/src", " /tmp "] {
            assert!(is_wsl_linux_dir("wsl.exe ~ -d Alpine", dir), "{dir}");
            assert!(is_wsl_linux_dir(r"C:\Windows\System32\WSL.EXE -d Alpine", dir), "{dir}");
            assert_eq!(commandline_for_start("wsl.exe ~ -d Alpine", Some(dir)), "wsl.exe ~ -d Alpine");
        }
        for dir in [r"C:\code", r"\\wsl$\Alpine\root", "%USERPROFILE%", "relative"] {
            assert!(!is_wsl_linux_dir("wsl.exe ~ -d Alpine", dir), "{dir}");
        }
        // Only for WSL.
        assert!(!is_wsl_linux_dir("/bin/bash -l", "/tmp"));
        assert!(!is_wsl_linux_dir("cmd.exe", "/tmp"));
    }

    #[test]
    fn default_profile_falls_back_when_removed() {
        let shells = [found("bash", "/bin/bash -l"), found("zsh", "/bin/zsh -l")];
        for wanted in [guid_for("zsh"), guid_for("zsh").to_uppercase(), "zsh".to_string()] {
            let mut s = Settings::default();
            s.refresh_discovered(&shells, true);
            s.default_profile = Some(wanted);
            assert_eq!(s.default_profile_index(), 1);
            s.refresh_discovered(&shells[..1], true);
            assert_eq!(s.default_profile, None);
            assert_eq!(s.default_profile_index(), 0);
        }
        // Anything else stays.
        let mut s = Settings::default();
        s.refresh_discovered(&shells, true);
        s.default_profile = Some("bash".into());
        s.refresh_discovered(&shells[..1], true);
        assert_eq!(s.default_profile.as_deref(), Some("bash"));
        // Pointing at nothing (edited by hand).
        s.default_profile = Some("nope".into());
        assert_eq!(s.default_profile_index(), 0);
    }

    #[test]
    fn default_profile_hidden_falls_back() {
        let mut s = Settings::default();
        s.refresh_discovered(&[found("bash", "/bin/bash -l"), found("zsh", "/bin/zsh -l")], true);
        s.default_profile = Some("bash".into());
        s.profiles.list[0].hidden = Some(true);
        assert_eq!(s.visible_profiles()[s.default_profile_index()].name, "zsh");
    }

    #[test]
    fn reload_environment_setting() {
        assert!(Settings::parse("{}").unwrap().reload_environment_variables);
        // Windows Terminal's name, so an imported or shared file means the same.
        let off = Settings::parse(r#"{ "compatibility.reloadEnvironmentVariables": false }"#).unwrap();
        assert!(!off.reload_environment_variables);
        assert_eq!(Settings::parse(&off.to_json()).unwrap(), off);
        assert!(!off.extra.contains_key("compatibility.reloadEnvironmentVariables"));
        let vars = |name: &str| (name == "P").then(|| "fresh".to_string());
        assert_eq!(expand_env_with("%P%;x;%NOPE%", vars), "fresh;x;%NOPE%");
    }

    #[test]
    fn inherit_working_directory_setting() {
        assert!(Settings::parse("{}").unwrap().inherit_working_directory);
        let off = Settings::parse(r#"{ "wtpp.inheritWorkingDirectory": false }"#).unwrap();
        assert!(!off.inherit_working_directory);
        assert!(off.to_json().contains("\"wtpp.inheritWorkingDirectory\": false"));
        assert!(off.split_duplicates_ssh);
        let off = Settings::parse(r#"{ "wtpp.splitDuplicatesSsh": false }"#).unwrap();
        assert!(!off.split_duplicates_ssh && off.inherit_working_directory);
        assert!(off.to_json().contains("\"wtpp.splitDuplicatesSsh\": false"));
        assert_eq!(Settings::parse(&off.to_json()).unwrap(), off);
    }

    #[test]
    fn profiles_discovered_round_trip() {
        let mut s = Settings::parse(r#"{ "launchWindowAnimation": true }"#).unwrap();
        assert!(!s.profiles_discovered);
        s.profiles_discovered = true;
        let json = s.to_json();
        assert!(json.contains("\"wtpp.profilesDiscovered\": true"), "{json}");
        assert!(json.contains("\"launchWindowAnimation\": true"), "{json}");
        let again = Settings::parse(&json).unwrap();
        assert!(again.profiles_discovered);
        assert_eq!(again, s);
        // Not leaked into the unknown keys.
        assert!(!again.extra.contains_key("wtpp.profilesDiscovered"));
        assert!(!Settings::parse("{}").unwrap().profiles_discovered);
    }

    #[test]
    fn nothing_visible_still_gives_a_shell() {
        let mut s = Settings::default();
        assert_eq!(s.default_profile_index(), 0);
        let fallback = s.visible_profiles();
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].source.as_deref(), Some(FALLBACK_SOURCE));
        assert!(!fallback[0].commandline.trim().is_empty());
        assert!(!fallback[0].hidden);
        let wanted = if cfg!(windows) { "cmd" } else { "sh" };
        assert!(fallback[0].commandline.to_lowercase().contains(wanted), "{}", fallback[0].commandline);

        // Everything hidden: the same, and defaults still apply.
        s.profiles.defaults.color_scheme = Some("Vintage".into());
        s.refresh_discovered(&[found("bash", "/bin/bash -l")], true);
        s.profiles.list[0].hidden = Some(true);
        s.default_profile = Some("bash".into());
        assert_eq!(s.default_profile_index(), 0);
        let fallback = s.visible_profiles();
        assert_eq!(fallback[0].source.as_deref(), Some(FALLBACK_SOURCE));
        assert_eq!(fallback[0].color_scheme, "Vintage");

        // It's never written to the file.
        assert!(!s.to_json().contains(FALLBACK_SOURCE));
        assert_eq!(s.profiles.list.len(), 1);
        assert!(!Settings::default().to_json().contains(FALLBACK_SOURCE));
    }

    #[test]
    fn fallback_makes_no_catalog_or_label_trouble() {
        let s = Settings::default();
        let names: Vec<String> = s.visible_profiles().into_iter().map(|p| p.name).collect();
        for a in Action::catalog(names.len()) {
            let _ = a.label(&names);
        }
        let _ = Action::NewTab(Some(5)).label(&[]);
    }

    #[test]
    fn distro_names() {
        for good in ["Ubuntu", "Ubuntu-22.04", "kali-linux", "OracleLinux_8_5", "SUSE-Linux-Enterprise-Server-15-SP4", "docker-desktop-data", "1a"] {
            assert!(is_plausible_distro_name(good), "{good}");
        }
        let long = "a".repeat(65);
        for bad in ["", "--", "-d", ".hidden", "My Distro", "Usage:", "<x>", "Ünix", "a/b", "wsl.exe [x]", &long] {
            assert!(!is_plausible_distro_name(bad), "{bad}");
        }
    }

    #[test]
    fn refresh_summaries() {
        let report = |added: &[&str], removed: &[&str]| RefreshReport {
            added: added.iter().map(|s| s.to_string()).collect(),
            removed: removed.iter().map(|s| s.to_string()).collect(),
            wsl_kept: false,
        };
        assert_eq!(report(&[], &[]).summary(), "No changes.");
        assert_eq!(report(&["a", "b"], &["c"]).summary(), "Added a, b. Removed c.");
        assert_eq!(report(&[], &["c"]).summary(), "Removed c.");
        let kept = RefreshReport { wsl_kept: true, ..Default::default() };
        assert_eq!(kept.summary(), "No changes. WSL couldn't be read, so existing WSL profiles were kept.");
        assert_eq!(report(&[], &["1", "2", "3", "4", "5", "6"]).summary(), "Removed 1, 2, 3, 4, 2 more.");
        let long = "é".repeat(50);
        assert_eq!(report(&[&long], &[]).summary(), format!("Added {}….", "é".repeat(32)));

        let mut merged = report(&["a"], &["c"]);
        merged.merge(report(&["a", "b"], &["c"]));
        assert_eq!(merged, report(&["a", "b"], &["c"]));
        merged.merge(kept);
        assert!(merged.wsl_kept);
    }

    #[test]
    fn broken_file_is_backed_up_not_lost() {
        let dir = std::env::temp_dir().join(format!("wtpp-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");

        // No file, or a fine one: nothing to do.
        assert_eq!(Settings::backup_if_broken(&path).unwrap(), None);
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(Settings::backup_if_broken(&path).unwrap(), None);

        // load() reports the error and gives defaults; the file stays as it is.
        std::fs::write(&path, "{ broken").unwrap();
        let (settings, error) = Settings::load(&path);
        assert!(error.is_some());
        assert!(!settings.profiles_discovered, "a failed load discovers again");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");

        let first = Settings::backup_if_broken(&path).unwrap().unwrap();
        assert_eq!(first, dir.join("settings.json.bak"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "{ broken");
        // An older backup is never overwritten.
        std::fs::write(&path, "{ broken again").unwrap();
        let second = Settings::backup_if_broken(&path).unwrap().unwrap();
        assert_eq!(second, dir.join("settings.json.bak.1"));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "{ broken");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "{ broken again");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn jsonc() {
        let src = "{\"a\": \"// not a comment\", // comment\n \"b\": [1, 2,], /* c */ \"c\": {\"d\": 1,},\n}";
        let v: Value = serde_json::from_str(&strip_jsonc(src)).unwrap();
        assert_eq!(v["a"], "// not a comment");
        assert_eq!(v["b"], serde_json::json!([1, 2]));
    }

    #[test]
    fn commandlines() {
        assert_eq!(
            split_commandline_windows(r#""C:\Program Files\Git\bin\bash.exe" -i -l"#),
            (r"C:\Program Files\Git\bin\bash.exe".to_string(), vec!["-i".to_string(), "-l".to_string()])
        );
        assert_eq!(split_commandline_windows("wsl.exe -d Debian"), ("wsl.exe".into(), vec!["-d".into(), "Debian".into()]));
        assert_eq!(join_commandline_windows(r"C:\Program Files\x.exe", &["-a".into()]), r#""C:\Program Files\x.exe" -a"#);
        // Backslashes are path separators, single quotes ordinary.
        assert_eq!(split_commandline_windows(r"C:\Windows\system32\cmd.exe /c 'x'").1, ["/c", "'x'"]);
        assert_eq!(split_commandline_windows(r"C:\Windows\system32\cmd.exe").0, r"C:\Windows\system32\cmd.exe");
        // The platform's own.
        let (windows, unix) = (split_commandline_windows("sh -c 'a b'"), split_commandline_unix("sh -c 'a b'"));
        assert_eq!(split_commandline("sh -c 'a b'"), if cfg!(windows) { windows } else { unix });
    }

    /// POSIX shell words on Linux and macOS.
    #[test]
    fn unix_commandlines() {
        let split = |s: &str| {
            let (p, a) = split_commandline_unix(s);
            std::iter::once(p).chain(a).collect::<Vec<_>>()
        };
        assert_eq!(split("sh -c 'echo hello; exit 3'"), ["sh", "-c", "echo hello; exit 3"]);
        assert_eq!(split(r#"/bin/bash -l"#), ["/bin/bash", "-l"]);
        assert_eq!(split(r#""/opt/my shell/bin/zsh" -l"#), ["/opt/my shell/bin/zsh", "-l"]);
        assert_eq!(split(r#"/opt/my\ shell/zsh"#), ["/opt/my shell/zsh"]);
        assert_eq!(split(r#"echo "a \"b\" \$HOME \\ \n" x"#), ["echo", r#"a "b" $HOME \ \n"#, "x"]);
        assert_eq!(split(r#"echo 'it'\''s' "$HOME" ~ *"#), ["echo", "it's", "$HOME", "~", "*"]);
        assert_eq!(split("a '' \"\" b"), ["a", "", "", "b"]);
        assert_eq!(split("  a\tb  \n c "), ["a", "b", "c"]);
        assert_eq!(split("a\\\nb"), ["ab"]);
        assert_eq!(split("a 'unterminated b"), ["a", "unterminated b"]);
        assert_eq!(split(r"trailing\"), [r"trailing\"]);
        assert_eq!(split_commandline_unix(""), (String::new(), vec![]));
        // What's joined splits back.
        for args in [&["-c", "echo hello; exit 3"][..], &["it's", "", "a\\b", "$x", "tab\there", "plain-arg", "~/x"], &[]] {
            let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            for program in ["sh", "/opt/my shell/zsh"] {
                let line = join_commandline_unix(program, &args);
                assert_eq!(split_commandline_unix(&line), (program.to_string(), args.clone()), "{line}");
            }
        }
        assert_eq!(join_commandline_unix("/bin/bash", &["-l".into()]), "/bin/bash -l");
        assert_eq!(join_commandline_unix("sh", &["-c".into(), "a b".into()]), "sh -c 'a b'");
    }

    #[test]
    fn env_expansion() {
        // SAFETY: tests in this module don't read this variable concurrently.
        unsafe { std::env::set_var("WTPP_TEST_ROOT", r"C:\Windows") };
        assert_eq!(expand_env(r"%WTPP_TEST_ROOT%\System32\cmd.exe"), r"C:\Windows\System32\cmd.exe");
        assert_eq!(expand_env("100% sure %NOPE_NOT_SET%"), "100% sure %NOPE_NOT_SET%");
    }

    #[test]
    fn profile_extras() {
        let s = Settings::parse(r#"{
            "profiles": {
                "defaults": { "environment": { "A": "1", "B": "2" }, "font": { "weight": "bold", "cellHeight": "1.4" } },
                "list": [ { "name": "x", "commandline": "x", "environment": { "B": "3" }, "bellStyle": "visual",
                            "intenseTextStyle": "all", "tabTitle": "T", "suppressApplicationTitle": true,
                            "font": { "weight": 300 } } ]
            }
        }"#).unwrap();
        let p = s.resolve(&s.profiles.list[0]);
        assert_eq!(p.environment.get("A").map(String::as_str), Some("1"));
        assert_eq!(p.environment.get("B").map(String::as_str), Some("3"));
        assert_eq!(p.bell_style, BellStyle::Visual);
        assert!(p.intense_text_style.bright() && p.intense_text_style.bold_font());
        assert_eq!(p.font_weight, 300);
        assert_eq!(p.cell_height, 1.4);
        assert!(p.ligatures);
        let mut font = FontSettings::default();
        font.set_ligatures(false);
        assert_eq!(font.ligatures(), Some(false));
        font.set_ligatures(true);
        assert_eq!(font.ligatures(), None);
        assert!(font.extra.is_empty());
        assert_eq!(p.tab_title.as_deref(), Some("T"));
        assert!(p.suppress_application_title);
        let json = s.to_json();
        assert!(json.contains("\"cellHeight\": \"1.4\""), "{json}");
        assert!(json.contains("\"weight\": \"bold\""));
    }

    #[test]
    fn padding_forms() {
        assert_eq!(Padding::parse("8").unwrap().0, [8.0; 4]);
        assert_eq!(Padding::parse("8, 4").unwrap().0, [8.0, 4.0, 8.0, 4.0]);
        assert_eq!(Padding::parse("1,2,3,4").unwrap().0, [1.0, 2.0, 3.0, 4.0]);
        assert!(Padding::parse("1,2,3").is_none());
    }

    #[test]
    fn guids_are_stable_and_distinct() {
        assert_eq!(guid_for("bash"), guid_for("bash"));
        assert_ne!(guid_for("bash"), guid_for("zsh"));
        assert_eq!(guid_for("bash").len(), 38);
    }

    fn profile(s: &Settings, i: usize) -> ResolvedProfile {
        s.resolve(&s.profiles.list[i])
    }

    #[test]
    fn background_settings_parse_and_resolve() {
        let s = Settings::parse(
            r#"{ "profiles": { "list": [ { "name": "A", "opacity": 75, "backgroundImage": " C:/pics/cat.png ",
                "backgroundImageOpacity": 0.4, "backgroundImageStretchMode": "fill",
                "backgroundImageAlignment": "bottomRight" } ] } }"#,
        )
        .unwrap();
        let p = profile(&s, 0);
        assert_eq!(p.opacity, 0.75);
        assert_eq!(p.background_image.as_deref(), Some("C:/pics/cat.png"));
        assert_eq!(p.background_image_opacity, 0.4);
        assert_eq!(p.background_image_stretch_mode, BackgroundImageStretchMode::Fill);
        assert_eq!(p.background_image_alignment, BackgroundImageAlignment::BottomRight);
    }

    #[test]
    fn background_settings_defaults_and_inheritance() {
        let s = Settings::parse(
            r#"{ "profiles": { "defaults": { "opacity": 50, "backgroundImage": "desktopWallpaper",
                "backgroundImageOpacity": 0.3 },
                "list": [ { "name": "A" }, { "name": "B", "opacity": 90, "backgroundImage": "" } ] } }"#,
        )
        .unwrap();
        let a = profile(&s, 0);
        assert_eq!(a.opacity, 0.5);
        assert_eq!(a.background_image.as_deref(), Some("desktopWallpaper"));
        assert_eq!(a.background_image_opacity, 0.3);
        let b = profile(&s, 1);
        assert_eq!(b.opacity, 0.9);
        // An empty path in the profile turns off the inherited image.
        assert_eq!(b.background_image, None);

        let plain = Settings::parse(r#"{ "profiles": { "list": [ { "name": "A" } ] } }"#).unwrap();
        let p = profile(&plain, 0);
        assert_eq!(p.opacity, 1.0);
        assert_eq!(p.background_image, None);
        assert_eq!(p.background_image_opacity, 1.0);
        assert_eq!(p.background_image_stretch_mode, BackgroundImageStretchMode::UniformToFill);
        assert_eq!(p.background_image_alignment, BackgroundImageAlignment::Center);
        assert!(!plain.wants_transparency());
        assert!(s.wants_transparency());
        let only_one = Settings::parse(r#"{ "profiles": { "list": [ { "name": "A" }, { "name": "B", "opacity": 99 } ] } }"#).unwrap();
        assert!(only_one.wants_transparency());
    }

    #[test]
    fn background_settings_clamp() {
        let s = Settings::parse(
            r#"{ "profiles": { "list": [ { "name": "A", "opacity": 150, "backgroundImageOpacity": 2.0 },
                { "name": "B", "opacity": -5, "backgroundImageOpacity": "-1" } ] } }"#,
        )
        .unwrap();
        assert_eq!(profile(&s, 0).opacity, 1.0);
        assert_eq!(profile(&s, 0).background_image_opacity, 1.0);
        assert_eq!(profile(&s, 1).opacity, 0.0);
        assert_eq!(profile(&s, 1).background_image_opacity, 0.0);
    }

    #[test]
    fn background_settings_round_trip() {
        let mut s = Settings::default();
        s.profiles.list.push(ProfileSettings {
            name: Some("A".into()),
            opacity: Some(60.0),
            background_image: Some("~/bg.png".into()),
            background_image_opacity: Some(0.3),
            background_image_stretch_mode: Some(BackgroundImageStretchMode::None),
            background_image_alignment: Some(BackgroundImageAlignment::TopLeft),
            ..Default::default()
        });
        let json = s.to_json();
        assert!(json.contains("\"opacity\": 60,"), "{json}");
        assert!(json.contains("\"backgroundImageOpacity\": 0.3,"), "{json}");
        assert!(json.contains("\"backgroundImageStretchMode\": \"none\""), "{json}");
        assert!(json.contains("\"backgroundImageAlignment\": \"topLeft\""), "{json}");
        let back = Settings::parse(&json).unwrap();
        assert_eq!(back.profiles.list[0], s.profiles.list[0]);
    }

    #[test]
    fn background_alignment_axes() {
        use BackgroundImageAlignment as A;
        let expect = [
            (A::Center, 0, 0),
            (A::Left, -1, 0),
            (A::Top, 0, -1),
            (A::Right, 1, 0),
            (A::Bottom, 0, 1),
            (A::TopLeft, -1, -1),
            (A::TopRight, 1, -1),
            (A::BottomLeft, -1, 1),
            (A::BottomRight, 1, 1),
        ];
        assert_eq!(expect.len(), A::ALL.len());
        for (a, h, v) in expect {
            assert_eq!((a.horizontal(), a.vertical()), (h, v), "{a:?}");
        }
    }
}
