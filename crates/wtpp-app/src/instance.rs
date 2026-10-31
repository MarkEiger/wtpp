//! One process for all windows, like Windows Terminal: launching wt++ again
//! hands its command line to the running instance, which opens the window.
//! That's what lets tabs be dragged between any two windows.
//!
//! The first process listens on a per-user named pipe (Windows) or Unix
//! socket; later ones send `{"args": [...], "cwd": "..."}` and exit.
//! `WTPP_INSTANCE` names a separate instance (tests use it so they never
//! reach the user's windows).

use std::io::{Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
}

/// Either we're the instance (and own the listener), or the request went to
/// the running one.
pub enum Claim {
    Primary(Listener),
    Forwarded,
}

/// Become the instance, or hand `request` to the one that's running.
pub fn claim(request: &Request) -> Claim {
    let name = endpoint_name();
    for _ in 0..3 {
        if imp::send(&name, &serde_json::to_vec(request).unwrap_or_default()) {
            return Claim::Forwarded;
        }
        match imp::listen(&name) {
            Some(listener) => return Claim::Primary(Listener(Some(listener))),
            // Another process won the race to listen: send to it instead.
            None => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    log::warn!("couldn't reach or become the wt++ instance; running on our own");
    Claim::Primary(Listener(None))
}

pub struct Listener(Option<imp::Endpoint>);

impl Listener {
    /// Hand each incoming request to `on_request` on the UI thread.
    pub fn serve(self, on_request: fn(Request)) {
        let Some(endpoint) = self.0 else { return };
        std::thread::spawn(move || {
            imp::accept_loop(endpoint, |bytes| match serde_json::from_slice::<Request>(&bytes) {
                Ok(request) => {
                    slint::invoke_from_event_loop(move || on_request(request)).ok();
                }
                Err(e) => log::warn!("bad request from another wt++: {e}"),
            });
        });
    }
}

fn endpoint_name() -> String {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let instance = std::env::var("WTPP_INSTANCE").unwrap_or_else(|_| "default".into());
    let clean = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect::<String>();
    format!("wtpp-{}-{}", clean(&user), clean(&instance))
}

fn read_all(mut r: impl Read) -> Vec<u8> {
    let mut bytes = Vec::new();
    r.read_to_end(&mut bytes).ok();
    bytes
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};

    use windows_sys::Win32::Foundation::{ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};

    use super::*;

    pub type Endpoint = OwnedHandle;

    fn path(name: &str) -> String {
        format!(r"\\.\pipe\{name}")
    }

    pub fn send(name: &str, message: &[u8]) -> bool {
        for _ in 0..40 {
            match std::fs::OpenOptions::new().write(true).open(path(name)) {
                Ok(mut pipe) => {
                    // Let the instance bring its new window to the front:
                    // we have the foreground rights (the user launched us).
                    // SAFETY: no preconditions.
                    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
                    return pipe.write_all(message).is_ok();
                }
                // The instance is between two clients.
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(_) => return false,
            }
        }
        false
    }

    pub fn listen(name: &str) -> Option<Endpoint> {
        let wide: Vec<u16> = path(name).encode_utf16().chain([0]).collect();
        // SAFETY: `wide` is NUL-terminated; FIRST_PIPE_INSTANCE makes this
        // fail if another process already owns the name.
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        // SAFETY: a valid handle we now own.
        (handle != INVALID_HANDLE_VALUE).then(|| unsafe { OwnedHandle::from_raw_handle(handle) })
    }

    pub fn accept_loop(endpoint: Endpoint, mut on_message: impl FnMut(Vec<u8>)) {
        use std::os::windows::io::AsRawHandle;
        let handle = endpoint.as_raw_handle();
        loop {
            // SAFETY: `handle` stays open for the life of `endpoint`.
            let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } != 0
                || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if connected {
                // Read through a borrowed File; the handle stays ours.
                let file = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(handle) });
                on_message(read_all(&*file));
            }
            // SAFETY: as above.
            unsafe { DisconnectNamedPipe(handle) };
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::os::unix::net::{UnixListener, UnixStream};

    use super::*;

    pub type Endpoint = UnixListener;

    fn path(name: &str) -> PathBuf {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        dir.join(format!("{name}.sock"))
    }

    pub fn send(name: &str, message: &[u8]) -> bool {
        UnixStream::connect(path(name)).and_then(|mut s| s.write_all(message)).is_ok()
    }

    pub fn listen(name: &str) -> Option<Endpoint> {
        let path = path(name);
        // Nobody answered on it (`send` failed), so it's left over from a
        // crashed instance.
        std::fs::remove_file(&path).ok();
        UnixListener::bind(&path).map_err(|e| log::warn!("can't listen on {}: {e}", path.display())).ok()
    }

    pub fn accept_loop(endpoint: Endpoint, mut on_message: impl FnMut(Vec<u8>)) {
        for stream in endpoint.incoming().flatten() {
            on_message(read_all(stream));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip() {
        let request = Request { args: vec!["-d".into(), "C:\\code".into()], cwd: Some(PathBuf::from("/tmp")) };
        let bytes = serde_json::to_vec(&request).unwrap();
        assert_eq!(serde_json::from_slice::<Request>(&bytes).unwrap(), request);
    }

    #[cfg(unix)]
    #[test]
    fn second_launch_is_forwarded() {
        // SAFETY: tests in this module don't read the variable concurrently.
        unsafe { std::env::set_var("WTPP_INSTANCE", format!("test-{}", std::process::id())) };
        let request = Request { args: vec!["new-tab".into()], cwd: None };
        let Claim::Primary(Listener(Some(endpoint))) = claim(&request) else { panic!("should be primary") };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || imp::accept_loop(endpoint, |m| tx.send(m).unwrap()));
        assert!(matches!(claim(&request), Claim::Forwarded));
        let got: Request = serde_json::from_slice(&rx.recv().unwrap()).unwrap();
        assert_eq!(got, request);
    }
}
