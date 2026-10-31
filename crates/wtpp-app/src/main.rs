// A GUI app on Windows: don't open a console window alongside it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod cli;
mod default_terminal;
mod file_drop;
mod find;
mod instance;
mod keys;
mod mark;
mod palette;
mod platform;
mod pane_tree;
mod settings_ui;
mod tabs;
mod titlebar;
mod window_state;
mod wsl;

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Context;
use slint::ComponentHandle;
use slint::wgpu_30::{WGPUConfiguration, WGPUSettings, wgpu};
use wtpp_input::Mods;

use app::App;

slint::include_modules!();

/// Index of a window in `WINDOWS` (never reused).
pub type WindowId = usize;

struct WindowEntry {
    ui: MainWindow,
    app: RefCell<App>,
}

thread_local! {
    static WINDOWS: RefCell<Vec<Option<Rc<WindowEntry>>>> = const { RefCell::new(Vec::new()) };
    /// Windows by how recently they were focused, most recent first.
    static FOCUS_ORDER: RefCell<Vec<WindowId>> = const { RefCell::new(Vec::new()) };
    /// The shared font database and monospace families, once loaded.
    static FONTS: RefCell<Option<(wtpp_render::SharedFonts, Vec<String>)>> = const { RefCell::new(None) };
}

fn entry(id: WindowId) -> Option<Rc<WindowEntry>> {
    WINDOWS.with(|w| w.borrow().get(id).cloned().flatten())
}

fn window_ids() -> Vec<WindowId> {
    WINDOWS.with(|w| w.borrow().iter().enumerate().filter(|(_, e)| e.is_some()).map(|(i, _)| i).collect())
}

/// Run `f` on a window's state. Re-entrant calls (a UI callback fired while
/// that window is already updating its UI) are dropped rather than panicking.
pub fn with_window<R>(id: WindowId, f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let entry = entry(id)?;
    let Ok(mut app) = entry.app.try_borrow_mut() else {
        log::debug!("re-entrant access to window {id} ignored");
        return None;
    };
    Some(f(&mut app))
}

/// Run `f` on the window that currently holds pane `pane`: tabs move between
/// windows, so shells find their window this way.
pub fn with_pane<R>(pane: pane_tree::PaneId, f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let id = window_ids()
        .into_iter()
        .find(|&id| entry(id).is_some_and(|e| e.app.try_borrow().is_ok_and(|app| app.panes.contains_key(&pane))))?;
    with_window(id, f)
}

fn for_each_window(f: impl Fn(&mut App)) {
    for id in window_ids() {
        with_window(id, &f);
    }
}

fn window_focused(id: WindowId) {
    FOCUS_ORDER.with(|f| {
        let mut f = f.borrow_mut();
        f.retain(|&w| w != id);
        f.insert(0, id);
    });
}

/// Top-left of a window's client area on screen, in physical pixels.
pub(crate) fn client_origin(window: &slint::Window) -> Option<(i32, i32)> {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|w| w.inner_position().ok().map(|p| (p.x, p.y))).flatten()
}

/// Where in `ui`'s tab row a tab dropped at `screen` would go: the index
/// to insert it at, or `None` if `screen` isn't over the tab row.
fn tab_row_slot(ui: &MainWindow, (sx, sy): (i32, i32)) -> Option<usize> {
    let window = ui.window();
    if !window.is_visible() || window.is_minimized() {
        return None;
    }
    let (left, top) = client_origin(window)?;
    let scale = window.scale_factor();
    let (x, y) = ((sx - left) as f32 / scale, (sy - top) as f32 / scale);
    let width = window.size().width as f32 / scale;
    if x < 0.0 || x > width || y < -8.0 || y > ui.get_titlebar_height() + 8.0 {
        return None;
    }
    let n = slint::Model::row_count(&ui.get_tabs());
    let pitch = ui.get_tab_pitch();
    let slot = if pitch > 0.0 { ((x - ui.get_tabs_left()) / pitch).round().clamp(0.0, n as f32) as usize } else { n };
    Some(slot)
}

/// A tab from window `source` is being dragged over `screen`: the window
/// whose tab row is there (the most recently focused one if they overlap)
/// and the slot it would go in. Marks that slot and clears the marker
/// everywhere else; `None` for `screen` clears them all.
pub(crate) fn tab_drop_target(source: WindowId, screen: Option<(i32, i32)>) -> Option<(WindowId, usize)> {
    let mut order = FOCUS_ORDER.with(|f| f.borrow().clone());
    let rest: Vec<_> = window_ids().into_iter().filter(|id| !order.contains(id)).collect();
    order.extend(rest);
    let target = screen.and_then(|screen| {
        order
            .into_iter()
            .filter(|&id| id != source)
            .find_map(|id| entry(id).and_then(|e| tab_row_slot(&e.ui, screen)).map(|slot| (id, slot)))
    });
    for id in window_ids() {
        if let Some(e) = entry(id) {
            e.ui.set_drop_index(match target {
                Some((t, slot)) if t == id => slot as i32,
                _ => -1,
            });
        }
    }
    target
}

/// Put a tab dragged from another window into window `target` at `slot`,
/// and bring that window to the front.
pub(crate) fn give_tab(target: WindowId, moved: app::MovedTab, slot: usize) {
    with_window(target, |app| app.adopt_tab_at(moved, slot));
    if let Some(e) = entry(target) {
        use slint::winit_030::WinitWindowAccessor;
        e.ui.window().with_winit_window(|w| w.focus_window());
    }
}

/// Close a window (after the current event). The app exits with the last one.
pub fn close_window(id: WindowId) {
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let Some(entry) = WINDOWS.with(|w| w.borrow_mut().get_mut(id).and_then(Option::take)) else { return };
        FOCUS_ORDER.with(|f| f.borrow_mut().retain(|&w| w != id));
        let last = window_ids().is_empty();
        if last && entry.app.try_borrow().is_ok_and(|app| app.settings.remember_window_bounds) {
            window_state::save(entry.ui.window());
        }
        entry.ui.hide().ok();
        drop(entry);
        if last {
            slint::quit_event_loop().ok();
        }
    });
}

/// Where a new window opens.
pub(crate) enum Placement {
    /// The app's first window: configured or remembered bounds.
    First,
    /// Configured size, cascaded from the most recently focused window (so
    /// it opens on the monitor you're working on).
    Default,
    /// A tab dropped outside any window: there, at its old window's size.
    At(slint::PhysicalPosition, slint::PhysicalSize),
}

/// Open a new window (after the current event), optionally with a tab moved
/// from another window.
pub(crate) fn request_window(moved: Option<app::MovedTab>, placement: Placement) {
    // A timer rather than invoke_from_event_loop: the moved tab isn't Send.
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        let settings = wtpp_config::Settings::load(&wtpp_config::settings::settings_path()).0;
        if let Err(e) = create_window(settings, None, moved, placement) {
            log::error!("couldn't open a window: {e:#}");
        }
    });
}

/// Log to stderr, or to the file named by `WTPP_LOG_FILE` (release builds on
/// Windows have no console to print to).
fn init_logging() {
    let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if let Some(path) = std::env::var_os("WTPP_LOG_FILE")
        && let Ok(file) = std::fs::File::create(path)
    {
        builder.target(env_logger::Target::Pipe(Box::new(file)));
    }
    builder.format_timestamp_millis().init();
}

/// Probe only each platform's native graphics API: checking every backend
/// (Vulkan, DX12, GL…) costs hundreds of milliseconds at startup.
/// `api` is the user's `rendering.graphicsAPI`; the WGPU_BACKEND environment
/// variable still wins, for debugging.
fn wgpu_configuration(api: wtpp_config::GraphicsApi) -> WGPUConfiguration {
    use wtpp_config::GraphicsApi;
    let mut settings = WGPUSettings::default();
    if std::env::var_os("WGPU_BACKEND").is_some() {
        return WGPUConfiguration::Automatic(settings);
    }
    settings.backends = match api {
        GraphicsApi::Automatic if cfg!(windows) => wgpu::Backends::DX12,
        GraphicsApi::Automatic if cfg!(target_os = "macos") => wgpu::Backends::METAL,
        GraphicsApi::Automatic => settings.backends,
        GraphicsApi::Direct3d12 => wgpu::Backends::DX12,
        GraphicsApi::Vulkan => wgpu::Backends::VULKAN,
        GraphicsApi::Metal => wgpu::Backends::METAL,
        GraphicsApi::OpenGl => wgpu::Backends::GL,
        GraphicsApi::Software => {
            // Slint skips CPU adapters unless told; wgpu picks the adapter
            // named here first (WARP on Windows, llvmpipe with Mesa).
            // SAFETY: single-threaded at this point in startup.
            unsafe {
                std::env::set_var("SLINT_WGPU_CPU", "1");
                std::env::set_var("WGPU_ADAPTER_NAME", if cfg!(windows) { "Microsoft Basic Render Driver" } else { "llvmpipe" });
            }
            if cfg!(windows) { wgpu::Backends::DX12 } else { wgpu::Backends::VULKAN | wgpu::Backends::GL }
        }
    };
    WGPUConfiguration::Automatic(settings)
}

/// Window size from `initialCols`/`initialRows` and the launch mode. The size
/// is an estimate from the default font (real metrics need the fonts, which
/// load in the background), close enough to WT's behavior.
/// Returns the window size it chose.
/// Only the app's `first` window restores the remembered bounds.
fn apply_launch_settings(ui: &MainWindow, settings: &wtpp_config::Settings, first: bool) -> slint::WindowSize {
    let profile = settings.visible_profiles().into_iter().nth(settings.default_profile_index());
    let (size, padding) = profile.map_or((wtpp_config::settings::DEFAULT_FONT_SIZE, wtpp_config::settings::DEFAULT_PADDING), |p| {
        (p.font_size, p.padding)
    });
    let px = size * 96.0 / 72.0;
    let width = settings.initial_cols as f32 * (px * 0.6).round() + padding[0] + padding[2];
    let height = settings.initial_rows as f32 * (px * 1.2).round() + padding[1] + padding[3] + 40.0;
    let mut size = slint::WindowSize::Logical(slint::LogicalSize::new(width.min(2400.0), height.min(1600.0)));

    let mut maximize = false;
    if let Some((x, y)) = settings.initial_position.as_deref().and_then(window_state::parse_position) {
        ui.window().set_position(slint::PhysicalPosition::new(x, y));
    } else if first
        && settings.remember_window_bounds
        && let Some(state) = window_state::load()
    {
        ui.window().set_position(slint::PhysicalPosition::new(state.x, state.y));
        size = slint::WindowSize::Physical(slint::PhysicalSize::new(state.width, state.height));
        maximize = state.maximized;
    }
    ui.window().set_size(size.clone());
    if maximize && settings.launch_mode == wtpp_config::LaunchMode::Default {
        ui.window().set_maximized(true);
    }
    match settings.launch_mode {
        wtpp_config::LaunchMode::Default => {}
        wtpp_config::LaunchMode::Maximized => ui.window().set_maximized(true),
        wtpp_config::LaunchMode::Fullscreen => ui.window().set_fullscreen(true),
    }
    size
}

/// Reload settings when `settings.json` changes on disk.
fn watch_settings(path: &std::path::Path) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;
    let dir = path.parent()?.to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    let file_name = path.file_name()?.to_os_string();
    let pending = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        if !event.paths.iter().any(|p| p.file_name() == Some(&file_name)) {
            return;
        }
        // Editors write in bursts; coalesce into one reload.
        if !pending.swap(true, std::sync::atomic::Ordering::AcqRel) {
            let pending = pending.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(150));
                pending.store(false, std::sync::atomic::Ordering::Release);
                slint::invoke_from_event_loop(|| {
                    for_each_window(|app| app.on_settings_file_changed());
                })
                .ok();
            });
        }
    })
    .map_err(|e| log::warn!("can't watch settings: {e}"))
    .ok()?;
    watcher.watch(&dir, notify::RecursiveMode::NonRecursive).map_err(|e| log::warn!("can't watch settings: {e}")).ok()?;
    Some(watcher)
}

fn wire_settings(ui: &MainWindow, id: WindowId) {
    let data = ui.global::<SettingsData>();
    data.on_set_value(move |key, value| {
        with_window(id, |app| app.settings_set_value(&key, &value));
    });
    data.on_save(move || {
        with_window(id, |app| app.settings_save());
    });
    data.on_discard(move || {
        with_window(id, |app| app.settings_discard());
    });
    data.on_open_json(move || {
        with_window(id, |app| app.settings_open_json());
    });
    data.on_set_default_term(move |make_default| {
        with_window(id, |app| app.settings_set_default_term(make_default));
    });
    data.on_import_wt(move || {
        with_window(id, |app| app.settings_import_wt());
    });
    data.on_select_scheme(move |i| {
        with_window(id, |app| app.settings_select_scheme(i.max(0) as usize));
    });
    data.on_set_scheme_color(move |i, hex| {
        with_window(id, |app| app.settings_set_scheme_color(i.max(0) as usize, &hex));
    });
    data.on_new_scheme(move || {
        with_window(id, |app| app.settings_new_scheme());
    });
    data.on_delete_scheme(move || {
        with_window(id, |app| app.settings_delete_scheme());
    });
    data.on_record_binding(move |row| {
        with_window(id, |app| app.settings_record_binding(row.max(0) as usize));
    });
    data.on_delete_binding(move |row| {
        with_window(id, |app| app.settings_delete_binding(row.max(0) as usize));
    });
    data.on_add_binding(move || {
        with_window(id, |app| app.settings_add_binding());
    });
    data.on_reset_bindings(move || {
        with_window(id, |app| app.settings_reset_bindings());
    });
    data.on_set_binding_input(move |row, text| {
        with_window(id, |app| app.settings_set_binding_input(row.max(0) as usize, &text));
    });
    data.on_add_profile(move || {
        with_window(id, |app| app.settings_add_profile());
    });
    data.on_delete_profile(move || {
        with_window(id, |app| app.settings_delete_profile());
    });
}

fn main() -> anyhow::Result<()> {
    init_logging();
    wsl::ensure_gpu_env();

    let started = std::time::Instant::now();

    let request = instance::Request { args: std::env::args().skip(1).collect(), cwd: std::env::current_dir().ok() };
    let args = parse_args(request);
    // Started by COM for a console handoff (wt++ is the default terminal,
    // and no wt++ was running to take it): serve it, whatever else runs.
    let embedding = args.embedding;
    // Otherwise, when a wt++ is already running it opens the window, so tabs
    // can be dragged between all windows.
    let listener = if embedding {
        None
    } else {
        let request = instance::Request { args: std::env::args().skip(1).collect(), cwd: std::env::current_dir().ok() };
        match instance::claim(&request) {
            instance::Claim::Forwarded => {
                log::info!("handed the command line to the running wt++");
                return Ok(());
            }
            instance::Claim::Primary(listener) => Some(listener),
        }
    };

    // Settings are one small file: read them now so the window opens at
    // the right size and mode, with the chosen graphics API.
    let settings_path = wtpp_config::settings::settings_path();
    let (settings, settings_error) = wtpp_config::Settings::load(&settings_path);
    if let Some(error) = &settings_error {
        log::error!("settings: {error}");
    }

    slint::BackendSelector::new()
        .require_wgpu_30(wgpu_configuration(settings.graphics_api))
        .select()
        .context("no GPU backend available")?;
    log::info!("backend selected after {:?}", started.elapsed());

    // Font and shell discovery are slow (hundreds of fonts on Windows,
    // `wsl.exe -l`), so they run while the window opens. Started only now:
    // invoke_from_event_loop needs the backend selected above.
    std::thread::spawn(move || {
        let fonts = wtpp_render::load_fonts();
        log::info!("fonts loaded after {:?}", started.elapsed());
        let profiles = wtpp_session::profiles::discover();
        log::info!("{} profiles found after {:?}", profiles.len(), started.elapsed());
        let delivered = slint::invoke_from_event_loop(move || on_assets_loaded(fonts, profiles));
        if let Err(e) = delivered {
            log::error!("could not hand fonts to the UI thread: {e}");
        }
    });

    if embedding {
        // The window comes with the console; give up if none does.
        slint::Timer::single_shot(std::time::Duration::from_secs(60), || {
            if window_ids().is_empty() {
                slint::quit_event_loop().ok();
            }
        });
    } else {
        create_window(settings, Some(args), None, Placement::First)?;
        log::info!("window created after {:?}", started.elapsed());
    }
    if let Some(listener) = listener {
        listener.serve(on_instance_request);
    }
    #[cfg(windows)]
    default_terminal::serve(on_handoff, embedding);
    let _watcher = watch_settings(&settings_path);

    // Cursor blink, for every window.
    let blink = slint::Timer::default();
    blink.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(530), || {
        for_each_window(|app| app.on_blink());
    });

    // Windows come and go; `close_window` quits after the last one.
    slint::run_event_loop_until_quit()?;
    Ok(())
}

/// Command-line arguments; a relative `-d` is relative to the launching
/// process's directory.
fn parse_args(request: instance::Request) -> cli::Args {
    let mut args = cli::Args::parse(request.args).unwrap_or_else(|e| {
        log::error!("{e}");
        cli::Args::default()
    });
    if let (Some(dir), Some(cwd)) = (&args.starting_directory, &request.cwd)
        && dir.is_relative()
    {
        args.starting_directory = Some(cwd.join(dir));
    }
    args
}

/// wt++ was launched again: open its window here.
fn on_instance_request(request: instance::Request) {
    let args = parse_args(request);
    if args.embedding {
        return;
    }
    let settings = wtpp_config::Settings::load(&wtpp_config::settings::settings_path()).0;
    if let Err(e) = create_window(settings, Some(args), None, Placement::Default) {
        log::error!("couldn't open a window: {e:#}");
    }
}

/// Windows handed us a console program (wt++ is the default terminal): a
/// new window for it, like WT.
#[cfg(windows)]
fn on_handoff(handoff: default_terminal::Handoff) {
    let settings = wtpp_config::Settings::load(&wtpp_config::settings::settings_path()).0;
    let placement = if window_ids().is_empty() { Placement::First } else { Placement::Default };
    match create_window(settings, None, None, placement) {
        Ok(id) => {
            with_window(id, |app| app.handoff = Some(handoff));
            // The user started the program, so its window belongs in front
            // (Windows won't put it there for us; see bring_to_front). Again
            // once it has drawn, in case it wasn't ready the first time.
            let raise = move || {
                if let Some(e) = entry(id) {
                    platform::bring_to_front(e.ui.window());
                }
            };
            raise();
            slint::Timer::single_shot(std::time::Duration::from_millis(250), raise);
        }
        Err(e) => log::error!("couldn't open a window for a console: {e:#}"),
    }
}

/// The background thread finished loading fonts and finding shells.
fn on_assets_loaded(fonts: wtpp_render::FontSystem, discovered: Vec<wtpp_session::Profile>) {
    let families = wtpp_render::monospace_families(&fonts);
    let fonts: wtpp_render::SharedFonts = Rc::new(RefCell::new(fonts));
    FONTS.with(|f| *f.borrow_mut() = Some((fonts.clone(), families.clone())));
    for (n, id) in window_ids().into_iter().enumerate() {
        // Only one window merges newly found shells into settings.json.
        let discovered = (n == 0).then_some(discovered.as_slice());
        with_window(id, |app| app.on_assets_loaded(fonts.clone(), families.clone(), discovered));
    }
}

/// Down and to the right of the most recently focused window, like WT;
/// back to the top-left of its monitor once that would run off the bottom
/// or right.
fn cascade_position() -> Option<slint::PhysicalPosition> {
    let id = FOCUS_ORDER.with(|f| f.borrow().first().copied()).or_else(|| window_ids().first().copied())?;
    let e = entry(id)?;
    let window = e.ui.window();
    let (outer, size) = (window.position(), window.size());
    let step = (32.0 * window.scale_factor()).round() as i32;
    let (mut x, mut y) = (outer.x + step, outer.y + step);
    if let Some((left, top, right, bottom)) = platform::work_area_at(outer.x + size.width as i32 / 2, outer.y + 8)
        && (x + size.width as i32 > right || y + size.height as i32 > bottom)
    {
        (x, y) = (left, top);
    }
    Some(slint::PhysicalPosition::new(x, y))
}

/// Create and show a window, opening `args` (a command line) or `moved` (a
/// tab from another window) in it.
fn create_window(
    settings: wtpp_config::Settings,
    args: Option<cli::Args>,
    moved: Option<app::MovedTab>,
    placement: Placement,
) -> anyhow::Result<WindowId> {
    let ui = MainWindow::new()?;
    let id = WINDOWS.with(|w| w.borrow().len());
    let custom_titlebar = cfg!(windows) && settings.show_tabs_in_titlebar;
    let mut launch_size = apply_launch_settings(&ui, &settings, matches!(placement, Placement::First));
    match placement {
        _ if let Some((x, y)) = args.as_ref().and_then(|a| a.position) => {
            ui.window().set_position(slint::PhysicalPosition::new(x, y));
        }
        Placement::First => {}
        Placement::Default => {
            if let Some(position) = cascade_position() {
                ui.window().set_position(position);
            }
        }
        Placement::At(position, size) => {
            ui.window().set_position(position);
            launch_size = slint::WindowSize::Physical(size);
            ui.window().set_size(launch_size.clone());
        }
    }
    let mut app = App::new(&ui, settings, wtpp_config::settings::settings_path(), id);
    app.startup_args = args;
    app.launch_size = Some(launch_size);
    app.adopt = moved;
    let entry = Rc::new(WindowEntry { ui: ui.clone_strong(), app: RefCell::new(app) });
    WINDOWS.with(|w| w.borrow_mut().push(Some(entry)));
    if let Some((fonts, families)) = FONTS.with(|f| f.borrow().clone()) {
        with_window(id, |app| app.on_assets_loaded(fonts, families, None));
    }
    wire_window(&ui, id, custom_titlebar)?;
    ui.show()?;
    Ok(id)
}

fn wire_window(ui: &MainWindow, id: WindowId, custom_titlebar: bool) -> anyhow::Result<()> {
    ui.window().set_rendering_notifier(move |state, api| {
        if custom_titlebar && matches!(state, slint::RenderingState::AfterRendering) {
            slint::invoke_from_event_loop(move || {
                with_window(id, |app| app.update_titlebar_geometry());
            })
            .ok();
        }
        if let (slint::RenderingState::RenderingSetup, slint::GraphicsAPI::WGPU30 { device, queue, .. }) = (state, api)
        {
            let (device, queue) = (device.clone(), queue.clone());
            // Defer: the notifier runs mid-render, where touching UI
            // properties (or resizing the frame) isn't allowed.
            slint::invoke_from_event_loop(move || {
                with_window(id, |app| {
                    if custom_titlebar {
                        app.install_titlebar();
                    }
                    app.on_gpu_ready(device, queue);
                });
            })
            .ok();
        }
    })?;

    ui.on_content_resized(move || {
        with_window(id, |app| app.relayout());
    });
    ui.on_key_pressed(move |text, ctrl, alt, shift, meta| {
        // Slint reports ⌘ as "control" and Ctrl as "meta" on macOS (like
        // Qt). The terminal needs the real Ctrl, so swap them back: our
        // `ctrl` is always the Ctrl key, `meta` is ⌘ (or the Windows key).
        let (ctrl, meta) = if cfg!(target_os = "macos") { (meta, ctrl) } else { (ctrl, meta) };
        with_window(id, |app| app.on_key(&text, Mods { shift, alt, ctrl, meta })).unwrap_or(false)
    });
    ui.on_select_tab(move |i| {
        with_window(id, |app| app.select_tab(i as usize));
    });
    ui.on_close_tab(move |i| {
        with_window(id, |app| app.close_tab(i as usize));
    });
    ui.on_new_tab(move || {
        with_window(id, |app| app.new_tab(app.settings.default_profile_index()));
    });
    ui.on_menu_activated(move |i| {
        with_window(id, |app| app.on_menu(i as usize));
    });
    ui.on_dialog_result(move |ok| {
        with_window(id, |app| app.on_dialog_result(ok));
    });
    ui.on_minimize_window(move || {
        with_window(id, |app| app.ui().window().set_minimized(true));
    });
    ui.on_close_window(move || {
        if with_window(id, |app| app.on_close_requested()) == Some(slint::CloseRequestResponse::HideWindow) {
            close_window(id);
        }
    });
    ui.window().on_close_requested(move || {
        let response =
            with_window(id, |app| app.on_close_requested()).unwrap_or(slint::CloseRequestResponse::HideWindow);
        if response == slint::CloseRequestResponse::HideWindow {
            close_window(id);
        }
        response
    });
    ui.on_pane_menu(move |i, action| {
        with_window(id, |app| app.pane_menu(i.max(0) as usize, &action));
    });
    ui.on_tab_menu(move |i, action| {
        with_window(id, |app| app.tab_menu(i.max(0) as usize, &action));
    });
    ui.on_tab_renamed(move |i, name| {
        with_window(id, |app| app.tab_renamed(i.max(0) as usize, &name));
    });
    ui.on_tab_drag(move |i, strip_x, width, x, y| {
        with_window(id, |app| app.tab_drag(i.max(0) as usize, strip_x, width, x, y));
    });
    ui.on_tab_drag_end(move || {
        with_window(id, |app| app.tab_drag_end());
    });
    ui.on_divider_drag(move |i, x, y| {
        with_window(id, |app| app.on_divider_drag(i.max(0) as usize, x, y));
    });
    ui.on_pane_scrollbar(move |i, top| {
        with_window(id, |app| app.on_scrollbar(i.max(0) as usize, top));
    });
    ui.on_palette_changed(move |q| {
        with_window(id, |app| app.palette_changed(&q));
    });
    ui.on_palette_run(move |i| {
        with_window(id, |app| app.palette_run(i.max(0) as usize));
    });
    ui.on_palette_close(move || {
        with_window(id, |app| app.palette_close());
    });
    ui.on_find_changed(move |q| {
        with_window(id, |app| app.find_changed(&q));
    });
    ui.on_find_next(move |down| {
        with_window(id, |app| app.find_next(down));
    });
    ui.on_find_close(move || {
        with_window(id, |app| app.find_close());
    });
    ui.on_find_options(move |case, regex| {
        with_window(id, |app| app.find_options(case, regex));
    });
    ui.on_pane_pointer(move |i, kind, button, x, y, shift, ctrl, alt| {
        let mods = Mods { shift, alt, ctrl, meta: false };
        with_window(id, |app| app.on_pointer(i as usize, kind == 0, button as u8, x, y, mods));
    });
    ui.on_pane_hover(move |i, x, y, ctrl| {
        with_window(id, |app| app.on_pointer_hover(i as usize, x, y, ctrl));
    });
    ui.on_pane_moved(move |i, x, y| {
        with_window(id, |app| app.on_pointer_moved(i as usize, x, y));
    });
    ui.on_pane_scroll(move |i, dy, ctrl| {
        with_window(id, |app| app.on_scroll(i as usize, dy, ctrl)).unwrap_or(false)
    });
    {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
        // Dropping several files sends one event per file: collect them and
        // paste once, after the batch.
        let dropped: Rc<RefCell<Vec<std::path::PathBuf>>> = Rc::default();
        ui.window().on_winit_window_event(move |window, event| {
            match event {
                WindowEvent::Focused(active) => {
                    let active = *active;
                    slint::invoke_from_event_loop(move || {
                        if active {
                            window_focused(id);
                        }
                        with_window(id, |app| app.on_window_focus(active));
                    })
                    .ok();
                }
                WindowEvent::DroppedFile(path) => {
                    let first = dropped.borrow().is_empty();
                    dropped.borrow_mut().push(path.clone());
                    if first {
                        // Drops carry no position (and no mouse moves arrive
                        // during one), so ask the OS where the pointer is.
                        let at = platform::cursor_pos().zip(client_origin(window)).map(|((x, y), (left, top))| {
                            let scale = window.scale_factor();
                            ((x - left) as f32 / scale, (y - top) as f32 / scale)
                        });
                        let dropped = dropped.clone();
                        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                            let paths = std::mem::take(&mut *dropped.borrow_mut());
                            with_window(id, |app| app.on_files_dropped(paths, at));
                        });
                    }
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }
    wire_settings(ui, id);
    Ok(())
}
