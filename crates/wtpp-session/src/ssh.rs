//! Which ssh a pane's shell is running in the foreground, for shells that
//! don't report it (see `wtpp_config::ssh`): the newest ssh client among the
//! shell's descendants, and its arguments. Not for WSL, whose processes
//! Windows doesn't see.

use std::path::PathBuf;

/// An ssh client process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Its program, as a path where the system says.
    pub program: PathBuf,
    /// Its arguments, without the program.
    pub args: Vec<String>,
}

/// A process, as far as choosing one goes.
#[derive(Clone, Debug)]
struct Proc {
    pid: u32,
    ppid: u32,
    /// Process group (Unix).
    pgrp: Option<u32>,
    /// Its program's file name, lowercase.
    name: String,
    /// When it started, in the system's own units.
    start: u64,
}

fn is_ssh(name: &str) -> bool {
    name == "ssh" || name == "ssh.exe"
}

/// The ssh among `shell`'s descendants that's in the terminal's foreground
/// process group (`foreground`, where known), not started by another ssh
/// (a `ProxyJump`'s), the newest if there are several.
fn pick(shell: u32, foreground: Option<u32>, procs: &[Proc]) -> Option<u32> {
    let mut descendants: Vec<&Proc> = Vec::new();
    let mut parents = vec![procs.iter().find(|p| p.pid == shell)?];
    while let Some(parent) = parents.pop() {
        // A parent id can be a reused one (Windows): only children that
        // started after the parent are its.
        for child in procs.iter().filter(|p| p.ppid == parent.pid && p.pid != parent.pid && p.start >= parent.start) {
            if descendants.len() > 4096 {
                return None;
            }
            if !descendants.iter().any(|d| d.pid == child.pid) {
                descendants.push(child);
                parents.push(child);
            }
        }
    }
    let ssh_parent = |p: &Proc| descendants.iter().any(|d| d.pid == p.ppid && is_ssh(&d.name));
    descendants
        .iter()
        .filter(|p| is_ssh(&p.name) && !ssh_parent(p))
        .filter(|p| foreground.is_none() || p.pgrp == foreground)
        .max_by_key(|p| p.start)
        .map(|p| p.pid)
}

/// The ssh `shell` runs in the foreground, if any.
pub fn foreground_ssh(shell: u32) -> Option<Found> {
    #[cfg(target_os = "linux")]
    {
        linux::foreground_ssh(shell)
    }
    #[cfg(target_os = "macos")]
    {
        macos::foreground_ssh(shell)
    }
    #[cfg(all(windows, target_pointer_width = "64"))]
    {
        windows::foreground_ssh(shell)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", all(windows, target_pointer_width = "64"))))]
    {
        let _ = shell;
        None
    }
}

/// A Windows command line as the C runtime splits it into arguments
/// (CommandLineToArgvW's rules, for the arguments after the program).
pub fn split_windows_commandline(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    // The program: up to the first space, or in quotes, without escapes.
    if chars.first() == Some(&'"') {
        i = 1;
        while i < chars.len() && chars[i] != '"' {
            i += 1;
        }
        i += 1;
    } else {
        while i < chars.len() && !matches!(chars[i], ' ' | '\t') {
            i += 1;
        }
    }
    let mut args = Vec::new();
    loop {
        while i < chars.len() && matches!(chars[i], ' ' | '\t') {
            i += 1;
        }
        if i >= chars.len() {
            return args;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while i < chars.len() && (quoted || !matches!(chars[i], ' ' | '\t')) {
            match chars[i] {
                '\\' => {
                    let start = i;
                    while i < chars.len() && chars[i] == '\\' {
                        i += 1;
                    }
                    let n = i - start;
                    if chars.get(i) == Some(&'"') {
                        arg.extend(std::iter::repeat_n('\\', n / 2));
                        if n % 2 == 1 {
                            arg.push('"');
                            i += 1;
                        }
                    } else {
                        arg.extend(std::iter::repeat_n('\\', n));
                    }
                }
                '"' => {
                    // `""` inside quotes is a literal quote.
                    if quoted && chars.get(i + 1) == Some(&'"') {
                        arg.push('"');
                        i += 2;
                    } else {
                        quoted = !quoted;
                        i += 1;
                    }
                }
                c => {
                    arg.push(c);
                    i += 1;
                }
            }
        }
        args.push(arg);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{Found, Proc, pick};

    /// From `/proc/<pid>/stat`: the process, and its terminal's foreground
    /// process group.
    fn stat(pid: u32) -> Option<(Proc, Option<u32>)> {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The name is in parentheses and may have any of them itself.
        let (open, close) = (text.find('(')?, text.rfind(')')?);
        let name = text.get(open + 1..close)?.to_ascii_lowercase();
        let fields: Vec<&str> = text[close + 1..].split_whitespace().collect();
        let field = |n: usize| fields.get(n - 3).and_then(|f| f.parse::<i64>().ok());
        let tpgid = field(8).filter(|&t| t > 0).map(|t| t as u32);
        let proc = Proc { pid, ppid: field(4)? as u32, pgrp: field(5).map(|p| p as u32), name, start: field(22)? as u64 };
        Some((proc, tpgid))
    }

    pub fn foreground_ssh(shell: u32) -> Option<Found> {
        let (_, foreground) = stat(shell)?;
        let procs: Vec<Proc> = std::fs::read_dir("/proc")
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .filter_map(|pid| stat(pid).map(|(p, _)| p))
            .collect();
        let pid = pick(shell, Some(foreground?), &procs)?;
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let mut argv = cmdline.split(|&b| b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect::<Vec<_>>();
        if argv.last().is_some_and(String::is_empty) {
            argv.pop();
        }
        let program = std::fs::read_link(format!("/proc/{pid}/exe")).ok().unwrap_or_else(|| argv.first().cloned().unwrap_or_default().into());
        (!argv.is_empty()).then(|| Found { program, args: argv[1..].to_vec() })
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{Found, Proc, pick};
    use std::ffi::c_void;

    const PROC_PIDTBSDINFO: i32 = 3;
    /// `struct proc_bsdinfo`'s size, and its fields' offsets.
    const BSDINFO_SIZE: usize = 136;
    const PPID: usize = 16;
    const COMM: usize = 48;
    const PGID: usize = 100;
    const TPGID: usize = 112;
    const START_SEC: usize = 120;
    const START_USEC: usize = 128;
    const CTL_KERN: i32 = 1;
    const KERN_ARGMAX: i32 = 8;
    const KERN_PROCARGS2: i32 = 49;

    unsafe extern "C" {
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, size: i32) -> i32;
        fn proc_listchildpids(ppid: i32, buffer: *mut c_void, size: i32) -> i32;
        fn sysctl(name: *mut i32, namelen: u32, old: *mut c_void, oldlen: *mut usize, new: *mut c_void, newlen: usize) -> i32;
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_ne_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_ne_bytes(b[at..at + 8].try_into().unwrap())
    }

    /// The process, and its terminal's foreground process group.
    fn info(pid: u32) -> Option<(Proc, u32)> {
        let mut b = [0u8; BSDINFO_SIZE];
        // SAFETY: `b` is the struct's size, which is what's passed.
        let got = unsafe { proc_pidinfo(pid as i32, PROC_PIDTBSDINFO, 0, b.as_mut_ptr().cast(), b.len() as i32) };
        if got as usize != b.len() {
            return None;
        }
        let comm = &b[COMM..COMM + 16];
        let name = String::from_utf8_lossy(&comm[..comm.iter().position(|&c| c == 0).unwrap_or(16)]).to_ascii_lowercase();
        let start = u64_at(&b, START_SEC) * 1_000_000 + u64_at(&b, START_USEC);
        Some((Proc { pid, ppid: u32_at(&b, PPID), pgrp: Some(u32_at(&b, PGID)), name, start }, u32_at(&b, TPGID)))
    }

    fn children(pid: u32) -> Vec<u32> {
        let mut pids = vec![0i32; 1024];
        // SAFETY: the buffer holds `size` bytes.
        let n = unsafe { proc_listchildpids(pid as i32, pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
        pids.truncate(n.clamp(0, 1024) as usize);
        pids.into_iter().filter(|&p| p > 0).map(|p| p as u32).collect()
    }

    /// The process's arguments (KERN_PROCARGS2: argc, the executable's path,
    /// padding, then the arguments).
    fn argv(pid: u32) -> Option<(String, Vec<String>)> {
        let mut argmax: i32 = 0;
        let mut size = size_of::<i32>();
        let mut mib = [CTL_KERN, KERN_ARGMAX];
        // SAFETY: `argmax` is an int, as `size` says.
        if unsafe { sysctl(mib.as_mut_ptr(), 2, (&mut argmax as *mut i32).cast(), &mut size, std::ptr::null_mut(), 0) } != 0 {
            return None;
        }
        let mut buf = vec![0u8; argmax.max(4096) as usize];
        let mut size = buf.len();
        let mut mib = [CTL_KERN, KERN_PROCARGS2, pid as i32];
        // SAFETY: `buf` holds `size` bytes.
        if unsafe { sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) } != 0 || size < 4 {
            return None;
        }
        let buf = &buf[..size];
        let argc = u32_at(buf, 0) as usize;
        let mut rest = buf[4..].split(|&b| b == 0);
        let exe = String::from_utf8_lossy(rest.next()?).into_owned();
        let args: Vec<String> = rest.filter(|a| !a.is_empty()).take(argc).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        (args.len() == argc && argc > 0).then_some((exe, args))
    }

    pub fn foreground_ssh(shell: u32) -> Option<Found> {
        let (me, foreground) = info(shell)?;
        let mut procs = vec![me];
        let mut next = vec![shell];
        while let Some(pid) = next.pop() {
            for child in children(pid) {
                if procs.len() > 4096 || procs.iter().any(|p| p.pid == child) {
                    continue;
                }
                if let Some((p, _)) = info(child) {
                    procs.push(p);
                    next.push(child);
                }
            }
        }
        let pid = pick(shell, Some(foreground).filter(|&f| f > 0), &procs)?;
        let (exe, argv) = argv(pid)?;
        Some(Found { program: exe.into(), args: argv[1..].to_vec() })
    }
}

#[cfg(all(windows, target_pointer_width = "64"))]
mod windows {
    use super::{Found, Proc, pick, split_windows_commandline};
    use crate::cwd::windows::{Process, process_commandline};
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, PROCESS_NAME_WIN32, QueryFullProcessImageNameW};

    fn started(pid: u32) -> Option<u64> {
        let process = Process::open(pid)?;
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: four FILETIMEs to fill.
        let ok = unsafe { GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user) };
        (ok != 0).then_some((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64)
    }

    fn image(pid: u32) -> Option<String> {
        let process = Process::open(pid)?;
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        // SAFETY: `buf` holds `len` characters.
        let ok = unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) };
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }

    pub fn foreground_ssh(shell: u32) -> Option<Found> {
        // SAFETY: plain call; closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut all = Vec::new();
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        // SAFETY: `entry` has its size set, as the calls require.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase();
            all.push((entry.th32ProcessID, entry.th32ParentProcessID, name));
            // SAFETY: as above.
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        // SAFETY: opened above.
        unsafe { CloseHandle(snapshot) };
        // Only the shell's descendants need a start time: walk down from it.
        let mut procs: Vec<Proc> = Vec::new();
        let mut next = vec![shell];
        while let Some(pid) = next.pop() {
            if procs.len() > 4096 || procs.iter().any(|p| p.pid == pid) {
                continue;
            }
            let Some((_, ppid, name)) = all.iter().find(|(p, ..)| *p == pid) else { continue };
            let Some(start) = started(pid) else { continue };
            procs.push(Proc { pid, ppid: *ppid, pgrp: None, name: name.clone(), start });
            next.extend(all.iter().filter(|(p, parent, _)| *parent == pid && *p != pid).map(|(p, ..)| *p));
        }
        let pid = pick(shell, None, &procs)?;
        let line = process_commandline(pid)?;
        Some(Found { program: image(pid)?.into(), args: split_windows_commandline(&line) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, ppid: u32, pgrp: u32, name: &str, start: u64) -> Proc {
        Proc { pid, ppid, pgrp: Some(pgrp), name: name.into(), start }
    }

    #[test]
    fn picks_the_foreground_ssh() {
        let procs = vec![
            p(10, 1, 10, "bash", 1),
            p(11, 10, 11, "ssh", 2),   // in the background
            p(12, 10, 12, "ssh", 3),   // the foreground one
            p(13, 12, 12, "ssh", 4),   // its ProxyJump
            p(20, 1, 20, "bash", 1),   // another pane's
            p(21, 20, 12, "ssh", 9),
            p(14, 10, 14, "vim", 5),
        ];
        assert_eq!(pick(10, Some(12), &procs), Some(12));
        assert_eq!(pick(10, Some(14), &procs), None);
        assert_eq!(pick(10, Some(10), &procs), None);
        // No process groups (Windows): the newest.
        assert_eq!(pick(10, None, &procs), Some(12));
        assert_eq!(pick(99, None, &procs), None);
        // Deeper down (a script running ssh), and named ssh.exe.
        let procs = vec![p(1, 0, 1, "pwsh.exe", 5), p(2, 1, 2, "cmd.exe", 6), p(3, 2, 2, "ssh.exe", 7)];
        assert_eq!(pick(1, None, &procs), Some(3));
        // A reused parent id: started before its "parent", not its child.
        let procs = vec![p(1, 0, 1, "cmd.exe", 5), p(3, 1, 3, "ssh.exe", 2)];
        assert_eq!(pick(1, None, &procs), None);
    }

    #[test]
    fn windows_command_lines() {
        let s = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(split_windows_commandline(r"ssh.exe -p 22 me@host"), s(&["-p", "22", "me@host"]));
        assert_eq!(split_windows_commandline(r#""C:\Program Files\Git\usr\bin\ssh.exe" -i "C:\my keys\k" h"#), s(&["-i", r"C:\my keys\k", "h"]));
        assert_eq!(split_windows_commandline(r#"ssh a\\\"b "c d\\" e\f"#), s(&[r#"a\"b"#, r"c d\", r"e\f"]));
        assert_eq!(split_windows_commandline(r#"ssh "a""b" """#), s(&[r#"a"b"#, ""]));
        assert_eq!(split_windows_commandline("ssh"), Vec::<String>::new());
        assert_eq!(split_windows_commandline("ssh   "), Vec::<String>::new());
    }
}
