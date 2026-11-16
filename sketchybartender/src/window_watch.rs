//! Window-set watcher, shared by all window-manager integrations.
//!
//! An event can be lost: AeroSpace sends no window-closed event, and the
//! workspace repaint is debounced. Thus this watcher compares a fingerprint of
//! the window set (`wm::window_signature`) with the one the bar shows, and
//! repaints when they differ. It wakes on a nudge (from the event thread or from
//! `crate::window_events`) or every `WINDOW_POLL_INTERVAL`.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::handlers::{handle_workspace_refresh, DaemonState};
use crate::wm;

/// How often to check the window set for changes that send no event.
const WINDOW_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long to wait after a nudge before the window set is sampled. Nudges
/// come before the window manager has updated its state (AeroSpace drops a
/// closed window ~75ms later), so an immediate sample shows no change.
const NUDGE_SETTLE: Duration = Duration::from_millis(150);

/// Start the watcher thread. Returns the sender that nudges it.
pub fn spawn(state: Arc<Mutex<DaemonState>>) -> Sender<()> {
    let (nudge_tx, nudge_rx) = mpsc::channel();
    thread::spawn(move || poll_loop(state, nudge_rx));
    nudge_tx
}

/// The sampling runs on this thread, so the wait for the window manager to
/// settle never stops event dispatch.
fn poll_loop(state: Arc<Mutex<DaemonState>>, nudge_rx: Receiver<()>) {
    loop {
        if nudge_rx.recv_timeout(WINDOW_POLL_INTERVAL).is_ok() {
            // Collapse a burst of nudges into one sample.
            while nudge_rx.try_recv().is_ok() {}
            thread::sleep(NUDGE_SETTLE);
        }
        refresh_if_windows_changed(&state);
    }
}

fn window_manager(state: &Arc<Mutex<DaemonState>>) -> Option<crate::config::WindowManager> {
    state.lock().ok().map(|s| s.config.window_manager)
}

/// Repaint the workspace items and record the window set they show.
pub fn refresh_workspaces(state: &Arc<Mutex<DaemonState>>) {
    if !handle_workspace_refresh(state) {
        // Debounced. The bar can show an old window set, so keep the old
        // signature and let the poll find the change.
        return;
    }

    // Sample *after* the repaint, so the poll does not do the same work again.
    let signature = window_manager(state).and_then(wm::window_signature);
    if let Ok(mut s) = state.lock() {
        s.last_window_signature = signature;
    }
}

/// Repaint only if the window set is different from the one on the bar.
fn refresh_if_windows_changed(state: &Arc<Mutex<DaemonState>>) {
    let signature = match window_manager(state).and_then(wm::window_signature) {
        Some(sig) => sig,
        // Window manager not available. Keep the last signature, so we repaint
        // when it comes back, not on the failure.
        None => return,
    };

    let unchanged = match state.lock() {
        Ok(s) => s.last_window_signature.as_deref() == Some(signature.as_str()),
        Err(_) => return,
    };

    // Must be outside the lock: handle_workspace_refresh locks the state itself.
    if !unchanged {
        refresh_workspaces(state);
    }
}
