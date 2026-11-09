//! SIGINT stopt symlog voordat een rapport wordt gepubliceerd.
//!
//! Het signaal gaat door een pidfd van het kind dat deze test zelf startte.
//! Een pidfd noemt dat proces, niet een procesnummer dat hergebruikt kan worden,
//! dus geen ander proces op deze machine krijgt het signaal.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SIGINT: i32 = 2;
const SIGKILL: i32 = 9;
const SYS_PIDFD_SEND_SIGNAL: i64 = 424;
const SYS_PIDFD_OPEN: i64 = 434;

unsafe extern "C" {
    fn close(fd: i32) -> i32;
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct PidFd(i32);

impl PidFd {
    fn open(pid: i32) -> Self {
        let fd = unsafe { syscall4(SYS_PIDFD_OPEN, i64::from(pid), 0, 0, 0) };
        assert!(fd >= 0, "pidfd_open({pid}) returned {fd}");
        assert!(fd <= i64::from(i32::MAX), "pidfd {fd} does not fit");
        Self(fd as i32)
    }

    fn signal(&self, sig: i32) -> i64 {
        unsafe {
            syscall4(
                SYS_PIDFD_SEND_SIGNAL,
                i64::from(self.0),
                i64::from(sig),
                0,
                0,
            )
        }
    }
}

impl Drop for PidFd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            unsafe { close(self.0) };
        }
    }
}

/// Wacht het kind af. Leeft het nog bij het einde van de test, dan gaat SIGKILL
/// alleen naar dezelfde pidfd. Nooit naar een procesnummer.
/// `stdin` blijft open tot het kind is afgewacht, anders ziet het kind einde-van-invoer.
struct ChildGuard {
    child: Child,
    pidfd: PidFd,
    stdin: Option<std::process::ChildStdin>,
}

impl ChildGuard {
    fn spawn(mut command: Command) -> Self {
        let mut child = command.spawn().expect("spawn");
        let pid = i32::try_from(child.id()).expect("pid fits in i32");
        assert!(pid > 1, "refuse to signal pid {pid}");
        assert_ne!(pid as u32, std::process::id(), "refuse to signal this test");
        let pidfd = PidFd::open(pid);
        assert_eq!(
            parent_of(pid),
            Some(std::process::id()),
            "refuse to signal a process that is not this test's child"
        );
        let stdin = child.stdin.take();
        Self {
            child,
            pidfd,
            stdin,
        }
    }

    fn pid(&self) -> i32 {
        i32::try_from(self.child.id()).expect("pid fits in i32")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // `stdin` moet open blijven tot het kind weg is, anders leest het einde-van-invoer.
        let _held = self.stdin.as_ref();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.pidfd.signal(SIGKILL);
            let _ = self.child.wait();
        }
    }
}

fn scratch() -> Scratch {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "symlog-sigint-{}-{}-{}",
        std::process::id(),
        nanos,
        N.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).expect("create scratch dir");
    Scratch(dir)
}

fn parent_of(pid: i32) -> Option<u32> {
    let text = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("PPid:") else {
            continue;
        };
        return rest.trim().parse().ok();
    }
    None
}

fn blocked_in_read(pid: i32) -> bool {
    let Ok(text) = fs::read_to_string(format!("/proc/{pid}/syscall")) else {
        return false;
    };
    text.split_whitespace().next() == Some("0")
}

/// `syscall` met vier argumenten. Extra argumenten zijn 0 voor aanroepen die
/// er minder nemen. Alleen linux x86_64, dezelfde cfg als de test.
unsafe fn syscall4(nr: i64, a: i64, b: i64, c: i64, d: i64) -> i64 {
    let ret: i64;
    std::arch::asm!(
        "syscall",
        in("rax") nr,
        in("rdi") a,
        in("rsi") b,
        in("rdx") c,
        in("r10") d,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack)
    );
    ret
}

fn wait_until_blocked(pid: i32) -> bool {
    let start = Instant::now();
    let mut streak = 0;
    while start.elapsed() < Duration::from_secs(5) {
        if blocked_in_read(pid) {
            streak += 1;
            if streak >= 3 {
                return true;
            }
        } else {
            streak = 0;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn stop_child(guard: &mut ChildGuard) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = guard.child.try_wait().expect("try_wait") {
            return status;
        }
        if start.elapsed() > Duration::from_secs(5) {
            let _ = guard.pidfd.signal(SIGKILL);
            let status = guard.child.wait().expect("reap after SIGKILL");
            panic!("child did not exit after SIGINT: {status:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(not(coverage))]
#[test]
fn sigint_on_blocked_stdin_writes_no_report_and_exits_130() {
    let root = scratch();
    let report = root.0.join("report.txt");
    fs::write(&report, b"keep-this-report\n").unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_symlog"));
    command
        .args([
            "--plain",
            "--no-color",
            "--output",
            report.to_str().expect("utf-8 path"),
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut guard = ChildGuard::spawn(command);
    let pid = guard.pid();
    let mut stdout = guard.child.stdout.take().expect("stdout pipe");
    let mut stderr = guard.child.stderr.take().expect("stderr pipe");

    assert!(wait_until_blocked(pid), "child {pid} did not block in read");
    assert_eq!(
        parent_of(pid),
        Some(std::process::id()),
        "refuse to signal a process that is not this test's child"
    );
    let sent = guard.pidfd.signal(SIGINT);
    assert_eq!(sent, 0, "pidfd_send_signal(SIGINT) returned {sent}");

    let status = stop_child(&mut guard);
    let mut out = Vec::new();
    let mut err = Vec::new();
    stdout.read_to_end(&mut out).expect("read stdout");
    stderr.read_to_end(&mut err).expect("read stderr");
    let err = String::from_utf8(err).expect("stderr utf-8");

    assert_eq!(status.code(), Some(130), "stderr={err}");
    assert!(!status.success());
    assert_ne!(status.code(), Some(0));
    assert!(out.is_empty(), "stdout={out:?}");
    assert_eq!(err, "onderbroken door SIGINT\n");
    assert!(!err.contains("panicked"));
    assert_eq!(fs::read(&report).unwrap(), b"keep-this-report\n");
    let names: Vec<_> = fs::read_dir(&root.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "leftover files: {names:?}");
}
