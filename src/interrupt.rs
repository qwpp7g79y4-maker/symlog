//! SIGINT zet een vlag zodat het proces kan stoppen zonder een half rapport.
//!
//! De handler staat op Linux x86_64, waar de glibc-`sigaction`-indeling hieronder
//! is gemeten (152 bytes). Andere doelen houden de standaardafhandeling.
//! Een geblokkeerde `read` geeft dan `EINTR` terug, omdat `sa_flags` 0 is
//! (geen herstart van de systeemaanroep).

use std::sync::atomic::{AtomicBool, Ordering};

/// Exitstatus voor SIGINT. De gewone codes 0, 1, 2, 3 en 4 blijven staan.
pub const EXIT_INTERRUPTED: i32 = 130;

/// Enige stderr-regel na SIGINT. Geen hint, zodat een pipe precies deze regel ziet.
pub const INTERRUPTED_MESSAGE: &str = "onderbroken door SIGINT";

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

pub fn install() {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    install_handler();
}

/// Voert `body` uit terwijl SIGINT wordt tegengehouden, en laat een wachtend
/// signaal daarna door.
///
/// Het hernoemen van een af rapport gebruikt dit, zodat het signaal de
/// hernoeming niet halverwege kan knippen. Zonder handler is dit alleen `body`.
pub fn with_sigint_blocked<T>(body: impl FnOnce() -> T) -> T {
    block_sigint();
    let guard = Unblock;
    let value = body();
    drop(guard);
    value
}

struct Unblock;

impl Drop for Unblock {
    fn drop(&mut self) {
        unblock_sigint();
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn block_sigint() {}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn unblock_sigint() {}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SIGINT: i32 = 2;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SIG_BLOCK: i32 = 0;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SIG_UNBLOCK: i32 = 1;

/// glibc `sigset_t` op dit doel: 1024 bits.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[repr(C)]
struct Sigset {
    _val: [u64; 16],
}

/// glibc `struct sigaction` op dit doel. Gemeten offsets: handler 0, masker 8,
/// vlaggen 136, restorer 144, grootte 152.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[repr(C)]
struct Sigaction {
    sa_handler: usize,
    sa_mask: Sigset,
    sa_flags: i32,
    sa_restorer: usize,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const _: () = {
    assert!(std::mem::size_of::<Sigset>() == 128);
    assert!(std::mem::size_of::<Sigaction>() == 152);
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe extern "C" {
    fn sigaction(sig: i32, act: *const Sigaction, old: *mut Sigaction) -> i32;
    fn sigemptyset(set: *mut Sigset) -> i32;
    fn sigaddset(set: *mut Sigset, sig: i32) -> i32;
    fn sigprocmask(how: i32, set: *const Sigset, old: *mut Sigset) -> i32;
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_handler() {
    extern "C" fn on_sigint(_sig: i32) {
        // Eén store is het signaalveilige deel. De rest van de stop loopt
        // terug op het hoofdpad, dat het rapport afmaakt of weggooit.
        INTERRUPTED.store(true, Ordering::SeqCst);
    }

    // SAFETY: de structuur komt overeen met glibc `sigaction` op linux x86_64.
    // `sa_flags` blijft 0 zodat een geblokkeerde `read` EINTR teruggeeft, en
    // glibc vult `sa_restorer` zelf in.
    unsafe {
        let mut action: Sigaction = std::mem::zeroed();
        if sigemptyset(&mut action.sa_mask) != 0 {
            return;
        }
        action.sa_handler = on_sigint as *const () as usize;
        action.sa_flags = 0;
        let _ = sigaction(SIGINT, &action, std::ptr::null_mut());
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn sigint_set() -> Option<Sigset> {
    // SAFETY: `Sigset` is glibc's `sigset_t` op dit doel. Nulgeheugen is een
    // leeg masker; `sigemptyset` maakt dat expliciet.
    unsafe {
        let mut set: Sigset = std::mem::zeroed();
        if sigemptyset(&mut set) != 0 || sigaddset(&mut set, SIGINT) != 0 {
            return None;
        }
        Some(set)
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn block_sigint() {
    let Some(set) = sigint_set() else {
        return;
    };
    // SAFETY: `set` is een gevulde glibc `sigset_t` en SIG_BLOCK is 0.
    unsafe {
        let _ = sigprocmask(SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn unblock_sigint() {
    let Some(set) = sigint_set() else {
        return;
    };
    // SAFETY: hetzelfde masker als `block_sigint`. Vrijgeven levert een
    // wachtend SIGINT af aan `on_sigint`, dat alleen de vlag zet.
    unsafe {
        let _ = sigprocmask(SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
}
