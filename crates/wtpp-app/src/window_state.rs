//! The window's last position and size, restored on the next launch
//! (`wtpp.rememberWindowBounds`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    /// Physical pixels, of the restored (not maximized) window.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

fn path() -> PathBuf {
    wtpp_config::settings::settings_path().with_file_name("state.json")
}

pub fn load() -> Option<WindowState> {
    let state: WindowState = serde_json::from_str(&std::fs::read_to_string(path()).ok()?).ok()?;
    // Ignore nonsense (e.g. a window that was minimized when saved).
    (state.width >= 200 && state.height >= 120 && state.x > -10_000 && state.y > -10_000).then_some(state)
}

pub fn save(window: &slint::Window) {
    let maximized = window.is_maximized();
    if window.is_minimized() || window.is_fullscreen() {
        return;
    }
    // Maximized: keep the last normal bounds, just remember the flag.
    let previous = load();
    let (pos, size) = (window.position(), window.size());
    let state = match (maximized, previous) {
        (true, Some(prev)) => WindowState { maximized: true, ..prev },
        _ => WindowState { x: pos.x, y: pos.y, width: size.width, height: size.height, maximized },
    };
    if let Ok(json) = serde_json::to_string_pretty(&state)
        && let Err(e) = std::fs::write(path(), json)
    {
        log::warn!("couldn't save window state: {e}");
    }
}

/// Parse WT's `initialPosition`: "x,y".
pub fn parse_position(s: &str) -> Option<(i32, i32)> {
    let (x, y) = s.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions() {
        assert_eq!(parse_position("100, 200"), Some((100, 200)));
        assert_eq!(parse_position("-5,7"), Some((-5, 7)));
        assert_eq!(parse_position(","), None);
        assert_eq!(parse_position("12"), None);
    }
}
