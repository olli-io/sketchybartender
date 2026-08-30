//! WindowServer window-lifecycle notifications.
//!
//! AeroSpace emits no window-closed event, so a closed window is invisible to
//! the event stream in `aerospace_events`. macOS itself does report it: the
//! WindowServer sends connection notifications when a window is created or
//! destroyed, which is how sketchybar's built-in `space_windows_change` event
//! was produced before the socket migration (see `src/app_windows.c` upstream).
//! Registering for them here gets that signal back without routing it through a
//! sketchybar listener item and `sketchycli`.
//!
//! These are private SkyLight APIs, so treat them as best-effort: if any part of
//! the setup fails we log and carry on, and the periodic window-set poll in
//! `aerospace_events` still catches the change.
//!
//! Three things are required, and all of them matter:
//!   1. `NSApplicationLoad()` before touching SkyLight, to bootstrap the Cocoa
//!      application environment.
//!   2. `SLSRegisterNotifyProc` for the event numbers we care about.
//!   3. `RunApplicationEventLoop()` on the **main thread**. A bare
//!      `CFRunLoopRun()` is not enough — registration succeeds but the
//!      WindowServer never delivers a single callback. yabai uses `[NSApp run]`
//!      and sketchybar uses `RunApplicationEventLoop()` for the same reason.

use std::ffi::c_void;
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

/// WindowServer notification: a window was created.
const EVENT_WINDOW_CREATED: u32 = 1325;
/// WindowServer notification: a window was destroyed. This is the one AeroSpace
/// never tells us about.
const EVENT_WINDOW_DESTROYED: u32 = 1326;

#[link(name = "SkyLight", kind = "framework")]
extern "C" {
    fn SLSMainConnectionID() -> i32;
    fn SLSRegisterNotifyProc(
        handler: extern "C" fn(u32, *mut c_void, usize, *mut c_void),
        event: u32,
        context: *mut c_void,
    ) -> i32;
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {
    fn NSApplicationLoad() -> bool;
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn RunApplicationEventLoop();
}

/// Channel to the window-set watcher. The notify proc runs on the main event
/// loop, so it must not block — all it does is nudge the watcher, which samples
/// AeroSpace and repaints on its own thread.
static NUDGE: OnceLock<Sender<()>> = OnceLock::new();

/// The WindowServer passes a `{ sid: u64, wid: u32 }` payload here, but we don't
/// need it: the watcher diffs the whole window set against AeroSpace anyway, and
/// AeroSpace is the authority on which workspace a window belonged to.
extern "C" fn on_window_lifecycle(_event: u32, _data: *mut c_void, _len: usize, _ctx: *mut c_void) {
    if let Some(tx) = NUDGE.get() {
        let _ = tx.send(());
    }
}

/// Take over the main thread: register for window create/destroy notifications
/// and run the event loop that delivers them. Never returns.
///
/// Must be called on the main thread, and callers must have moved every other
/// long-running task onto its own thread first.
pub fn run(nudge: Sender<()>) -> ! {
    if NUDGE.set(nudge).is_err() {
        eprintln!("[window-events] Already running");
    }

    if !unsafe { NSApplicationLoad() } {
        eprintln!("[window-events] NSApplicationLoad failed; window close detection falls back to polling");
    }

    let cid = unsafe { SLSMainConnectionID() };
    if cid == 0 {
        eprintln!("[window-events] No WindowServer connection; window close detection falls back to polling");
    } else {
        for event in [EVENT_WINDOW_CREATED, EVENT_WINDOW_DESTROYED] {
            let err = unsafe {
                SLSRegisterNotifyProc(on_window_lifecycle, event, cid as isize as *mut c_void)
            };
            if err != 0 {
                eprintln!(
                    "[window-events] Failed to register for WindowServer event {}: error {}",
                    event, err
                );
            }
        }
        eprintln!("[window-events] Watching WindowServer window create/destroy");
    }

    // Blocks forever. Even if registration failed above, this parks the main
    // thread while the worker threads keep the bar running.
    unsafe { RunApplicationEventLoop() };

    unreachable!("RunApplicationEventLoop returned");
}
