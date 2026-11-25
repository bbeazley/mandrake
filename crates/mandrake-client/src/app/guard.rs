use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

static CAUGHT: AtomicI32 = AtomicI32::new(0);
static HEARTBEAT: AtomicU64 = AtomicU64::new(0);
static TICK: AtomicU32 = AtomicU32::new(u32::MAX);
static STARTED: AtomicBool = AtomicBool::new(false);

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) const HANG_SECS: u64 = 20;

#[cfg(all(unix, not(target_arch = "wasm32")))]
mod imp {
    use super::{CAUGHT, Ordering};

    const SIGHUP: i32 = 1;
    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;
    const SIG_DFL: usize = 0;

    unsafe extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }

    extern "C" fn on_signal(sig: i32) {
        CAUGHT.store(sig, Ordering::SeqCst);
        unsafe {
            signal(sig, SIG_DFL);
        }
    }

    pub(super) fn install() {
        let handler = on_signal as extern "C" fn(i32) as usize;
        for sig in [SIGHUP, SIGINT, SIGTERM] {
            unsafe {
                signal(sig, handler);
            }
        }
    }
}

#[cfg(not(all(unix, not(target_arch = "wasm32"))))]
mod imp {
    pub(super) fn install() {}
}

pub(crate) fn install() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    imp::install();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let start = std::time::Instant::now();
        HEARTBEAT.store(1, Ordering::SeqCst);
        let clock = move || start.elapsed().as_millis() as u64 + 1;
        CLOCK.with(|c| *c.borrow_mut() = Some(Box::new(clock)));
        let _ = std::thread::Builder::new()
            .name("m2-watchdog".into())
            .spawn(move || watchdog(clock));
    }
}

thread_local! {
    static CLOCK: std::cell::RefCell<Option<Box<dyn Fn() -> u64>>> =
        const { std::cell::RefCell::new(None) };
}

pub(crate) fn frame(tick: Option<u32>) {
    let now = CLOCK.with(|c| c.borrow().as_ref().map(|f| f()));
    if let Some(now) = now {
        HEARTBEAT.store(now, Ordering::SeqCst);
        TICK.store(tick.unwrap_or(u32::MAX), Ordering::SeqCst);
    }
}

pub(crate) fn take_signal() -> Option<i32> {
    match CAUGHT.swap(0, Ordering::SeqCst) {
        0 => None,
        sig => Some(sig),
    }
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn hang_line(silent: u64, tick: u32) -> String {
    let at = if tick == u32::MAX {
        "with no map".to_string()
    } else {
        format!("at game tick {tick}")
    };
    format!(
        "mandrake: the window has run no frame for {} s, {at}: the client is hung (the last \
         recording holds the game to its last 30 s checkpoint)",
        silent / 1000
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn watchdog(clock: impl Fn() -> u64) {
    let mut reported = false;
    loop {
        std::thread::sleep(Duration::from_secs(1));
        let last = HEARTBEAT.load(Ordering::SeqCst);
        let silent = clock().saturating_sub(last);
        if silent >= HANG_SECS * 1000 {
            if !reported {
                reported = true;
                tracing::error!("{}", hang_line(silent, TICK.load(Ordering::SeqCst)));
            }
        } else if reported {
            reported = false;
            tracing::error!("mandrake: the window runs frames again");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hang_names_its_tick() {
        assert!(hang_line(21_500, 1200).contains("no frame for 21 s, at game tick 1200"));
        assert!(hang_line(20_000, u32::MAX).contains("with no map"));
    }

    #[test]
    fn a_signal_is_taken_once() {
        CAUGHT.store(2, Ordering::SeqCst);
        assert_eq!(take_signal(), Some(2));
        assert_eq!(take_signal(), None);
    }
}
