//! Direct subscription to AeroSpace server events.
//!
//! Instead of routing workspace/focus changes through a sketchybar listener item
//! and `sketchycli`, the daemon holds one persistent socket connection subscribed
//! to AeroSpace's event stream and dispatches each event straight to the matching
//! handler. See the socket protocol's `subscribe` mode:
//! https://nikitabobko.github.io/AeroSpace/guide#socket-protocol
//!
//! Observed event names (from `aerospace subscribe --all`, 0.21.3-Beta):
//!   focused-workspace-changed, window-detected, focus-changed,
//!   focused-monitor-changed, mode-changed, binding-triggered
//!
//! Note what is *missing* from that list: there is no window-closed event. A
//! window disappearing is only ever observable as a change in `list-windows`,
//! which is why this module also watches the window set (see `window_signature`).
//! Before the socket migration this was covered by sketchybar's built-in
//! `space_windows_change` event driving a `sketchycli on-workspace-changed` hop.
//!
//! The prompt signal for a closure comes from the WindowServer instead — see
//! `crate::window_events`, which nudges the watcher spawned here. The periodic
//! poll stays as a fallback for whenever that private API stops delivering.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::aerospace_socket::{self, Subscription};
use crate::handlers::{handle_focus_refresh, handle_workspace_refresh, DaemonState};

/// How often to re-check the window set for closures that produce no event at
/// all — a background app quitting, or a window closing in a workspace other
/// than the focused one.
const WINDOW_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long to wait after a nudge before sampling the window set. Both nudge
/// sources fire while the closing window is still in `list-windows` — AeroSpace
/// only drops it ~75ms later — so sampling immediately would see no change.
const NUDGE_SETTLE: Duration = Duration::from_millis(150);

/// Spawn the background threads that keep the bar in sync with AeroSpace: one
/// subscribed to the event stream, one watching the window set as a safety net
/// for closures AeroSpace does not report.
///
/// Returns a sender that makes the watcher re-check the window set. Anything
/// that learns of a window appearing or disappearing can nudge it; the watcher
/// only repaints when the window set actually differs.
pub fn spawn(state: Arc<Mutex<DaemonState>>) -> Sender<()> {
    let (nudge_tx, nudge_rx) = mpsc::channel();
    let poll_state = Arc::clone(&state);
    let event_nudge = nudge_tx.clone();
    thread::spawn(move || run_loop(state, event_nudge));
    thread::spawn(move || poll_loop(poll_state, nudge_rx));
    nudge_tx
}

fn run_loop(state: Arc<Mutex<DaemonState>>, nudge_tx: Sender<()>) {
    // Backoff between reconnect attempts, capped so a persistently-down server
    // doesn't spin.
    const MIN_BACKOFF: Duration = Duration::from_millis(250);
    const MAX_BACKOFF: Duration = Duration::from_secs(5);
    let mut backoff = MIN_BACKOFF;

    loop {
        // Subscribe to every event; ask for the initial state so the bar paints
        // correctly right after (re)connecting.
        match Subscription::open(&[], true, true) {
            Ok(mut sub) => {
                eprintln!("[aerospace-events] Subscribed to AeroSpace event stream");
                backoff = MIN_BACKOFF; // reset after a successful connect

                loop {
                    match sub.next_event() {
                        Ok(event) => dispatch(&event, &state, &nudge_tx),
                        Err(e) => {
                            eprintln!("[aerospace-events] Event stream ended: {}", e);
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("[aerospace-events] Failed to subscribe: {}", e);
            }
        }

        thread::sleep(backoff);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Repaint the workspace items whenever the window set changes without an event
/// to announce it. Wakes on a nudge from the event thread, or on its own every
/// `WINDOW_POLL_INTERVAL` for changes that produce no event at all.
///
/// The sampling runs here rather than on the event thread so that waiting for
/// AeroSpace to settle never stalls event dispatch.
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

/// Route a single event to the appropriate handler based on its `_event` field.
fn dispatch(event: &serde_json::Value, state: &Arc<Mutex<DaemonState>>, nudge_tx: &Sender<()>) {
    let kind = event.get("_event").and_then(|v| v.as_str()).unwrap_or("");

    match kind {
        // A workspace gained focus, a window appeared/moved, or monitor focus
        // changed — any of these can change what the workspace items should show.
        "focused-workspace-changed" | "window-detected" | "focused-monitor-changed" => {
            refresh_workspaces(state);
        }
        // The focused window changed — refresh the front_app item. We don't get
        // an app name in the event, so pass None and let the handler query
        // AeroSpace for the focused app.
        //
        // This is also the only notification we get when the focused window is
        // closed, so nudge the watcher: it makes the common case (closing the
        // window you're looking at) repaint promptly instead of waiting out the
        // poll interval.
        "focus-changed" => {
            handle_focus_refresh(None, state);
            let _ = nudge_tx.send(());
        }
        // mode-changed / binding-triggered / anything else: nothing to draw.
        _ => {}
    }
}

/// A cheap fingerprint of every window AeroSpace knows about and the workspace
/// it lives on. Lines are sorted so a reordering of AeroSpace's window tree
/// doesn't read as a change.
fn window_signature() -> Option<String> {
    let stdout = aerospace_socket::run(&[
        "list-windows",
        "--all",
        "--format",
        "%{window-id}%{workspace}",
    ])
    .ok()?;

    let mut lines: Vec<&str> = stdout.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    lines.sort_unstable();
    Some(lines.join("\n"))
}

/// Repaint the workspace items and record the window set they were painted from.
fn refresh_workspaces(state: &Arc<Mutex<DaemonState>>) {
    if !handle_workspace_refresh(state) {
        // Debounced — the bar still shows the old window set, so leave the
        // recorded signature stale and let the poll pick the change back up.
        return;
    }

    // Sample the signature *after* the repaint so the poll doesn't immediately
    // redo the same work.
    let signature = window_signature();
    if let Ok(mut s) = state.lock() {
        s.last_window_signature = signature;
    }
}

/// Repaint only if the window set differs from the one the bar currently shows.
fn refresh_if_windows_changed(state: &Arc<Mutex<DaemonState>>) {
    let signature = match window_signature() {
        Some(sig) => sig,
        // AeroSpace unreachable — leave the last known signature in place so we
        // repaint once it comes back rather than on the failure itself.
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
