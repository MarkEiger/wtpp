//! A new shell's environment on Windows: built afresh from the registry, so
//! a variable changed since wt++ started (PATH, after installing something)
//! reaches new panes without restarting wt++. Like Windows Terminal's
//! `compatibility.reloadEnvironmentVariables`. Elsewhere, shells inherit
//! wt++'s environment as before.

use std::sync::OnceLock;

/// Environment variables, `(name, value)`.
pub type Vars = Vec<(String, String)>;

fn find<'a>(vars: &'a [(String, String)], name: &str) -> Option<&'a str> {
    vars.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// Variables that describe the terminal wt++ was started from, not wt++.
const NOT_OURS: [&str; 2] = ["WT_SESSION", "WT_PROFILE_ID"];

/// A new pane's environment: `fresh` (what Windows would give a new logon:
/// the system's and the user's variables, as they're set now), plus what
/// only wt++'s own `process` has (set by whatever started wt++). Not
/// carried over: what `at_start` (the registry's set when wt++ started) had
/// and `fresh` doesn't (deleted since, so gone for new shells too), the
/// hidden per-drive directories (`=C:`, wt++'s own), and the variables of a
/// terminal wt++ was started from. Sorted by name, as Windows likes it.
pub fn merge(fresh: Vars, process: Vars, at_start: Option<&[(String, String)]>) -> Vars {
    let mut vars = fresh;
    for (name, value) in process {
        let carried = !name.starts_with('=')
            && !NOT_OURS.iter().any(|n| n.eq_ignore_ascii_case(&name))
            && find(&vars, &name).is_none()
            && at_start.is_none_or(|start| find(start, &name).is_none());
        if carried {
            vars.push((name, value));
        }
    }
    vars.sort_by_key(|(k, _)| k.to_uppercase());
    vars
}

static AT_START: OnceLock<Option<Vars>> = OnceLock::new();

/// Note the registry's environment as it is now: call when wt++ starts.
pub fn remember_startup_environment() {
    AT_START.get_or_init(registry_environment);
}

/// The environment for a new pane (see [`merge`]); `None` where it isn't
/// built afresh, or couldn't be.
pub fn pane_environment() -> Option<Vars> {
    let fresh = registry_environment()?;
    let at_start = AT_START.get_or_init(|| None).as_deref();
    let process = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))).collect();
    Some(merge(fresh, process, at_start))
}

/// What Windows gives a new logon of this user: CreateEnvironmentBlock,
/// without inheriting from wt++ (system, user and volatile variables, Path
/// joined, `REG_EXPAND_SZ` expanded).
#[cfg(windows)]
pub fn registry_environment() -> Option<Vars> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY};
    use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: our own process's token, closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE, &mut token) } == 0 {
        log::warn!("can't open our token: {}", std::io::Error::last_os_error());
        return None;
    }
    let mut block: *mut std::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `block` receives a block freed with DestroyEnvironmentBlock.
    let ok = unsafe { CreateEnvironmentBlock(&mut block, token, 0) } != 0;
    // SAFETY: opened above.
    unsafe { CloseHandle(token) };
    if !ok || block.is_null() {
        log::warn!("can't build an environment: {}", std::io::Error::last_os_error());
        return None;
    }
    // SAFETY: a block of NUL-terminated strings, ended by an empty one.
    let vars = unsafe { parse_block(block as *const u16) };
    // SAFETY: from CreateEnvironmentBlock, freed once.
    unsafe { DestroyEnvironmentBlock(block) };
    Some(vars)
}

#[cfg(not(windows))]
pub fn registry_environment() -> Option<Vars> {
    None
}

/// `name=value\0…\0\0`, UTF-16.
#[cfg(windows)]
unsafe fn parse_block(mut p: *const u16) -> Vars {
    let mut vars = Vec::new();
    loop {
        let mut len = 0;
        // SAFETY: the caller's block is terminated.
        while unsafe { *p.add(len) } != 0 {
            len += 1;
        }
        if len == 0 {
            return vars;
        }
        // SAFETY: `len` characters were just read.
        let entry = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) });
        // A name may start with '=' (the per-drive directories).
        if let Some(eq) = entry[1..].find('=') {
            vars.push((entry[..eq + 1].to_string(), entry[eq + 2..].to_string()));
        }
        // SAFETY: past this string and its NUL, still in the block.
        p = unsafe { p.add(len + 1) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(list: &[(&str, &str)]) -> Vars {
        list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn fresh_values_win() {
        let fresh = vars(&[("Path", r"C:\new;C:\Windows"), ("TEMP", r"C:\T")]);
        let process = vars(&[("PATH", r"C:\Windows"), ("TEMP", r"C:\old")]);
        let start = vars(&[("Path", r"C:\Windows"), ("TEMP", r"C:\old")]);
        assert_eq!(merge(fresh.clone(), process, Some(&start)), fresh);
    }

    #[test]
    fn what_only_wtpp_has_is_carried_over() {
        let fresh = vars(&[("Path", r"C:\Windows")]);
        let process = vars(&[("Path", r"C:\dev;C:\Windows"), ("VSCMD_VER", "17"), ("=C:", r"C:\wtpp"), ("=ExitCode", "0")]);
        let start = vars(&[("Path", r"C:\Windows")]);
        // The registry's PATH, like Windows Terminal; the launcher's own variables too.
        assert_eq!(merge(fresh, process, Some(&start)), vars(&[("Path", r"C:\Windows"), ("VSCMD_VER", "17")]));
    }

    #[test]
    fn deleted_since_start_stays_deleted() {
        let fresh = vars(&[("Path", "p")]);
        let process = vars(&[("Path", "p"), ("GONE", "1"), ("LAUNCHER", "2")]);
        let start = vars(&[("Path", "p"), ("gone", "1")]);
        assert_eq!(merge(fresh.clone(), process.clone(), Some(&start)), vars(&[("LAUNCHER", "2"), ("Path", "p")]));
        // Without knowing the start, kept: better than losing a launcher's.
        assert_eq!(merge(fresh, process, None), vars(&[("GONE", "1"), ("LAUNCHER", "2"), ("Path", "p")]));
    }

    #[test]
    fn a_terminal_we_were_started_from_isnt_ours() {
        let process = vars(&[("WT_SESSION", "x"), ("wt_profile_id", "y"), ("WT_OTHER", "z")]);
        assert_eq!(merge(Vec::new(), process, None), vars(&[("WT_OTHER", "z")]));
    }

    #[test]
    fn names_compare_without_case_and_come_out_sorted() {
        let fresh = vars(&[("windir", "W"), ("ComSpec", "c"), ("Path", "p")]);
        let process = vars(&[("WINDIR", "old"), ("COMSPEC", "old"), ("aaa", "1"), ("PATH", "old")]);
        assert_eq!(merge(fresh, process, None), vars(&[("aaa", "1"), ("ComSpec", "c"), ("Path", "p"), ("windir", "W")]));
    }

    /// HKCU\Environment values changed for a test, put back exactly as they
    /// were (type and bytes, or absent) when dropped, also on a panic.
    #[cfg(windows)]
    struct UserEnvironment {
        key: windows_sys::Win32::System::Registry::HKEY,
        saved: Vec<(Vec<u16>, Option<(u32, Vec<u8>)>)>,
    }

    #[cfg(windows)]
    impl UserEnvironment {
        fn open() -> UserEnvironment {
            use windows_sys::Win32::System::Registry::*;
            let mut key = std::ptr::null_mut();
            let name: Vec<u16> = "Environment\0".encode_utf16().collect();
            // SAFETY: plain call; closed in drop.
            let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, KEY_READ | KEY_WRITE, &mut key) };
            assert_eq!(status, 0);
            UserEnvironment { key, saved: Vec::new() }
        }

        fn get(&self, name: &str) -> Option<(u32, Vec<u8>)> {
            use windows_sys::Win32::System::Registry::RegQueryValueExW;
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let (mut kind, mut size) = (0u32, 0u32);
            // SAFETY: a size query, then a buffer of that size.
            unsafe {
                if RegQueryValueExW(self.key, wide.as_ptr(), std::ptr::null(), &mut kind, std::ptr::null_mut(), &mut size) != 0 {
                    return None;
                }
                let mut data = vec![0u8; size as usize];
                assert_eq!(RegQueryValueExW(self.key, wide.as_ptr(), std::ptr::null(), &mut kind, data.as_mut_ptr(), &mut size), 0);
                data.truncate(size as usize);
                Some((kind, data))
            }
        }

        fn put(&self, name: &[u16], value: Option<&(u32, Vec<u8>)>) {
            use windows_sys::Win32::System::Registry::{RegDeleteValueW, RegSetValueExW};
            // SAFETY: `name` is NUL-terminated; `data` holds its length.
            unsafe {
                match value {
                    Some((kind, data)) => assert_eq!(RegSetValueExW(self.key, name.as_ptr(), 0, *kind, data.as_ptr(), data.len() as u32), 0),
                    None => {
                        RegDeleteValueW(self.key, name.as_ptr());
                    }
                }
            }
        }

        /// Set `name` to a string (REG_EXPAND_SZ if `expand`), remembering what it was.
        fn set(&mut self, name: &str, text: &str, expand: bool) {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            if !self.saved.iter().any(|(n, _)| *n == wide) {
                self.saved.push((wide.clone(), self.get(name)));
            }
            let data: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
            let kind = if expand { windows_sys::Win32::System::Registry::REG_EXPAND_SZ } else { windows_sys::Win32::System::Registry::REG_SZ };
            self.put(&wide, Some(&(kind, data)));
        }

        fn restore(&mut self) {
            for (name, value) in std::mem::take(&mut self.saved).into_iter().rev() {
                self.put(&name, value.as_ref());
            }
        }
    }

    #[cfg(windows)]
    impl Drop for UserEnvironment {
        fn drop(&mut self) {
            self.restore();
            // SAFETY: opened in open().
            unsafe { windows_sys::Win32::System::Registry::RegCloseKey(self.key) };
        }
    }

    /// A variable set and PATH changed after this process started reach a
    /// shell started now, through the same code panes start with.
    #[cfg(windows)]
    #[test]
    fn new_shells_see_registry_changes() {
        use crate::{GridSize, Profile, Session};
        use std::time::{Duration, Instant};
        let tag = format!("{}{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos());
        let (name, expanded, dir) = (format!("WTPP_TEST_ENV_{tag}"), format!("WTPP_TEST_ENV_{tag}_X"), format!(r"C:\wtpp-test-path-{tag}"));
        let mut registry = UserEnvironment::open();
        let path_before = registry.get("Path");
        let (path_kind, path_text) = match &path_before {
            Some((kind, data)) => {
                let wide: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                (*kind, String::from_utf16_lossy(&wide))
            }
            None => (windows_sys::Win32::System::Registry::REG_EXPAND_SZ, String::new()),
        };
        registry.set(&name, "fresh-value", false);
        registry.set(&expanded, r"%USERPROFILE%\wtpp", true);
        let new_path = if path_text.is_empty() { dir.clone() } else { format!("{};{dir}", path_text.trim_end_matches(';')) };
        registry.set("Path", &new_path, path_kind == windows_sys::Win32::System::Registry::REG_EXPAND_SZ);
        assert!(std::env::var(&name).is_err(), "this process started before");

        let vars = pane_environment().unwrap();
        assert_eq!(find(&vars, &name), Some("fresh-value"));
        assert_eq!(find(&vars, &expanded).map(str::to_string), Some(format!(r"{}\wtpp", std::env::var("USERPROFILE").unwrap())));
        assert!(find(&vars, "Path").unwrap().split(';').any(|d| d == dir));

        // /k: the shell stays, so all its output arrives.
        let mut profile = Profile::new("t", "cmd.exe", &["/d", "/k", &format!("echo [%{name}%] [%{expanded}%] & echo %PATH%")]);
        profile.base_env = Some(vars);
        profile.cwd = std::env::var("SystemRoot").ok().map(Into::into);
        let session = Session::spawn(&profile, GridSize { cols: 200, rows: 40, cell_width: 8, cell_height: 16 }, |_| {}).unwrap();
        let text = || {
            let term = session.term().lock();
            let grid = term.grid();
            (0..40)
                .map(|l| (0..200).map(|c| grid[alacritty_terminal::index::Line(l)][alacritty_terminal::index::Column(c)].c).collect::<String>().trim_end().to_string())
                .collect::<String>()
        };
        let start = Instant::now();
        while !text().contains(&dir) && start.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let shown = text();
        registry.restore();
        assert!(shown.contains("[fresh-value]"), "{shown}");
        assert!(shown.contains(r"\wtpp]"), "{shown}");
        assert!(shown.contains(&dir), "{shown}");

        // Exactly as it was.
        assert_eq!(registry.get("Path"), path_before);
        assert_eq!(registry.get(&name), None);
        assert_eq!(registry.get(&expanded), None);
        let after = registry_environment().unwrap();
        assert_eq!(find(&after, &name), None);
        assert!(!find(&after, "Path").unwrap().split(';').any(|d| d == dir));
    }

    /// What Windows hands a new logon here: the basics are all there.
    #[cfg(windows)]
    #[test]
    fn this_machines_registry_environment() {
        let fresh = registry_environment().unwrap();
        for name in ["Path", "SystemRoot", "USERPROFILE", "TEMP", "ComSpec", "APPDATA", "LOCALAPPDATA", "USERNAME", "PATHEXT"] {
            assert!(find(&fresh, name).is_some_and(|v| !v.is_empty()), "{name} missing");
        }
        // Expanded, and the system's Path comes first.
        assert!(!find(&fresh, "TEMP").unwrap().contains('%'));
        assert!(find(&fresh, "Path").unwrap().to_lowercase().contains(r"\system32"));
        assert!(fresh.iter().all(|(k, _)| !k.starts_with('=')));
        let process: Vars = std::env::vars().collect();
        let missing: Vec<&str> = process.iter().map(|(k, _)| k.as_str()).filter(|k| find(&fresh, k).is_none()).collect();
        println!("only in this process: {missing:?}");
    }
}
