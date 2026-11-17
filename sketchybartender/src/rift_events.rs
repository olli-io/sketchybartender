//! Subscription to Rift events over its Mach IPC service.
//!
//! Rift sends each event to a Mach port that we own. When Rift stops, nothing
//! tells that port, and the blocked receive in `rift-client` never returns. Thus
//! we keep a send right to Rift's service port: when Rift stops, that right
//! becomes a dead name. A supervisor sees this and subscribes again when Rift
//! comes back. The old receiver thread stays blocked; one parked thread for each
//! Rift restart is an acceptable cost.
//!
//! Rift sends `windows_changed` when a window closes, so a closure does not
//! need the WindowServer nudge. The shared watcher in `crate::window_watch`
//! still runs, because the workspace repaint is debounced and can drop an event.

use std::ffi::CString;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mach2::bootstrap::bootstrap_look_up;
use mach2::kern_return::{kern_return_t, KERN_SUCCESS};
use mach2::mach_port::mach_port_deallocate;
use mach2::port::{mach_port_name_t, mach_port_t};
use mach2::task::task_get_special_port;
use mach2::task_special_ports::TASK_BOOTSTRAP_PORT;
use mach2::traps::mach_task_self;
use rift_client::{EventKind, RiftMachClient};

use crate::handlers::DaemonState;
use crate::window_watch::{self, refresh_workspaces};

const DEFAULT_SERVICE_NAME: &str = "git.acsandmann.rift";

/// How often to check that Rift is still the same process.
const LIVENESS_INTERVAL: Duration = Duration::from_secs(1);

/// `MACH_PORT_TYPE(MACH_PORT_RIGHT_DEAD_NAME)`
const MACH_PORT_TYPE_DEAD_NAME: u32 = 1 << 20;

extern "C" {
    fn mach_port_type(task: mach_port_t, name: mach_port_name_t, ptype: *mut u32) -> kern_return_t;
}

/// A send right to Rift's service port. It becomes a dead name when Rift stops.
struct ServiceRight(mach_port_t);

impl ServiceRight {
    fn look_up() -> Option<Self> {
        // rift-cli and rift-client read the same variable.
        let name = std::env::var("RIFT_BS_NAME").unwrap_or_else(|_| DEFAULT_SERVICE_NAME.to_owned());
        let name = CString::new(name).ok()?;
        unsafe {
            let mut bootstrap: mach_port_t = 0;
            if task_get_special_port(mach_task_self(), TASK_BOOTSTRAP_PORT, &mut bootstrap) != KERN_SUCCESS {
                return None;
            }
            let mut port: mach_port_t = 0;
            let kr = bootstrap_look_up(bootstrap, name.as_ptr(), &mut port);
            mach_port_deallocate(mach_task_self(), bootstrap);
            (kr == KERN_SUCCESS).then_some(ServiceRight(port))
        }
    }

    fn is_dead(&self) -> bool {
        let mut ptype = 0u32;
        let kr = unsafe { mach_port_type(mach_task_self(), self.0, &mut ptype) };
        kr != KERN_SUCCESS || ptype & MACH_PORT_TYPE_DEAD_NAME != 0
    }
}

impl Drop for ServiceRight {
    fn drop(&mut self) {
        unsafe {
            mach_port_deallocate(mach_task_self(), self.0);
        }
    }
}

/// Start the threads that keep the bar in sync with Rift. Returns the sender
/// that nudges the window-set watcher.
pub fn spawn(state: Arc<Mutex<DaemonState>>) -> Sender<()> {
    let nudge_tx = window_watch::spawn(Arc::clone(&state));
    let event_nudge = nudge_tx.clone();
    thread::spawn(move || supervise(state, event_nudge));
    nudge_tx
}

/// Subscribe, wait until Rift stops, and do it again.
fn supervise(state: Arc<Mutex<DaemonState>>, nudge_tx: Sender<()>) {
    const MIN_BACKOFF: Duration = Duration::from_millis(250);
    const MAX_BACKOFF: Duration = Duration::from_secs(5);
    let mut backoff = MIN_BACKOFF;

    loop {
        match subscribe_until_rift_stops(&state, &nudge_tx) {
            Ok(()) => {
                eprintln!("[rift-events] Rift stopped; waiting to subscribe again");
                backoff = MIN_BACKOFF;
            }
            Err(e) => eprintln!("[rift-events] Failed to subscribe: {}", e),
        }

        thread::sleep(backoff);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Subscribe to all events and dispatch them on a new thread. Returns when Rift
/// stops, or with `Err` if the subscription failed.
fn subscribe_until_rift_stops(state: &Arc<Mutex<DaemonState>>, nudge_tx: &Sender<()>) -> Result<(), String> {
    // Get the liveness right before the subscription: if Rift restarts between
    // the two, the right is already dead and we subscribe again.
    let right = ServiceRight::look_up().ok_or("service is not registered")?;
    let subscription = RiftMachClient.subscribe(EventKind::All).map_err(|e| e.to_string())?;
    eprintln!("[rift-events] Subscribed to Rift events");

    let reader_state = Arc::clone(state);
    let reader_nudge = nudge_tx.clone();
    thread::spawn(move || loop {
        match subscription.recv_event_value() {
            Ok(event) => dispatch(&event, &reader_state, &reader_nudge),
            Err(e) => {
                eprintln!("[rift-events] Event stream ended: {}", e);
                return;
            }
        }
    });

    // Rift sends no initial state, so paint it now.
    refresh_workspaces(state);

    while !right.is_dead() {
        thread::sleep(LIVENESS_INTERVAL);
    }
    Ok(())
}

/// Route one event to its handler, from its `type` field.
fn dispatch(event: &serde_json::Value, state: &Arc<Mutex<DaemonState>>, nudge_tx: &Sender<()>) {
    let kind = event.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match kind {
        // The active workspace changed, or the windows in a workspace changed
        // (this includes a closed window).
        "workspace_changed" | "windows_changed" => {
            refresh_workspaces(state);
            // The repaint can be debounced. The watcher finds what it dropped.
            let _ = nudge_tx.send(());
        }
        // Focus, layout, stacks, titles, binding mode, config: nothing to draw.
        _ => {}
    }
}
