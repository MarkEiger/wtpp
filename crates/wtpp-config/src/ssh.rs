//! A split or duplicated tab of a pane that's in ssh opens an ssh to the same
//! place, over the same connection: what the shells are given for that, the
//! report they send, and which of the ssh's arguments the new pane gets.
//!
//! bash and zsh get an `ssh` function (unless the user has an `ssh` alias or
//! function of their own) that adds OpenSSH connection sharing where the
//! user's ssh configuration has none (`ControlMaster auto`, a socket in a
//! private directory, `ControlPersist`), and tells wt++ the arguments when
//! ssh starts and when it ends, by a private OSC that carries the pane's
//! token. The new pane's shell runs `ssh` with the arguments it's given in
//! `WTPP_SSH`, once, at its first prompt: through the same function, so it
//! uses the same socket and needs no new login.
//!
//! The report comes through the terminal, like everything a remote host
//! prints, so only one with the pane's token counts. The token is in the
//! shell's environment as `WTPP_TOKEN` when it starts; the shell moves it to
//! an unexported variable at its first prompt, so no program it starts (ssh,
//! or anything on the other side) has it. And what a report can make wt++ run
//! is only ever `ssh`, with the arguments as separate words (no shell reads
//! them as a command line), and only those [`session_args`] lets through.

use std::hash::{BuildHasher, Hasher};

/// The private OSC's number: `ESC ] 6973 ; ssh ; <token> ; <arg> ; … ST` when
/// ssh starts (each argument with `%` and `;` percent-encoded), and
/// `ESC ] 6973 ; ssh-end ; <token> ST` when it's done.
pub const OSC: &str = "6973";

/// How long a shared connection stays after its last session ends: closing
/// the pane that opened it (or typing `exit` there) leaves the others
/// connected, and an ssh to the same place within that time needs no login,
/// but nothing stays connected for good.
pub const CONTROL_PERSIST: &str = "10m";

/// A new random token for a pane, in hex: 128 bits from the keys the
/// standard library seeds from the system's random source.
pub fn new_token() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let half = |salt: u8| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u8(salt);
        h.write_u64(n);
        h.write_u128(nanos);
        h.write_u32(std::process::id());
        h.finish()
    };
    format!("{:016x}{:016x}", half(1), half(2))
}

/// What a shell reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    /// ssh started with these arguments (the ones the user gave).
    Started(Vec<String>),
    Ended,
}

/// A report in an OSC's text (between `ESC ]` and its terminator), if it's
/// one with `token`.
pub fn parse_report(osc: &[u8], token: &str) -> Option<Report> {
    let text = std::str::from_utf8(osc).ok()?;
    let mut parts = text.strip_prefix(OSC)?.strip_prefix(';')?.split(';');
    let kind = parts.next()?;
    let given = parts.next()?;
    if token.is_empty() || !same_token(given, token) {
        return None;
    }
    match kind {
        "ssh-end" => parts.next().is_none().then_some(Report::Ended),
        "ssh" => {
            let args = parts.map(percent_decode).collect::<Option<Vec<_>>>()?;
            (!args.is_empty()).then_some(Report::Started(args))
        }
        _ => None,
    }
}

/// Compares in the same time whatever the difference.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// ssh's options that take a value (OpenSSH 10's getopt string).
const WITH_VALUE: &str = "BbcDEeFIiJLlmOoPpQRSWw";
/// Options that carry over as they are.
const KEPT_FLAGS: &str = "46AaCgKkqTtvXxYy";
const KEPT_WITH_VALUE: &str = "BbceFiJlmPpS";
/// Options for what this ssh does besides the session: forwards, going to
/// the background, no shell; and `-I`/`-E`, a library to load and a log to
/// write, which a shared connection doesn't need. Dropped.
const DROPPED_FLAGS: &str = "fMNn";
const DROPPED_WITH_VALUE: &str = "DLRwIE";
// Anything else (-G, -V, -Q, -O, -s, -W, unknown ones) isn't a session to
// open again: no duplicate.

/// `-o` options that carry over: where to connect and how, nothing that
/// runs a local command or loads code.
const KEPT_OPTIONS: &[&str] = &[
    "addkeystoagent", "addressfamily", "batchmode", "bindaddress", "bindinterface", "canonicaldomains",
    "canonicalizefallbacklocal", "canonicalizehostname", "canonicalizemaxdots", "canonicalizepermittedcnames",
    "casignaturealgorithms", "certificatefile", "checkhostip", "ciphers", "compression", "connectionattempts",
    "connecttimeout", "controlpath", "enableescapecommandline", "escapechar", "fingerprinthash", "forwardagent",
    "forwardx11", "forwardx11timeout", "forwardx11trusted", "globalknownhostsfile", "gssapiauthentication",
    "gssapidelegatecredentials", "hashknownhosts", "hostbasedacceptedalgorithms", "hostbasedauthentication",
    "hostkeyalgorithms", "hostkeyalias", "hostname", "identitiesonly", "identityagent", "identityfile", "ipqos",
    "kbdinteractiveauthentication", "kbdinteractivedevices", "kexalgorithms", "loglevel", "macs",
    "nohostauthenticationforlocalhost", "numberofpasswordprompts", "obscurekeystroketiming", "passwordauthentication",
    "port", "preferredauthentications", "proxyjump", "pubkeyacceptedalgorithms", "pubkeyacceptedkeytypes",
    "pubkeyauthentication", "rekeylimit", "requesttty", "requiredrsasize", "revokedhostkeys", "sendenv",
    "serveralivecountmax", "serveraliveinterval", "setenv", "stricthostkeychecking", "tcpkeepalive", "updatehostkeys",
    "user", "userknownhostsfile", "verifyhostkeydns", "visualhostkey", "warnweakcrypto",
];
/// `-o` options for the rest of what this ssh does (see [`DROPPED_FLAGS`]),
/// or that the new pane's own `ssh` sets. Dropped.
const DROPPED_OPTIONS: &[&str] = &[
    "clearallforwardings", "controlmaster", "controlpersist", "dynamicforward", "exitonforwardfailure",
    "forkafterauthentication", "gatewayports", "localforward", "permitremoteopen", "pkcs11provider", "remotecommand",
    "remoteforward", "securitykeyprovider", "sessiontype", "stdinnull", "streamlocalbindmask", "streamlocalbindunlink",
    "tunnel", "tunneldevice",
];

/// The arguments for a new ssh to where `args` (ssh's, without the program)
/// goes, with the same connection options: no forwards, no `-f`/`-N`, no
/// remote command. An error says why there's none (not a session, or an
/// option that isn't carried over, such as `ProxyCommand`).
pub fn session_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut destination: Option<String> = None;
    let mut i = 0;
    // After `--`, no more options, also after the destination.
    let mut terminated = false;
    while i < args.len() {
        let arg = &args[i];
        i += 1;
        if arg.is_empty() || arg.chars().any(char::is_control) {
            return Err("an argument that's empty or has control characters".into());
        }
        if terminated || !arg.starts_with('-') || arg == "-" {
            if destination.is_some() {
                // The remote command: dropped, and with it the rest.
                break;
            }
            if arg.starts_with('-') {
                return Err(format!("destination {arg:?}"));
            }
            destination = Some(arg.clone());
            continue;
        }
        if arg == "--" {
            terminated = true;
            continue;
        }
        for (at, flag) in arg[1..].char_indices() {
            if !WITH_VALUE.contains(flag) {
                if KEPT_FLAGS.contains(flag) {
                    out.push(format!("-{flag}"));
                } else if !DROPPED_FLAGS.contains(flag) {
                    return Err(format!("option -{flag}"));
                }
                continue;
            }
            let rest = &arg[1 + at + flag.len_utf8()..];
            let value = if !rest.is_empty() {
                rest.to_string()
            } else {
                let value = args.get(i).ok_or_else(|| format!("-{flag} without a value"))?;
                i += 1;
                value.clone()
            };
            if value.chars().any(char::is_control) {
                return Err(format!("-{flag} with control characters"));
            }
            if flag == 'o' {
                let name = value.split(['=', ' ', '\t']).next().unwrap_or("").to_ascii_lowercase();
                if KEPT_OPTIONS.contains(&name.as_str()) {
                    out.extend(["-o".to_string(), value]);
                } else if !DROPPED_OPTIONS.contains(&name.as_str()) {
                    return Err(format!("option {name:?}"));
                }
            } else if KEPT_WITH_VALUE.contains(flag) {
                if flag == 'J' && value.starts_with('-') {
                    return Err(format!("jump host {value:?}"));
                }
                out.extend([format!("-{flag}"), value]);
            } else if !DROPPED_WITH_VALUE.contains(flag) {
                return Err(format!("option -{flag}"));
            }
            break;
        }
    }
    let destination = destination.ok_or("no destination")?;
    out.push(destination);
    Ok(out)
}

/// What a new pane is given to run ssh: the arguments, one per line, for
/// `WTPP_SSH` (none has a control character, see [`session_args`]).
pub fn env_value(args: &[String]) -> String {
    args.join("\n")
}

/// How the new pane's shell runs the ssh it's given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    /// Through `ssh` as the shell knows it: the function, which shares the
    /// connection (the arguments are the ones the function reported).
    Function,
    /// As `command ssh`, past aliases and functions: the arguments were found
    /// on the running ssh, so whatever an alias or function of the user's
    /// added is in them already, and must not be added twice.
    Command,
}

/// Environment variables for the shell of a pane: its token, and with `ssh`
/// the arguments for the ssh it runs at its first prompt, and how.
pub fn shell_env(token: &str, ssh: Option<(&[String], Run)>) -> Vec<(String, String)> {
    let mut env = vec![("WTPP_TOKEN".to_string(), token.to_string())];
    if let Some((args, run)) = ssh {
        env.push(("WTPP_SSH".into(), env_value(args)));
        if run == Run::Command {
            env.push(("WTPP_SSH_DIRECT".into(), "1".into()));
        }
    }
    env
}

/// bash: at the first prompt, the token goes into an unexported variable and
/// `ssh` becomes a function (unless it's an alias or function already). The
/// function asks ssh for the destination's configuration (`ssh -G`, which
/// doesn't connect): not a session (`-V`, `-Q`, a usage error), it runs ssh
/// as it is; one with no ControlMaster or ControlPath configured or given
/// (`-M`, `-S`, `-o Control…`, which the first one would lose to), it
/// shares the connection. It reports the arguments unless one has a
/// control character, writing to the terminal (not to where ssh's output
/// goes). Works in bash 3.2 (macOS's) too. `function ssh` rather than
/// `ssh()`, which an alias of the same name would expand in. Every prompt
/// then says ssh isn't running: Ctrl+C at ssh's password prompt ends the
/// function too, before it can.
pub const BASH_SSH_INIT: &str = concat!(
    r#"[ -n "${WTPP_TOKEN-}" ] && { __wtpp_token=$WTPP_TOKEN; unset WTPP_TOKEN; { alias ssh || declare -F ssh; } >/dev/null 2>&1 || "#,
    r#"function ssh { local c t=${__wtpp_token-} a p= s d=${XDG_RUNTIME_DIR:-$HOME/.cache}/wtpp-ssh; local -a o; "#,
    r#"c=$(command ssh -G "$@" 2>/dev/null); "#,
    r#"case $c in *$'\n'controlmaster\ *) ;; *) command ssh "$@"; return;; esac; "#,
    r#"case $c in *$'\n'controlmaster\ false$'\n'*) case $c in *$'\n'controlpath\ *) ;; "#,
    r#"*) case $* in *[Cc][Oo][Nn][Tt][Rr][Oo][Ll][MmPp][AaEe]*) ;; "#,
    r#"*) [ ${#d} -le 45 ] && mkdir -p -m 700 "$d" 2>/dev/null && o=(-o ControlMaster=auto -o "ControlPath=$d/%C" -o ControlPersist=10m);; esac;; esac;; esac; "#,
    r#"[ -n "$t" ] && for a; do case $a in *[[:cntrl:]]*) t=; break;; esac; a=${a//\%/%25}; p="$p;${a//;/%3B}"; done; "#,
    r#"[ -n "$t" ] && { printf '\033]6973;ssh;%s%s\033\\' "$t" "$p" >/dev/tty; } 2>/dev/null; "#,
    r#"command ssh ${o[@]+"${o[@]}"} "$@"; s=$?; "#,
    r#"[ -n "$t" ] && { printf '\033]6973;ssh-end;%s\033\\' "$t" >/dev/tty; } 2>/dev/null; "#,
    r#"return $s; }; }; "#,
    // At a prompt, ssh isn't running (also if Ctrl+C ended it and the
    // function with it, before its own report).
    r#"[ -n "${__wtpp_token-}" ] && printf '\033]6973;ssh-end;%s\033\\' "$__wtpp_token""#,
);

/// bash: the ssh a new pane was given (`WTPP_SSH`), once, through `ssh` as
/// the shell knows it, or with `WTPP_SSH_DIRECT` as `command ssh` (see
/// [`Run`]), and into the history so ↑ runs it again.
pub const BASH_SSH_RUN: &str = concat!(
    r#"[ -n "${WTPP_SSH-}" ] && { IFS=$'\n' read -r -d '' -a __wtpp_a <<< "$WTPP_SSH"; __wtpp_c=${WTPP_SSH_DIRECT:+command }; "#,
    r#"unset WTPP_SSH WTPP_SSH_DIRECT; printf -v __wtpp_h ' %q' "${__wtpp_a[@]}"; history -s "${__wtpp_c}ssh$__wtpp_h"; "#,
    r#"if [ -n "$__wtpp_c" ]; then command ssh "${__wtpp_a[@]}"; else ssh "${__wtpp_a[@]}"; fi; unset __wtpp_a __wtpp_h __wtpp_c; }"#,
);

/// zsh's part of `.zshrc` (see [`crate::cwd::ZSH_FILES`]): the same as bash's.
pub const ZSH_SSH: &str = r#"if [[ -n $WTPP_TOKEN ]]; then
  builtin typeset -g __wtpp_token=$WTPP_TOKEN
  builtin unset WTPP_TOKEN
  if (( ! $+functions[ssh] && ! $+aliases[ssh] )); then
    function ssh {
      builtin emulate -L zsh
      local c t=$__wtpp_token a p= s d=${XDG_RUNTIME_DIR:-$HOME/.cache}/wtpp-ssh
      local -a o
      c=$(command ssh -G "$@" 2>/dev/null)
      [[ $c == *$'\n'controlmaster\ * ]] || { command ssh "$@"; return }
      if [[ $c == *$'\n'controlmaster\ false$'\n'* && $c != *$'\n'controlpath\ * ]] &&
        [[ "$*" != *[Cc][Oo][Nn][Tt][Rr][Oo][Ll][MmPp][AaEe]* ]] && (( ${#d} <= 45 )) &&
        command mkdir -p -m 700 -- $d 2>/dev/null; then
        o=(-o ControlMaster=auto -o ControlPath=$d/%C -o ControlPersist=10m)
      fi
      if [[ -n $t ]]; then
        for a in "$@"; do
          [[ $a == *[[:cntrl:]]* ]] && { t=; break }
          a=${a//\%/%25}
          p+=";${a//;/%3B}"
        done
      fi
      [[ -n $t ]] && { builtin print -rn -- $'\e]6973;ssh;'$t$p$'\e\\' >/dev/tty } 2>/dev/null
      {
        command ssh $o "$@"
        s=$?
      } always {
        [[ -n $t ]] && { builtin print -rn -- $'\e]6973;ssh-end;'$t$'\e\\' >/dev/tty } 2>/dev/null
      }
      return s
    }
  fi
  __wtpp_ssh_run() {
    precmd_functions=(${precmd_functions:#__wtpp_ssh_run})
    [[ -n $WTPP_SSH ]] || return 0
    local -a a
    local c=${WTPP_SSH_DIRECT:+command }
    a=("${(@f)WTPP_SSH}")
    builtin unset WTPP_SSH WTPP_SSH_DIRECT
    builtin print -rs -- "${c}ssh ${(j: :)${(@q)a}}"
    {
      if [[ -n $c ]]; then command ssh "${a[@]}"; else ssh "${a[@]}"; fi
    } always { __wtpp_ssh_idle }
  }
  # At a prompt, ssh isn't running (also if Ctrl+C ended the function).
  __wtpp_ssh_idle() {
    builtin print -rn -- $'\e]6973;ssh-end;'$__wtpp_token$'\e\\'
  }
  precmd_functions=(__wtpp_ssh_idle $precmd_functions __wtpp_ssh_run)
fi
"#;

/// PowerShell (Windows' own ssh.exe, which can't share a connection): run
/// the ssh in `WTPP_SSH_PROGRAM` with the arguments in `WTPP_SSH`, as an
/// argument list. Added to `-Command` after the prompt hook, if any. No
/// double quotes (see `cwd::POWERSHELL_PROMPT_HOOK`).
pub const POWERSHELL_SSH_RUN: &str = "if ($env:WTPP_SSH) { $a = $env:WTPP_SSH.Split([char]10); $p = $env:WTPP_SSH_PROGRAM; \
     Remove-Item Env:WTPP_SSH, Env:WTPP_SSH_PROGRAM; & $p @a }";

/// Whether `args` can go to Windows PowerShell as an argument list: its
/// quoting of native arguments loses double quotes.
pub fn fits_powershell(args: &[String]) -> bool {
    !args.iter().any(|a| a.contains('"'))
}

/// cmd: `/k <ssh> <args>`, if every word is plain enough that cmd reads it
/// back as it is (no quoting, no `%`, `&`, `^`, …), else `None`.
pub fn cmd_args(program: &str, args: &[String]) -> Option<Vec<String>> {
    let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "@._:/\\=+,~-[]".contains(c));
    let mut out = vec!["/k".to_string(), program.to_string()];
    out.extend(args.iter().cloned());
    out[1..].iter().all(|a| plain(a)).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn args(a: &[&str]) -> Result<Vec<String>, String> {
        session_args(&v(a))
    }

    #[test]
    fn tokens() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn reports() {
        let t = "0123456789abcdef0123456789abcdef";
        assert_eq!(parse_report(format!("6973;ssh;{t};-p;22;me@host").as_bytes(), t), Some(Report::Started(v(&["-p", "22", "me@host"]))));
        assert_eq!(parse_report(format!("6973;ssh-end;{t}").as_bytes(), t), Some(Report::Ended));
        // `%` and `;` come encoded.
        assert_eq!(
            parse_report(format!("6973;ssh;{t};-o;SetEnv=A=1%3Bb%25;host").as_bytes(), t),
            Some(Report::Started(v(&["-o", "SetEnv=A=1;b%", "host"])))
        );
        assert_eq!(parse_report(format!("6973;ssh;{t};h%C3%BCst").as_bytes(), t), Some(Report::Started(v(&["hüst"]))));
        // Without the token, or another one, or none: nothing.
        assert_eq!(parse_report(b"6973;ssh;;host", t), None);
        assert_eq!(parse_report(b"6973;ssh;0123456789abcdef0123456789abcdee;host", t), None);
        assert_eq!(parse_report(b"6973;ssh;0123;host", t), None);
        assert_eq!(parse_report(format!("6973;ssh;{t}").as_bytes(), ""), None);
        assert_eq!(parse_report(b"6973;ssh;;host", ""), None);
        assert_eq!(parse_report(format!("6973;ssh-end;x{t}").as_bytes(), t), None);
        // Not a report, or a broken one.
        assert_eq!(parse_report(format!("6973;ssh;{t}").as_bytes(), t), None);
        assert_eq!(parse_report(format!("6973;run;{t};x").as_bytes(), t), None);
        assert_eq!(parse_report(format!("69730;ssh;{t};x").as_bytes(), t), None);
        assert_eq!(parse_report(format!("6973;ssh;{t};%zz").as_bytes(), t), None);
        assert_eq!(parse_report(format!("6973;ssh;{t};%F").as_bytes(), t), None);
        assert_eq!(parse_report(format!("6973;ssh;{t};%FF").as_bytes(), t), None);
        assert_eq!(parse_report(format!("7;file:///{t}").as_bytes(), t), None);
    }

    #[test]
    fn carried_over() {
        assert_eq!(args(&["host"]), Ok(v(&["host"])));
        assert_eq!(args(&["-p", "2222", "-l", "me", "host"]), Ok(v(&["-p", "2222", "-l", "me", "host"])));
        // Bundled, attached values, and options after the destination.
        assert_eq!(args(&["-vAp2222", "host", "-i", "~/k", "-tt"]), Ok(v(&["-v", "-A", "-p", "2222", "-i", "~/k", "-t", "-t", "host"])));
        assert_eq!(args(&["-oUser=me", "-o", "Port 22", "h"]), Ok(v(&["-o", "User=me", "-o", "Port 22", "h"])));
        assert_eq!(args(&["-J", "jump", "-F", "/tmp/cfg", "ssh://me@h:22"]), Ok(v(&["-J", "jump", "-F", "/tmp/cfg", "ssh://me@h:22"])));
        // The connection's socket, given or added by the shell function.
        assert_eq!(
            args(&["-o", "ControlMaster=auto", "-o", "ControlPath=/run/user/1/wtpp-ssh/%C", "-o", "ControlPersist=10m", "h"]),
            Ok(v(&["-o", "ControlPath=/run/user/1/wtpp-ssh/%C", "h"]))
        );
        assert_eq!(args(&["-M", "-S", "/tmp/s", "h"]), Ok(v(&["-S", "/tmp/s", "h"])));
        assert_eq!(args(&["--", "h", "-p"]), Ok(v(&["h"])));
    }

    #[test]
    fn dropped() {
        // Forwards, background, no shell, and the remote command.
        assert_eq!(args(&["-L", "8080:localhost:80", "-R8081:x:1", "-D", "1080", "-fNn", "h"]), Ok(v(&["h"])));
        assert_eq!(args(&["-o", "LocalForward 1 x:1", "-oRemoteCommand=top", "-o", "SessionType=none", "h"]), Ok(v(&["h"])));
        assert_eq!(args(&["h", "ls", "-l", "-p", "1"]), Ok(v(&["h"])));
        assert_eq!(args(&["-t", "h", "tmux", "attach"]), Ok(v(&["-t", "h"])));
        assert_eq!(args(&["-I", "/lib/p11.so", "-E", "/tmp/log", "-w", "0:1", "h"]), Ok(v(&["h"])));
    }

    #[test]
    fn refused() {
        // Not a session.
        for a in [&["-V"][..], &["-G", "h"], &["-Q", "cipher"], &["-O", "check", "h"], &["-W", "x:22", "h"], &["-s", "h", "sftp"], &[]] {
            assert!(args(a).is_err(), "{a:?}");
        }
        // Options that run something here.
        for o in ["ProxyCommand=nc %h %p", "proxycommand nc", "LocalCommand=touch x", "PermitLocalCommand=yes", "KnownHostsCommand=x", "Include x", "Match exec x", "NoSuchOption=1"] {
            assert!(args(&["-o", o, "h"]).is_err(), "{o}");
            assert!(args(&[&format!("-o{o}"), "h"]).is_err(), "{o}");
        }
        assert!(args(&["-Ap", "1", "-oProxyCommand=x", "h"]).is_err());
        // Odd destinations and values.
        assert!(args(&["-J", "-oProxyCommand=x", "h"]).is_err());
        assert!(args(&["--", "-oProxyCommand=x"]).is_err());
        assert!(args(&["h\x1b]0;x"]).is_err());
        assert!(args(&["-l", "me\nyou", "h"]).is_err());
        assert!(args(&["", "h"]).is_err());
        assert!(args(&["-p"]).is_err());
        assert!(args(&["-Z", "h"]).is_err());
    }

    /// The function in real shells: what it adds to ssh's arguments, by a
    /// stand-in ssh that prints them (and asks the real one for `-G`, with
    /// no configuration but the one given). bash, and with WTPP_TEST_ZSH
    /// (and WTPP_TEST_ZSH_MODULES, see wtpp-session's tests) zsh.
    #[cfg(unix)]
    #[test]
    fn function_options() {
        let Some(real) = std::process::Command::new("sh").args(["-c", "command -v ssh"]).output().ok().filter(|o| o.status.success()) else { return };
        let real = String::from_utf8(real.stdout).unwrap().trim().to_string();
        let dir = std::path::PathBuf::from(format!("/tmp/wtpp-fn-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let fake = dir.join("bin/ssh");
        std::fs::write(&fake, format!("#!/bin/sh\nif [ \"$1\" = -G ]; then shift; exec {real} -F /dev/null -G \"$@\"; fi\necho \"ARGS $*\"\n")).unwrap();
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let theirs = dir.join("theirs");
        std::fs::write(&theirs, "Host *\n  ControlMaster auto\n  ControlPath /tmp/x-%C\n").unwrap();
        let short = dir.join("run").to_string_lossy().into_owned();
        let long = format!("/tmp/{}", "x".repeat(40));
        let shared = format!("ARGS -o ControlMaster=auto -o ControlPath={short}/wtpp-ssh/%C -o ControlPersist=10m");
        let mut shells: Vec<(Vec<String>, String)> = vec![(
            vec!["bash".into(), "--norc".into(), "--noprofile".into(), "-c".into()],
            "eval \"$1\"; ".to_string(),
        )];
        let zsh_rc = dir.join("wtpp-ssh.zsh");
        std::fs::write(&zsh_rc, ZSH_SSH).unwrap();
        if let Ok(zsh) = std::env::var("WTPP_TEST_ZSH") {
            let modules = std::env::var("WTPP_TEST_ZSH_MODULES").map(|m| format!("module_path=({m}); ")).unwrap_or_default();
            shells.push((vec![zsh, "-f".into(), "-c".into()], format!("{modules}source {}; ", zsh_rc.display())));
        }
        let pc = crate::cwd::bash_prompt_command(false, false, true).unwrap();
        for (shell, setup) in &shells {
            let go = |runtime: &str, line: &str| {
                let mut c = std::process::Command::new(&shell[0]);
                c.args(&shell[1..]).arg(format!("{setup}{line}"));
                // bash gets the PROMPT_COMMAND to eval as $1.
                if shell[0] == "bash" {
                    c.args(["bash", &pc]);
                }
                c.env("PATH", format!("{}:/usr/bin:/bin", dir.join("bin").display())).env("XDG_RUNTIME_DIR", runtime).env("WTPP_TOKEN", "tok");
                let out = c.output().unwrap();
                // Without the prompt's report that ssh isn't running.
                let stdout = String::from_utf8_lossy(&out.stdout).replace("\x1b]6973;ssh-end;tok\x1b\\", "");
                stdout.trim().to_string() + &String::from_utf8_lossy(&out.stderr)
            };
            assert_eq!(go(&short, "ssh -p 22 me@host ls 'a b'"), format!("{shared} -p 22 me@host ls a b"), "{shell:?}");
            assert_eq!(go(&short, "echo ${WTPP_TOKEN-unset} $__wtpp_token"), "unset tok", "{shell:?}");
            // Not a session; the user's own sharing, given or configured; a
            // socket path that would be too long.
            assert_eq!(go(&short, "ssh -V"), "ARGS -V", "{shell:?}");
            assert_eq!(go(&short, "ssh -S /tmp/s h"), "ARGS -S /tmp/s h", "{shell:?}");
            assert_eq!(go(&short, "ssh -M h"), "ARGS -M h", "{shell:?}");
            assert_eq!(go(&short, "ssh -o controlmaster=no h"), "ARGS -o controlmaster=no h", "{shell:?}");
            assert_eq!(go(&short, "ssh -oControlPersist=no h"), "ARGS -oControlPersist=no h", "{shell:?}");
            assert_eq!(go(&short, &format!("ssh -F {} h", theirs.display())), format!("ARGS -F {} h", theirs.display()), "{shell:?}");
            assert_eq!(go(&long, "ssh h"), "ARGS h", "{shell:?}");
            // Its exit status is ssh's; output redirected stays clean.
            assert_eq!(go(&short, "ssh h > /dev/null; echo $?"), "0", "{shell:?}");
        }
        assert_eq!(std::fs::metadata(dir.join("run/wtpp-ssh")).unwrap().permissions().mode() & 0o777, 0o700);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn windows_launches() {
        assert_eq!(
            cmd_args(r"C:\Windows\System32\OpenSSH\ssh.exe", &v(&["-p", "22", "me@host"])),
            Some(v(&["/k", r"C:\Windows\System32\OpenSSH\ssh.exe", "-p", "22", "me@host"]))
        );
        assert_eq!(cmd_args(r"C:\Program Files\Git\usr\bin\ssh.exe", &v(&["h"])), None);
        for bad in ["a&calc", "%PATH%", "a b", "\"x\"", "a^b", "a|b", "a>b", "(x)", "!x!"] {
            assert_eq!(cmd_args("ssh.exe", &v(&[bad])), None, "{bad}");
        }
        assert!(fits_powershell(&v(&["-o", "User=a b", "h"])));
        assert!(!fits_powershell(&v(&["-o", "User=\"a\"", "h"])));
        assert!(!POWERSHELL_SSH_RUN.contains('"'));
    }
}
