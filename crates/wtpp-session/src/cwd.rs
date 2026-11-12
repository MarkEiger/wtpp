//! A pane's current directory: what its shell reports (OSC 7, or Windows
//! Terminal's OSC 9;9), or the shell process's own.

use std::path::{Path, PathBuf};

/// The longest OSC we keep: a path, with room to spare. Longer ones (images,
/// big clipboard writes) are skipped, not collected.
const MAX_OSC: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Esc,
    Osc,
    /// ESC inside an OSC: `\` ends it.
    OscEsc,
    /// An OSC too long to be a directory: skipped until it ends.
    Skip,
    SkipEsc,
}

/// Finds directory reports (and other OSCs, see [`CwdScanner::feed_osc`])
/// in a raw output stream, read in any pieces.
#[derive(Debug, Default)]
pub struct CwdScanner {
    state: State,
    osc: Vec<u8>,
}

/// How a directory was reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CwdKind {
    /// OSC 7: what shells call the directory, symbolic links and all.
    Osc7,
    /// OSC 9;9 (Windows Terminal's).
    Osc99,
}

/// What a shell has reported about its directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reported {
    /// The last report, of either kind.
    pub last: Option<String>,
    /// The last OSC 9;9.
    pub osc99: Option<String>,
    /// An OSC 7 sent since that OSC 9;9: the same directory by the name the
    /// shell shows (wt++'s bash hook sends 9;9 without symbolic links, then 7).
    pub logical: Option<String>,
}

impl Reported {
    pub fn record(&mut self, kind: CwdKind, path: String) {
        match kind {
            CwdKind::Osc99 => {
                self.osc99 = Some(path.clone());
                self.logical = None;
            }
            CwdKind::Osc7 => self.logical = Some(path.clone()),
        }
        self.last = Some(path);
    }
}

impl CwdScanner {
    /// The directories reported in `bytes`, in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<(CwdKind, String)> {
        self.feed_osc(bytes).iter().filter_map(|osc| parse_cwd_report(osc)).collect()
    }

    /// The text of every OSC that ends in `bytes` (up to [`MAX_OSC`] long).
    pub fn feed_osc(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut found = Vec::new();
        for &b in bytes {
            self.state = match (self.state, b) {
                (State::Ground, 0x1b) => State::Esc,
                (State::Ground, _) => State::Ground,
                (State::Esc, b']') => {
                    self.osc.clear();
                    State::Osc
                }
                (State::Esc, 0x1b) => State::Esc,
                (State::Esc, _) => State::Ground,
                (State::Osc | State::Skip, 0x07) => {
                    if self.state == State::Osc {
                        found.push(self.osc.clone());
                    }
                    State::Ground
                }
                (State::Osc, 0x1b) => State::OscEsc,
                (State::Skip, 0x1b) => State::SkipEsc,
                // CAN and SUB cancel a sequence.
                (State::Osc | State::Skip, 0x18 | 0x1a) => State::Ground,
                (State::Osc, _) if self.osc.len() >= MAX_OSC => State::Skip,
                (State::Osc, b) => {
                    self.osc.push(b);
                    State::Osc
                }
                (State::Skip, _) => State::Skip,
                (State::OscEsc | State::SkipEsc, b'\\') => {
                    if self.state == State::OscEsc {
                        found.push(self.osc.clone());
                    }
                    State::Ground
                }
                // Not a string terminator: the ESC starts something new.
                (State::OscEsc | State::SkipEsc, b']') => {
                    self.osc.clear();
                    State::Osc
                }
                (State::OscEsc | State::SkipEsc, 0x1b) => State::Esc,
                (State::OscEsc | State::SkipEsc, _) => State::Ground,
            };
        }
        found
    }
}

/// The directory in an OSC's text (between `ESC ]` and its terminator):
/// `7;file://host/path` (percent-encoded; a host other than this machine
/// means another machine's directory, as over ssh) or `9;9;path` (Windows
/// Terminal's, maybe quoted). Windows paths come back as `C:\dir`.
pub fn parse_cwd_osc(osc: &[u8]) -> Option<String> {
    parse_cwd_report(osc).map(|(_, path)| path)
}

/// [`parse_cwd_osc`], and which kind of report it was.
pub fn parse_cwd_report(osc: &[u8]) -> Option<(CwdKind, String)> {
    let text = std::str::from_utf8(osc).ok()?;
    let kind = if text.starts_with("7;") { CwdKind::Osc7 } else { CwdKind::Osc99 };
    let path = if let Some(url) = text.strip_prefix("7;") {
        let rest = url.strip_prefix("file://")?;
        let (host, path) = rest.split_at(rest.find('/')?);
        if !is_local_host(host) {
            return None;
        }
        percent_decode(path)?
    } else {
        let path = text.strip_prefix("9;9;")?.trim();
        path.strip_prefix('"').and_then(|p| p.strip_suffix('"')).unwrap_or(path).to_string()
    };
    // file:///C:/dir → C:\dir
    let bytes = path.as_bytes();
    let path = if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        path[1..].replace('/', "\\")
    } else {
        path
    };
    (!path.is_empty()).then_some((kind, path))
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn is_local_host(host: &str) -> bool {
    let short = |h: &str| h.split('.').next().unwrap_or(h).to_ascii_lowercase();
    host.is_empty() || host.eq_ignore_ascii_case("localhost") || local_hostname().is_some_and(|me| short(&me) == short(host))
}

fn local_hostname() -> Option<String> {
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME").ok()
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/sys/kernel/hostname").ok().map(|h| h.trim().to_string())
    }
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn gethostname(name: *mut std::ffi::c_char, len: usize) -> i32;
        }
        let mut buf = [0u8; 256];
        // SAFETY: `buf` holds `len` bytes.
        if unsafe { gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
            return None;
        }
        let len = buf.iter().position(|&b| b == 0)?;
        String::from_utf8(buf[..len].to_vec()).ok()
    }
}

/// `reported`, the name a shell gave its directory (with any symbolic
/// links), if it's the same directory as `process`'s (which has none).
pub fn same_directory_name(process: &Path, reported: Option<&str>) -> Option<String> {
    let reported = reported?;
    let same = Path::new(reported).is_absolute()
        && Path::new(reported).canonicalize().ok()? == process.canonicalize().ok()?;
    same.then(|| reported.to_string())
}

/// macOS: `/private/etc` is better known as `/etc` (a symbolic link to it),
/// and so on for `/tmp` and `/var`: the short name, where it's the same.
pub fn short_private_name(process: &Path) -> Option<String> {
    let rest = process.to_str()?.strip_prefix("/private/")?;
    same_directory_name(process, Some(&format!("/{rest}")))
}

/// Process `pid`'s working directory, as the system keeps it. Right for
/// shells that change it as they go (cmd, bash, zsh, …), not for PowerShell
/// (its location isn't the process's) or for `wsl.exe`.
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        // A directory that's been removed reads as "<path> (deleted)".
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok().filter(|dir| dir.is_dir())
    }
    #[cfg(target_os = "macos")]
    {
        macos::process_cwd(pid).filter(|dir| dir.is_dir())
    }
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        windows::process_cwd(pid)
    }
    #[cfg(all(windows, not(target_pointer_width = "64")))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;

    const PROC_PIDVNODEPATHINFO: i32 = 9;
    /// `struct vnode_info`, before each `vip_path`.
    const VNODE_INFO_SIZE: usize = 152;
    const MAXPATHLEN: usize = 1024;

    unsafe extern "C" {
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut std::ffi::c_void, size: i32) -> i32;
    }

    /// `proc_vnodepathinfo`: the current directory's `vnode_info_path`, then
    /// the root directory's.
    pub fn process_cwd(pid: u32) -> Option<PathBuf> {
        let mut info = [0u8; 2 * (VNODE_INFO_SIZE + MAXPATHLEN)];
        // SAFETY: `info` holds the struct's size, which is what's passed.
        let got = unsafe { proc_pidinfo(pid as i32, PROC_PIDVNODEPATHINFO, 0, info.as_mut_ptr().cast(), info.len() as i32) };
        if got as usize != info.len() {
            return None;
        }
        let path = &info[VNODE_INFO_SIZE..VNODE_INFO_SIZE + MAXPATHLEN];
        let len = path.iter().position(|&b| b == 0)?;
        use std::os::unix::ffi::OsStrExt;
        (len > 0).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&path[..len])))
    }
}

#[cfg(all(windows, target_pointer_width = "64"))]
pub(crate) mod windows {
    use std::ffi::c_void;
    use std::path::PathBuf;
    use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};

    /// PROCESS_BASIC_INFORMATION, of which only the PEB's address is used.
    #[repr(C)]
    struct BasicInformation {
        exit_status: i32,
        peb: usize,
        affinity_mask: usize,
        base_priority: i32,
        pid: usize,
        parent_pid: usize,
    }

    /// x64 offsets: PEB.ProcessParameters, and in RTL_USER_PROCESS_PARAMETERS,
    /// CurrentDirectory.DosPath and CommandLine (UNICODE_STRINGs).
    const PROCESS_PARAMETERS: usize = 0x20;
    const CURRENT_DIRECTORY: usize = 0x38;
    const COMMAND_LINE: usize = 0x70;

    pub(crate) struct Process(pub(crate) HANDLE);

    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: opened in `open`, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    impl Process {
        pub(crate) fn open(pid: u32) -> Option<Process> {
            // SAFETY: plain call; the handle is closed by `Process`.
            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
            (!handle.is_null()).then_some(Process(handle))
        }

        fn read<const N: usize>(&self, address: usize) -> Option<[u8; N]> {
            let mut buf = [0u8; N];
            let mut read = 0;
            // SAFETY: `buf` holds N bytes; a bad address only fails the call.
            let ok = unsafe { ReadProcessMemory(self.0, address as *const c_void, buf.as_mut_ptr().cast(), N, &mut read) };
            (ok != 0 && read == N).then_some(buf)
        }

        /// The address of the process's RTL_USER_PROCESS_PARAMETERS.
        fn parameters(&self) -> Option<usize> {
            let mut info: BasicInformation = BasicInformation { exit_status: 0, peb: 0, affinity_mask: 0, base_priority: 0, pid: 0, parent_pid: 0 };
            // SAFETY: `info` is PROCESS_BASIC_INFORMATION's size and layout.
            let status = unsafe {
                NtQueryInformationProcess(
                    self.0,
                    ProcessBasicInformation,
                    (&mut info as *mut BasicInformation).cast(),
                    size_of::<BasicInformation>() as u32,
                    std::ptr::null_mut(),
                )
            };
            if status != 0 || info.peb == 0 {
                return None;
            }
            Some(usize::from_le_bytes(self.read::<8>(info.peb + PROCESS_PARAMETERS)?))
        }

        /// The UNICODE_STRING at `address`.
        fn unicode_string(&self, address: usize) -> Option<String> {
            let string = self.read::<16>(address)?;
            let len = u16::from_le_bytes([string[0], string[1]]) as usize;
            let buffer = usize::from_le_bytes(string[8..16].try_into().ok()?);
            if len == 0 || len % 2 != 0 {
                return None;
            }
            let mut bytes = vec![0u8; len];
            let mut read = 0;
            // SAFETY: `bytes` holds `len` bytes.
            let ok = unsafe { ReadProcessMemory(self.0, buffer as *const c_void, bytes.as_mut_ptr().cast(), len, &mut read) };
            if ok == 0 || read != len {
                return None;
            }
            let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16(&wide).ok()
        }
    }

    /// The process's current directory, read from its PEB, where Windows keeps it.
    pub fn process_cwd(pid: u32) -> Option<PathBuf> {
        let process = Process::open(pid)?;
        let mut path = process.unicode_string(process.parameters()? + CURRENT_DIRECTORY)?;
        // Kept as "C:\dir\"; but "C:\" is a root.
        if path.len() > 3 && path.ends_with('\\') {
            path.pop();
        }
        Some(PathBuf::from(path))
    }

    /// The command line the process was started with, from its PEB.
    pub(crate) fn process_commandline(pid: u32) -> Option<String> {
        let process = Process::open(pid)?;
        process.unicode_string(process.parameters()? + COMMAND_LINE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Option<String> {
        let mut scanner = CwdScanner::default();
        let mut last = None;
        for chunk in chunks {
            last = scanner.feed(chunk).pop().map(|(_, p)| p).or(last);
        }
        last
    }

    #[test]
    fn osc_9_9() {
        assert_eq!(parse_cwd_osc(br"9;9;C:\Users\me"), Some(r"C:\Users\me".into()));
        assert_eq!(parse_cwd_osc(br#"9;9;"C:\Program Files""#), Some(r"C:\Program Files".into()));
        assert_eq!(parse_cwd_osc(b"9;9;/home/me/my dir"), Some("/home/me/my dir".into()));
        assert_eq!(parse_cwd_osc(br"9;9;\\wsl$\Debian\home"), Some(r"\\wsl$\Debian\home".into()));
        assert_eq!(parse_cwd_osc(b"9;9;"), None);
        // Other OSC 9s (ConEmu's notifications, progress) aren't directories.
        assert_eq!(parse_cwd_osc(b"9;4;1;50"), None);
        assert_eq!(parse_cwd_osc(b"9;Build done"), None);
    }

    #[test]
    fn osc_7() {
        assert_eq!(parse_cwd_osc(b"7;file:///home/me"), Some("/home/me".into()));
        assert_eq!(parse_cwd_osc(b"7;file://localhost/home/my%20dir"), Some("/home/my dir".into()));
        assert_eq!(parse_cwd_osc(b"7;file:///home/%C3%BCber/100%"), Some("/home/über/100%".into()));
        assert_eq!(parse_cwd_osc(b"7;file:///C:/Users/me"), Some(r"C:\Users\me".into()));
        assert_eq!(parse_cwd_osc(b"7;file:///c:/"), Some(r"c:\".into()));
        // Another machine's directory, as over ssh.
        assert_eq!(parse_cwd_osc(b"7;file://far-away.example/home/me"), None);
        if let Some(me) = local_hostname() {
            assert_eq!(parse_cwd_osc(format!("7;file://{me}/tmp").as_bytes()), Some("/tmp".into()));
            assert_eq!(parse_cwd_osc(format!("7;file://{}/tmp", me.to_uppercase()).as_bytes()), Some("/tmp".into()));
        }
        assert_eq!(parse_cwd_osc(b"7;http://x/y"), None);
        assert_eq!(parse_cwd_osc(b"7;file://"), None);
        assert_eq!(parse_cwd_osc(b"7;file:///home/%FF"), None);
    }

    #[test]
    fn found_in_the_stream() {
        assert_eq!(scan(&[b"text\x1b]9;9;C:\\x\x1b\\more"]), Some(r"C:\x".into()));
        assert_eq!(scan(&[b"\x1b]7;file:///tmp\x07"]), Some("/tmp".into()));
        // The last one wins, also across reads.
        assert_eq!(scan(&[b"\x1b]9;9;C:\\a\x07\x1b]9;9;C:\\b\x07", b"\x1b]9;9;C:\\c\x07"]), Some(r"C:\c".into()));
        // Others are ignored: titles, hyperlinks, colors.
        assert_eq!(scan(&[b"\x1b]0;title\x07\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\"]), None);
        assert_eq!(scan(&[b"\x1b[31m\x1b]9;9;C:\\x\x07\x1b[0m"]), Some(r"C:\x".into()));
    }

    /// The bash hook's pair: without symbolic links (9;9), then as shown (7).
    #[test]
    fn reported_pairs() {
        let mut scanner = CwdScanner::default();
        let mut reported = Reported::default();
        let mut feed = |bytes: &[u8], reported: &mut Reported| {
            for (kind, path) in scanner.feed(bytes) {
                reported.record(kind, path);
            }
        };
        feed(b"\x1b]9;9;/usr/bin\x1b\\\x1b]7;file:///bin\x1b\\", &mut reported);
        assert_eq!(reported, Reported { last: Some("/bin".into()), osc99: Some("/usr/bin".into()), logical: Some("/bin".into()) });
        // A 9;9 alone (an older hook, PowerShell's): no logical name with it.
        feed(b"\x1b]9;9;/tmp\x1b\\", &mut reported);
        assert_eq!(reported, Reported { last: Some("/tmp".into()), osc99: Some("/tmp".into()), logical: None });
        // `%` in a name comes encoded.
        feed(b"\x1b]9;9;/srv/a%b c\x1b\\\x1b]7;file:///l/a%25b c\x1b\\", &mut reported);
        assert_eq!(reported.logical.as_deref(), Some("/l/a%b c"));
        assert_eq!(reported.osc99.as_deref(), Some("/srv/a%b c"));
    }

    #[test]
    fn split_across_reads() {
        let bytes = b"ab\x1b]7;file:///home/my%20dir\x1b\\cd";
        for at in 0..=bytes.len() {
            assert_eq!(scan(&[&bytes[..at], &bytes[at..]]), Some("/home/my dir".into()), "split at {at}");
        }
        let one_by_one: Vec<&[u8]> = bytes.chunks(1).collect();
        assert_eq!(scan(&one_by_one), Some("/home/my dir".into()));
    }

    #[test]
    fn broken_and_huge_sequences() {
        // Cancelled, or interrupted by another sequence.
        assert_eq!(scan(&[b"\x1b]9;9;C:\\x\x18\x07"]), None);
        assert_eq!(scan(&[b"\x1b]9;9;C:\\x\x1b[31m\x07"]), None);
        assert_eq!(scan(&[b"\x1b]9;9;C:\\x\x1b]9;9;C:\\y\x07"]), Some(r"C:\y".into()));
        // Too long to be one: skipped whole, and what follows still counts.
        let mut huge = b"\x1b]9;9;".to_vec();
        huge.extend(std::iter::repeat_n(b'a', MAX_OSC * 2));
        huge.extend(b"\x07\x1b]9;9;C:\\after\x07");
        assert_eq!(scan(&[&huge]), Some(r"C:\after".into()));
        assert_eq!(scan(&[b"\x1b]9;9;C:\\unterminated"]), None);
    }

    #[cfg(unix)]
    #[test]
    fn names_of_the_same_directory() {
        let base = std::env::temp_dir().join(format!("wtpp-name-{}", std::process::id()));
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
        let link = base.join("link").to_string_lossy().into_owned();
        let physical = real.canonicalize().unwrap();
        assert_eq!(same_directory_name(&physical, Some(&link)), Some(link.clone()));
        assert_eq!(same_directory_name(&real, Some(&link)), Some(link.clone()));
        // Another directory, a relative or missing one, or none at all.
        assert_eq!(same_directory_name(&physical, Some(&base.to_string_lossy())), None);
        assert_eq!(same_directory_name(&physical, Some("link")), None);
        assert_eq!(same_directory_name(&physical, Some("/wtpp-no-such-dir")), None);
        assert_eq!(same_directory_name(&physical, None), None);
        std::fs::remove_dir_all(&base).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn own_process_cwd() {
        assert_eq!(process_cwd(std::process::id()), std::env::current_dir().ok());
    }

    /// A shell whose directory was removed (/proc says "<dir> (deleted)") has none.
    #[cfg(unix)]
    #[test]
    fn removed_directory_is_no_directory() {
        let dir = std::env::temp_dir().join(format!("wtpp-gone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut child = std::process::Command::new("sleep").arg("30").current_dir(&dir).spawn().unwrap();
        let before = process_cwd(child.id());
        std::fs::remove_dir(&dir).unwrap();
        let after = process_cwd(child.id());
        child.kill().ok();
        child.wait().ok();
        if cfg!(target_os = "linux") {
            assert_eq!(before, Some(dir.clone()));
        }
        assert_eq!(after, None);
    }

    /// cmd's `cd` is the process's; PowerShell's `Set-Location` isn't.
    #[cfg(windows)]
    #[test]
    fn shells_process_cwd() {
        use std::io::Write;
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let wait_for = |pid: u32, want: &str| {
            let start = Instant::now();
            loop {
                let got = process_cwd(pid);
                if got.as_deref().is_some_and(|p| p.to_string_lossy().eq_ignore_ascii_case(want)) || start.elapsed() > Duration::from_secs(10) {
                    return got;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let windir = std::env::var("SystemRoot").unwrap();
        let system32 = format!(r"{windir}\System32");

        let mut cmd = Command::new("cmd.exe")
            .args(["/q", "/k"])
            .current_dir(&windir)
            .creation_flags(0x0800_0000)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = cmd.id();
        assert_eq!(wait_for(pid, &windir).map(|p| p.to_string_lossy().to_lowercase()), Some(windir.to_lowercase()));
        writeln!(cmd.stdin.as_mut().unwrap(), "cd /d {system32}").unwrap();
        assert_eq!(wait_for(pid, &system32).map(|p| p.to_string_lossy().to_lowercase()), Some(system32.to_lowercase()));
        writeln!(cmd.stdin.as_mut().unwrap(), r"cd /d C:\").unwrap();
        assert_eq!(wait_for(pid, r"C:\").map(|p| p.to_string_lossy().to_lowercase()), Some(r"c:\".into()));
        cmd.kill().ok();

        let mut ps = Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-Command", "-"])
            .current_dir(&windir)
            .creation_flags(0x0800_0000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        writeln!(ps.stdin.as_mut().unwrap(), "Set-Location {system32}; 'moved'").unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(&mut std::io::BufReader::new(ps.stdout.as_mut().unwrap()), &mut line).unwrap();
        assert_eq!(line.trim(), "moved");
        assert_eq!(process_cwd(ps.id()).map(|p| p.to_string_lossy().to_lowercase()), Some(windir.to_lowercase()));
        ps.kill().ok();

        assert_eq!(process_cwd(u32::MAX - 1), None);
    }
}
