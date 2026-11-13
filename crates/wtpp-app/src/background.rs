//! Background images (WT's `backgroundImage`): decoded off the UI thread,
//! cached by setting value, and decoded again when the file changes.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use slint::{Rgba8Pixel, SharedPixelBuffer};
use wtpp_config::{BackgroundImageAlignment, BackgroundImageStretchMode};

/// What identifies a version of a file: modification time and size.
type Stamp = (SystemTime, u64);

struct Entry {
    /// The file, once `backgroundImage` is resolved.
    path: Option<PathBuf>,
    /// The file as it was when last decoded (or tried); `None` if missing.
    stamp: Option<Stamp>,
    image: Option<slint::Image>,
    loading: bool,
}

thread_local! {
    /// By the setting's value as written (UI thread only).
    static CACHE: RefCell<HashMap<String, Entry>> = RefCell::new(HashMap::new());
    /// Images larger than the GPU's texture limit are scaled down to it.
    static MAX_DIM: Cell<u32> = const { Cell::new(8192) };
}

/// The device's largest texture side.
pub fn set_max_dimension(max: u32) {
    MAX_DIM.with(|m| m.set(max.clamp(256, 16384)));
}

/// The decoded image for a `backgroundImage` value; `None` while it loads,
/// or if it can't be read. Starts loading on first use.
pub fn image_for(raw: &str) -> Option<slint::Image> {
    let cached = CACHE.with(|c| c.borrow().get(raw).map(|e| e.image.clone()));
    if let Some(image) = cached {
        return image;
    }
    let path = resolve_path(raw);
    CACHE.with(|c| {
        c.borrow_mut()
            .insert(raw.to_string(), Entry { path: path.clone(), stamp: None, image: None, loading: path.is_some() })
    });
    if let Some(path) = path {
        load(raw.to_string(), path);
    }
    None
}

/// Forget images no profile uses any more, and decode again those whose
/// file (or, for `desktopWallpaper`, whose path) changed.
pub fn refresh(in_use: &HashSet<String>) {
    let reload: Vec<(String, PathBuf)> = CACHE.with(|c| {
        let mut cache = c.borrow_mut();
        cache.retain(|raw, _| in_use.contains(raw));
        cache
            .iter_mut()
            .filter(|(_, e)| !e.loading)
            .filter_map(|(raw, e)| {
                let path = resolve_path(raw);
                let changed = path != e.path;
                e.path = path.clone();
                let Some(path) = path else {
                    e.image = None;
                    return None;
                };
                (changed || stale(e.stamp, stamp(&path))).then(|| {
                    e.loading = true;
                    (raw.clone(), path)
                })
            })
            .collect()
    });
    for (raw, path) in reload {
        load(raw, path);
    }
}

/// The cached decode no longer matches the file.
fn stale(cached: Option<Stamp>, current: Option<Stamp>) -> bool {
    cached != current
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Decode on a worker thread, then store the image and repaint.
fn load(raw: String, path: PathBuf) {
    let max = MAX_DIM.with(Cell::get);
    std::thread::spawn(move || {
        let stamp = stamp(&path);
        let decoded = decode(&path, max);
        if let Err(e) = &decoded {
            log::warn!("background image {}: {e}", path.display());
        }
        let decoded = decoded.ok();
        slint::invoke_from_event_loop(move || {
            CACHE.with(|c| {
                if let Some(entry) = c.borrow_mut().get_mut(&raw) {
                    entry.loading = false;
                    entry.stamp = stamp;
                    // A failed decode shows no image, and isn't retried until
                    // the file changes.
                    entry.image = decoded.map(slint::Image::from_rgba8);
                }
            });
            crate::for_each_window(|app| app.render());
        })
        .ok();
    });
}

fn decode(path: &Path, max: u32) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    let mut rgba = image::open(path).map_err(|e| e.to_string())?.into_rgba8();
    let (w, h) = rgba.dimensions();
    if w > max || h > max {
        let (nw, nh) = fit_within(w, h, max);
        rgba = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Triangle);
    }
    let (w, h) = rgba.dimensions();
    Ok(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), w, h))
}

/// `w`×`h` scaled down, keeping the aspect ratio, so neither side exceeds `max`.
fn fit_within(w: u32, h: u32, max: u32) -> (u32, u32) {
    let scale = max as f64 / w.max(h) as f64;
    (((w as f64 * scale).round() as u32).clamp(1, max), ((h as f64 * scale).round() as u32).clamp(1, max))
}

/// The file a `backgroundImage` value names.
fn resolve_path(raw: &str) -> Option<PathBuf> {
    resolve_path_with(raw, |name| std::env::var(name).ok(), crate::platform::desktop_wallpaper, cfg!(windows))
}

fn resolve_path_with(
    raw: &str,
    var: impl Fn(&str) -> Option<String>,
    wallpaper: impl FnOnce() -> Option<PathBuf>,
    windows: bool,
) -> Option<PathBuf> {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.eq_ignore_ascii_case("desktopWallpaper") {
        if !windows && !WARNED.swap(true, Ordering::Relaxed) {
            log::info!("backgroundImage: desktopWallpaper is only supported on Windows");
        }
        return if windows { wallpaper() } else { None };
    }
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("ms-appx:") || lower.starts_with("ms-appdata:") {
        log::info!("backgroundImage: {raw} (a Windows Terminal package path) isn't supported");
        return None;
    }
    let raw = match raw.get(..8).filter(|p| p.eq_ignore_ascii_case("file:///")) {
        // file:///C:/x on Windows, file:///home/x elsewhere.
        Some(_) if windows => &raw[8..],
        Some(_) => &raw[7..],
        None => raw,
    };
    Some(PathBuf::from(wtpp_config::settings::expand_env_with(raw, var)))
}

/// Slint `Image` placement for WT's stretch mode and alignment:
/// image-fit (0 none, 1 fill, 2 uniform, 3 uniform to fill) and the
/// horizontal and vertical alignment (-1 start, 0 centre, 1 end).
pub fn image_layout(mode: BackgroundImageStretchMode, align: BackgroundImageAlignment) -> (i32, i32, i32) {
    let fit = match mode {
        BackgroundImageStretchMode::None => 0,
        BackgroundImageStretchMode::Fill => 1,
        BackgroundImageStretchMode::Uniform => 2,
        BackgroundImageStretchMode::UniformToFill => 3,
    };
    (fit, align.horizontal() as i32, align.vertical() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn env(name: &str) -> Option<String> {
        match name {
            "HOME" | "USERPROFILE" => Some("/home/me".into()),
            "PICS" => Some("/pics".into()),
            _ => None,
        }
    }

    fn resolve(raw: &str, windows: bool) -> Option<PathBuf> {
        resolve_path_with(raw, env, || Some(PathBuf::from("C:/wall.jpg")), windows)
    }

    #[test]
    fn resolves_paths() {
        assert_eq!(resolve("  ", false), None);
        assert_eq!(resolve("/a/b.png", false), Some("/a/b.png".into()));
        assert_eq!(resolve("~/bg.png", false), Some("/home/me/bg.png".into()));
        assert_eq!(resolve("%PICS%/cat.jpg", false), Some("/pics/cat.jpg".into()));
        assert_eq!(resolve("file:///home/me/x.png", false), Some("/home/me/x.png".into()));
        assert_eq!(resolve("file:///C:/x.png", true), Some("C:/x.png".into()));
        assert_eq!(resolve("ms-appx:///images/bg.png", true), None);
    }

    #[test]
    fn desktop_wallpaper_only_on_windows() {
        assert_eq!(resolve("desktopWallpaper", true), Some("C:/wall.jpg".into()));
        assert_eq!(resolve("DESKTOPWALLPAPER", true), Some("C:/wall.jpg".into()));
        assert_eq!(resolve("desktopWallpaper", false), None);
        assert_eq!(resolve_path_with("desktopWallpaper", env, || None, true), None);
    }

    #[test]
    fn staleness() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        assert!(!stale(Some((t, 10)), Some((t, 10))));
        assert!(stale(Some((t, 10)), Some((t, 11))));
        assert!(stale(Some((t, 10)), Some((t + Duration::from_secs(1), 10))));
        assert!(stale(None, Some((t, 10))), "the file appeared");
        assert!(stale(Some((t, 10)), None), "the file went away");
        assert!(!stale(None, None), "still missing: don't retry");
    }

    #[test]
    fn layout_mapping() {
        use BackgroundImageAlignment as A;
        use BackgroundImageStretchMode as S;
        assert_eq!(image_layout(S::default(), A::default()), (3, 0, 0));
        assert_eq!(image_layout(S::None, A::TopLeft), (0, -1, -1));
        assert_eq!(image_layout(S::Fill, A::Right), (1, 1, 0));
        assert_eq!(image_layout(S::Uniform, A::BottomRight), (2, 1, 1));
    }

    #[test]
    fn scales_down_keeping_aspect() {
        assert_eq!(fit_within(16000, 8000, 8192), (8192, 4096));
        assert_eq!(fit_within(1000, 20000, 8192), (410, 8192));
        assert_eq!(fit_within(1, 100000, 8192), (1, 8192));
    }
}
