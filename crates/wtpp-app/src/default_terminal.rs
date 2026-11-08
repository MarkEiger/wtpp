//! Making wt++ the system's default terminal.
//!
//! - Windows: console programs started outside a terminal (Win+R, Explorer,
//!   a shortcut) are handed to the terminal named by
//!   `HKCU\Console\%%Startup\DelegationTerminal`, a COM server implementing
//!   `ITerminalHandoff3` (from microsoft/terminal). We register ours per
//!   user and receive the console's pipes; Windows Terminal's OpenConsole
//!   does the console side, like it does for WT.
//! - Linux: the freedesktop `xdg-terminals.list`, GNOME/Cinnamon's gsettings
//!   key and KDE's `TerminalApplication` all name us.
//! - macOS has no setting for a default terminal.

/// Whether wt++ is the default terminal, as the settings page shows it.
#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // Each platform uses only some of these.
pub enum Status {
    Default,
    NotDefault,
    /// No such setting (macOS).
    Unsupported,
}

pub fn status() -> Status {
    imp::status()
}

/// Make wt++ the default terminal. Ok carries a note for the user.
pub fn make_default() -> Result<String, String> {
    imp::make_default()
}

/// Go back to what was the default before.
pub fn restore() -> Result<String, String> {
    imp::restore()
}

#[cfg(windows)]
pub use imp::{Handoff, serve};

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::io::FromRawHandle;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, E_NOINTERFACE, E_POINTER, HANDLE, S_OK,
    };
    use windows_sys::Win32::System::Com::{
        CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoInitializeEx, CoRegisterClassObject, REGCLS_MULTIPLEUSE,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW,
        RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegSetValueExW,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, INFINITE, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
        GetProcessId, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, WaitForSingleObject,
    };
    use windows_sys::core::{GUID, HRESULT};

    use super::Status;

    /// wt++'s handoff server.
    const CLSID_WTPP: GUID = GUID::from_u128(0x1b6af1d5_8948_4a5c_b2bb_7f7bf27cba6a);
    /// Windows Terminal's OpenConsole (the console side of the handoff).
    const CLSID_OPENCONSOLE: &str = "{2EACA947-7F5F-4CFA-BA87-8F7FBEEFBE69}";
    const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
    const IID_ICLASSFACTORY: GUID = GUID::from_u128(0x00000001_0000_0000_c000_000000000046);
    const IID_ITERMINALHANDOFF3: GUID = GUID::from_u128(0x6f23da90_15c5_4203_9db0_64e73f1b1b00);
    const CLASS_E_NOAGGREGATION: HRESULT = 0x8004_0110_u32 as i32;

    const STARTUP_KEY: &str = r"Console\%%Startup";
    const SAVED_KEY: &str = r"Software\wtpp\DefaultTerminal";

    fn guid_string(g: &GUID) -> String {
        format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5],
            g.data4[6], g.data4[7]
        )
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn reg_get(key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buf = vec![0u16; 512];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: `buf` holds `size` bytes.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut size,
            )
        };
        (status == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    }

    /// Set (or with `None`, delete) a string value, creating the key.
    fn reg_set(key: &str, value: &str, data: Option<&str>) -> Result<(), String> {
        let wkey = wide(key);
        let mut hkey: HKEY = std::ptr::null_mut();
        // SAFETY: valid strings and out pointer; the key is closed below.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wkey.as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_READ | KEY_WRITE,
                std::ptr::null(),
                &mut hkey,
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(format!("can't open HKCU\\{key} (error {status})"));
        }
        let wvalue = wide(value);
        // SAFETY: `hkey` is open; data is a NUL-terminated UTF-16 string.
        let status = unsafe {
            let status = match data {
                Some(data) => {
                    let data = wide(data);
                    RegSetValueExW(hkey, wvalue.as_ptr(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32)
                }
                None => RegDeleteValueW(hkey, wvalue.as_ptr()),
            };
            RegCloseKey(hkey);
            status
        };
        match status {
            0 | 2 => Ok(()), // 2: deleting a value that isn't there.
            _ => Err(format!("can't write HKCU\\{key}\\{value} (error {status})")),
        }
    }

    pub fn status() -> Status {
        if reg_get(STARTUP_KEY, "DelegationTerminal").is_some_and(|v| v.eq_ignore_ascii_case(&guid_string(&CLSID_WTPP))) {
            Status::Default
        } else {
            Status::NotDefault
        }
    }

    /// Windows Terminal (which provides OpenConsole) is installed.
    fn have_openconsole() -> bool {
        std::env::var_os("LOCALAPPDATA")
            .is_some_and(|dir| std::path::Path::new(&dir).join(r"Microsoft\WindowsApps\wt.exe").exists())
    }

    pub fn make_default() -> Result<String, String> {
        if !have_openconsole() {
            return Err("Windows Terminal must be installed: wt++ uses its console host (OpenConsole) to receive console programs.".into());
        }
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let clsid = guid_string(&CLSID_WTPP);
        let class = format!(r"Software\Classes\CLSID\{clsid}");
        reg_set(&class, "", Some("wt++ terminal"))?;
        reg_set(&format!(r"{class}\LocalServer32"), "", Some(&format!("\"{}\" -Embedding", exe.display())))?;
        // Remember the current choice (once) so it can be restored.
        if status() != Status::Default {
            for value in ["DelegationConsole", "DelegationTerminal"] {
                reg_set(SAVED_KEY, value, Some(&reg_get(STARTUP_KEY, value).unwrap_or_default()))?;
            }
        }
        reg_set(STARTUP_KEY, "DelegationConsole", Some(CLSID_OPENCONSOLE))?;
        reg_set(STARTUP_KEY, "DelegationTerminal", Some(&clsid))?;
        serve_now();
        Ok("Console programs (Command Prompt, PowerShell, …) started from the Start menu, Run or Explorer now open in wt++.".into())
    }

    pub fn restore() -> Result<String, String> {
        for value in ["DelegationConsole", "DelegationTerminal"] {
            let saved = reg_get(SAVED_KEY, value).filter(|v| !v.is_empty());
            reg_set(STARTUP_KEY, value, saved.as_deref())?;
        }
        let wkey = wide(&format!(r"Software\Classes\CLSID\{}", guid_string(&CLSID_WTPP)));
        // SAFETY: a NUL-terminated key path.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wkey.as_ptr()) };
        Ok("The previous default terminal is back.".into())
    }

    /// A console handed over by Windows. Raw handles (they cross threads);
    /// [`Handoff::into_pipes`] takes ownership.
    pub struct Handoff {
        input: usize,
        output: usize,
        signal: usize,
        reference: usize,
        server: usize,
        client: usize,
    }

    impl Handoff {
        /// The client program's path, e.g. `C:\Windows\System32\cmd.exe`.
        pub fn program(&self) -> Option<String> {
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            // SAFETY: `client` is a process handle we own; `buf` holds `len`.
            let ok = unsafe {
                QueryFullProcessImageNameW(self.client as HANDLE, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
            };
            (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }

        pub fn into_pipes(self) -> wtpp_session::Pipes {
            // SAFETY: each handle is ours, open, and owned by exactly one File.
            let file = |h: usize| unsafe { File::from_raw_handle(h as *mut c_void) };
            let client = self.client;
            wtpp_session::Pipes {
                output: file(self.output),
                input: file(self.input),
                signal: file(self.signal),
                // SAFETY: `client` is a process handle we own.
                pid: Some(unsafe { GetProcessId(self.client as HANDLE) }).filter(|&p| p != 0),
                wait_exit: Box::new(move || {
                    let mut code = 0u32;
                    // SAFETY: `client` stays open until this closes it.
                    unsafe {
                        WaitForSingleObject(client as HANDLE, INFINITE);
                        let ok = GetExitCodeProcess(client as HANDLE, &mut code);
                        CloseHandle(client as HANDLE);
                        (ok != 0).then_some(code as i32)
                    }
                }),
                keep_alive: vec![file(self.reference), file(self.server)],
            }
        }
    }

    static ON_HANDOFF: OnceLock<fn(Handoff)> = OnceLock::new();
    static SERVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// Accept handoffs (when wt++ is the default terminal, or `force`d for a
    /// `-Embedding` launch), passing each to `on_handoff` on the UI thread.
    pub fn serve(on_handoff: fn(Handoff), force: bool) {
        let _ = ON_HANDOFF.set(on_handoff);
        if force || super::status() == Status::Default {
            serve_now();
        }
    }

    fn serve_now() {
        if ON_HANDOFF.get().is_none() || SERVING.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return;
        }
        // Calls arrive on COM's own threads (multithreaded apartment), away
        // from the UI thread's apartment.
        std::thread::spawn(|| {
            let mut cookie = 0u32;
            // SAFETY: COM initialization on this thread, then registering a
            // static class object that lives for the process.
            let hr = unsafe {
                CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32);
                CoRegisterClassObject(
                    &CLSID_WTPP,
                    &FACTORY as *const ComObject as *mut c_void,
                    CLSCTX_LOCAL_SERVER,
                    REGCLS_MULTIPLEUSE as u32,
                    &mut cookie,
                )
            };
            if hr != S_OK {
                log::error!("can't serve as the default terminal: 0x{:08X}", hr as u32);
                return;
            }
            log::info!("accepting console handoffs");
            // Keep the apartment (and the registration) alive.
            loop {
                std::thread::park();
            }
        });
    }

    // ---- COM objects: static, so reference counting is a no-op ----

    #[repr(C)]
    struct ComObject {
        vtbl: *const c_void,
    }
    // SAFETY: the objects are immutable statics.
    unsafe impl Sync for ComObject {}

    #[repr(C)]
    struct FactoryVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        create_instance: unsafe extern "system" fn(*mut c_void, *mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        lock_server: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    }

    #[repr(C)]
    struct HandoffVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        establish_pty_handoff: unsafe extern "system" fn(
            *mut c_void,
            *mut HANDLE,
            *mut HANDLE,
            HANDLE,
            HANDLE,
            HANDLE,
            HANDLE,
            *const c_void,
        ) -> HRESULT,
    }

    static FACTORY_VTBL: FactoryVtbl = FactoryVtbl {
        query_interface: factory_query_interface,
        add_ref,
        release,
        create_instance,
        lock_server,
    };
    static HANDOFF_VTBL: HandoffVtbl =
        HandoffVtbl { query_interface: handoff_query_interface, add_ref, release, establish_pty_handoff };
    static FACTORY: ComObject = ComObject { vtbl: &FACTORY_VTBL as *const FactoryVtbl as *const c_void };
    static HANDOFF: ComObject = ComObject { vtbl: &HANDOFF_VTBL as *const HandoffVtbl as *const c_void };

    fn guid_eq(a: *const GUID, b: &GUID) -> bool {
        // SAFETY: COM passes a valid IID pointer.
        let a = unsafe { &*a };
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

    unsafe fn answer(out: *mut *mut c_void, object: Option<&'static ComObject>) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        // SAFETY: `out` is a valid out pointer (checked above).
        unsafe {
            match object {
                Some(o) => {
                    *out = o as *const ComObject as *mut c_void;
                    S_OK
                }
                None => {
                    *out = std::ptr::null_mut();
                    E_NOINTERFACE
                }
            }
        }
    }

    unsafe extern "system" fn factory_query_interface(_: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        let ok = guid_eq(iid, &IID_IUNKNOWN) || guid_eq(iid, &IID_ICLASSFACTORY);
        // SAFETY: forwarded COM out pointer.
        unsafe { answer(out, ok.then_some(&FACTORY)) }
    }

    unsafe extern "system" fn handoff_query_interface(_: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        let ok = guid_eq(iid, &IID_IUNKNOWN) || guid_eq(iid, &IID_ITERMINALHANDOFF3);
        // SAFETY: forwarded COM out pointer.
        unsafe { answer(out, ok.then_some(&HANDOFF)) }
    }

    unsafe extern "system" fn add_ref(_: *mut c_void) -> u32 {
        1
    }

    unsafe extern "system" fn release(_: *mut c_void) -> u32 {
        1
    }

    unsafe extern "system" fn create_instance(
        _: *mut c_void,
        outer: *mut c_void,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        if !outer.is_null() {
            return CLASS_E_NOAGGREGATION;
        }
        // SAFETY: forwarded COM arguments.
        unsafe { handoff_query_interface(std::ptr::null_mut(), iid, out) }
    }

    unsafe extern "system" fn lock_server(_: *mut c_void, _: i32) -> HRESULT {
        S_OK
    }

    /// `ITerminalHandoff3::EstablishPtyHandoff`: we create the input/output
    /// pipes and return the console's ends; the other handles are borrowed,
    /// so we keep duplicates.
    #[allow(clippy::too_many_arguments)]
    unsafe extern "system" fn establish_pty_handoff(
        _: *mut c_void,
        input: *mut HANDLE,
        output: *mut HANDLE,
        signal: HANDLE,
        reference: HANDLE,
        server: HANDLE,
        client: HANDLE,
        _startup_info: *const c_void,
    ) -> HRESULT {
        if input.is_null() || output.is_null() {
            return E_POINTER;
        }
        let Some(on_handoff) = ON_HANDOFF.get().copied() else { return E_POINTER };
        // SAFETY: pipe and handle calls on valid handles and out pointers.
        unsafe {
            let (mut console_reads, mut we_write) = (std::ptr::null_mut(), std::ptr::null_mut());
            let (mut we_read, mut console_writes) = (std::ptr::null_mut(), std::ptr::null_mut());
            if CreatePipe(&mut console_reads, &mut we_write, std::ptr::null(), 0) == 0 {
                return windows_sys::Win32::Foundation::E_FAIL;
            }
            if CreatePipe(&mut we_read, &mut console_writes, std::ptr::null(), 0) == 0 {
                CloseHandle(console_reads);
                CloseHandle(we_write);
                return windows_sys::Win32::Foundation::E_FAIL;
            }
            let me = GetCurrentProcess();
            let dup = |h: HANDLE| -> usize {
                let mut copy = std::ptr::null_mut();
                DuplicateHandle(me, h, me, &mut copy, 0, 0, DUPLICATE_SAME_ACCESS);
                copy as usize
            };
            // The client handle needs rights to wait on it and read its exit
            // code and path, which the one we're lent may lack (like WT).
            let mut client_copy = std::ptr::null_mut();
            let access = PROCESS_SYNCHRONIZE | PROCESS_QUERY_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION;
            if DuplicateHandle(me, client, me, &mut client_copy, access, 0, 0) == 0 {
                client_copy = dup(client) as HANDLE;
            }
            let handoff = Handoff {
                input: we_write as usize,
                output: we_read as usize,
                signal: dup(signal),
                reference: dup(reference),
                server: dup(server),
                client: client_copy as usize,
            };
            // COM marshals these into the console host (like WT, we hand
            // them over rather than closing them).
            *input = console_reads;
            *output = console_writes;
            slint::invoke_from_event_loop(move || on_handoff(handoff)).ok();
        }
        S_OK
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::path::PathBuf;
    use std::process::Command;

    use super::Status;

    const DESKTOP_ID: &str = "wtpp.desktop";

    fn config_home() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
    }

    fn data_home() -> PathBuf {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"))
    }

    /// The freedesktop default-terminal list (xdg-terminal-exec), which
    /// GNOME and others follow: the first entry wins.
    fn terminals_list() -> PathBuf {
        config_home().join("xdg-terminals.list")
    }

    pub fn status() -> Status {
        let first = std::fs::read_to_string(terminals_list())
            .ok()
            .and_then(|s| s.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).map(String::from));
        if first.as_deref() == Some(DESKTOP_ID) { Status::Default } else { Status::NotDefault }
    }

    /// xdg-terminal-exec caches its choice; drop it so a change applies now.
    fn forget_cached_terminal() {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"));
        std::fs::remove_file(cache.join("xdg-terminal-exec")).ok();
    }

    const GSETTINGS: [&str; 2] =
        ["org.gnome.desktop.default-applications.terminal", "org.cinnamon.desktop.default-applications.terminal"];

    /// The terminal a gsettings schema names, if the schema exists.
    fn gsetting(schema: &str) -> Option<String> {
        let out = Command::new("gsettings").args(["get", schema, "exec"]).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string())
    }

    fn run(program: &str, args: &[&str]) -> bool {
        Command::new(program).args(args).output().is_ok_and(|o| o.status.success())
    }

    fn output(program: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(program).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// What make_default changed, so restore can put it back exactly.
    fn saved_path() -> PathBuf {
        config_home().join("wtpp").join("default-terminal-previous.json")
    }

    /// Marks a desktop entry wt++ wrote itself (not, say, the installer's).
    const OURS: &str = "X-Wtpp-Default-Terminal=true";

    fn on_kde() -> bool {
        std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("KDE"))
    }

    fn kde_tools() -> Option<(&'static str, &'static str)> {
        [("kwriteconfig6", "kreadconfig6"), ("kwriteconfig5", "kreadconfig5")]
            .into_iter()
            .find(|(w, _)| Command::new(w).arg("--help").output().is_ok())
    }

    pub fn make_default() -> Result<String, String> {
        use serde_json::{Value, json};
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut saved: Value =
            std::fs::read(saved_path()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or(json!({}));

        // A desktop entry to name: the installed one if there is one (keep
        // it: it may point at the installed copy), else our own.
        let apps = data_home().join("applications");
        let entry = apps.join(DESKTOP_ID);
        if !entry.exists() {
            std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
            let desktop = format!(
                "[Desktop Entry]\nType=Application\nName=wt++\nGenericName=Terminal\nComment=GPU-accelerated terminal with tabs and panes\nExec={exe}\nTryExec={exe}\nIcon=wtpp\nTerminal=false\nCategories=System;TerminalEmulator;\nStartupWMClass=wtpp\nX-TerminalArgExec=-e\nX-TerminalArgDir=--working-directory\n{OURS}\n",
                exe = exe.display()
            );
            std::fs::write(&entry, desktop).map_err(|e| e.to_string())?;
        }

        // Put us first in xdg-terminals.list, keeping the rest.
        let list = terminals_list();
        let old = std::fs::read_to_string(&list).ok();
        if saved.get("list_existed").is_none() {
            saved["list_existed"] = json!(old.is_some());
        }
        let old = old.unwrap_or_default();
        let rest: Vec<&str> = old.lines().filter(|l| l.trim() != DESKTOP_ID).collect();
        std::fs::create_dir_all(config_home()).map_err(|e| e.to_string())?;
        std::fs::write(&list, format!("{DESKTOP_ID}\n{}", rest.join("\n"))).map_err(|e| e.to_string())?;
        forget_cached_terminal();

        let exe = exe.display().to_string();
        // Older GNOME and Cinnamon read gsettings (newer GNOME points it at
        // xdg-terminal-exec, which follows the list above: leave that alone).
        for schema in GSETTINGS {
            if let Some(previous) = gsetting(schema).filter(|v| v != "xdg-terminal-exec" && *v != exe)
                && run("gsettings", &["set", schema, "exec", &exe])
            {
                let arg = output("gsettings", &["get", schema, "exec-arg"]).unwrap_or_default();
                saved["gsettings"][schema] = json!({ "exec": previous, "exec-arg": arg.trim_matches('\'') });
                run("gsettings", &["set", schema, "exec-arg", "-e"]);
            }
        }
        // KDE reads kdeglobals (only on KDE: other desktops may have the
        // tools installed too).
        if on_kde()
            && let Some((write, read)) = kde_tools()
        {
            let get = |key| output(read, &["--file", "kdeglobals", "--group", "General", "--key", key]);
            if saved.get("kde").is_none() {
                saved["kde"] = json!({ "app": get("TerminalApplication"), "service": get("TerminalService") });
            }
            run(write, &["--file", "kdeglobals", "--group", "General", "--key", "TerminalApplication", &exe]);
            run(write, &["--file", "kdeglobals", "--group", "General", "--key", "TerminalService", DESKTOP_ID]);
        }
        if let Some(dir) = saved_path().parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(saved_path(), saved.to_string()).map_err(|e| e.to_string())?;
        Ok("wt++ is now your desktop's default terminal.".into())
    }

    pub fn restore() -> Result<String, String> {
        use serde_json::Value;
        let saved: Value =
            std::fs::read(saved_path()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();

        let list = terminals_list();
        if let Ok(old) = std::fs::read_to_string(&list) {
            let rest: Vec<&str> = old.lines().filter(|l| l.trim() != DESKTOP_ID).collect();
            // An empty list (that wasn't there before) would still override
            // the desktop's own choice.
            if rest.iter().all(|l| l.trim().is_empty()) && saved["list_existed"] != Value::Bool(true) {
                std::fs::remove_file(&list).map_err(|e| e.to_string())?;
            } else {
                std::fs::write(&list, rest.join("\n")).map_err(|e| e.to_string())?;
            }
        }
        forget_cached_terminal();

        // Our own desktop entry only; an installed one stays.
        let entry = data_home().join("applications").join(DESKTOP_ID);
        if std::fs::read_to_string(&entry).is_ok_and(|e| e.contains(OURS)) {
            std::fs::remove_file(&entry).ok();
        }
        if let Some(schemas) = saved["gsettings"].as_object() {
            for (schema, previous) in schemas {
                if let Some(exec) = previous["exec"].as_str() {
                    run("gsettings", &["set", schema, "exec", exec]);
                }
                if let Some(arg) = previous["exec-arg"].as_str() {
                    run("gsettings", &["set", schema, "exec-arg", arg]);
                }
            }
        }
        if saved.get("kde").is_some()
            && let Some((write, _)) = kde_tools()
        {
            for (key, field) in [("TerminalApplication", "app"), ("TerminalService", "service")] {
                match saved["kde"][field].as_str().filter(|v| !v.is_empty()) {
                    Some(value) => run(write, &["--file", "kdeglobals", "--group", "General", "--key", key, value]),
                    None => run(write, &["--file", "kdeglobals", "--group", "General", "--key", key, "--delete"]),
                };
            }
        }
        std::fs::remove_file(saved_path()).ok();
        Ok("The previous default terminal is back.".into())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Status;

    const WHY: &str = "macOS has no setting for a default terminal.";

    pub fn status() -> Status {
        Status::Unsupported
    }

    pub fn make_default() -> Result<String, String> {
        Err(WHY.into())
    }

    pub fn restore() -> Result<String, String> {
        Err(WHY.into())
    }
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn linux_default_and_restore() {
        let dir = std::env::temp_dir().join(format!("wtpp-default-term-{}", std::process::id()));
        let (config, data) = (dir.join("config"), dir.join("data"));
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("xdg-terminals.list"), "kitty.desktop\n").unwrap();
        // SAFETY: no other test reads these variables.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &config);
            std::env::set_var("XDG_DATA_HOME", &data);
        }
        assert_eq!(status(), Status::NotDefault);

        make_default().unwrap();
        assert_eq!(status(), Status::Default);
        let list = std::fs::read_to_string(config.join("xdg-terminals.list")).unwrap();
        assert_eq!(list.lines().collect::<Vec<_>>(), ["wtpp.desktop", "kitty.desktop"]);
        let desktop = std::fs::read_to_string(data.join("applications/wtpp.desktop")).unwrap();
        assert!(desktop.contains("Categories=System;TerminalEmulator;"));
        assert!(desktop.contains("X-TerminalArgExec=-e"));

        restore().unwrap();
        assert_eq!(status(), Status::NotDefault);
        let list = std::fs::read_to_string(config.join("xdg-terminals.list")).unwrap();
        assert_eq!(list.trim(), "kitty.desktop");

        // Our own desktop entry goes with it; an installed one stays as it was.
        assert!(!data.join("applications/wtpp.desktop").exists());
        std::fs::create_dir_all(data.join("applications")).unwrap();
        std::fs::write(data.join("applications/wtpp.desktop"), "[Desktop Entry]\nExec=/opt/wtpp\n").unwrap();
        make_default().unwrap();
        restore().unwrap();
        assert_eq!(std::fs::read_to_string(data.join("applications/wtpp.desktop")).unwrap(), "[Desktop Entry]\nExec=/opt/wtpp\n");
        std::fs::remove_file(data.join("applications/wtpp.desktop")).unwrap();

        // A list that only had us is removed, so the desktop's default applies again.
        std::fs::remove_file(config.join("xdg-terminals.list")).unwrap();
        make_default().unwrap();
        restore().unwrap();
        assert!(!config.join("xdg-terminals.list").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
