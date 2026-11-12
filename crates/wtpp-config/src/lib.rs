//! wt++ settings: `settings.json` (Windows Terminal's format), color schemes,
//! and key bindings.

pub mod actions;
pub mod cwd;
pub mod keymap;
pub mod schemes;
pub mod ssh;
pub mod settings;

pub use actions::{Action, Axis, Chord, Direction};
pub use keymap::{Binding, Keymap};
pub use schemes::{COLOR_NAMES, Color, Scheme};
pub use settings::{AppTheme, TabWidthMode, BellStyle, GraphicsApi, CloseOnExit, CursorShape, IntenseTextStyle, KeyEncoding, LaunchMode, ResolvedProfile, Settings};
