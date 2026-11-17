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
//! which is why the shared watcher in `crate::window_watch` compares the window
//! set (see `window_signature`). The prompt signal for a closure comes from the
//! WindowServer — see `crate::window_events`, which nudges that watcher.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::aerospace_socket::{self, Subscription};
use crate::handlers::DaemonState;
use crate::window_watch::{self, refresh_workspaces};

/// Spawn the background threads that keep the bar in sync with AeroSpace: one
/// subscribed to the event stream, one watching the window set as a safety net
/// for closures AeroSpace does not report.
///
/// Returns a sender that makes the watcher re-check the window set. Anything
/// that learns of a window appearing or disappearing can nudge it; the watcher
/// only repaints when the window set actually differs.
pub fn spawn(state: Arc<Mutex<DaemonState>>) -> Sender<()> {
    let nudge_tx = window_watch::spawn(Arc::clone(&state));
    let event_nudge = nudge_tx.clone();
    thread::spawn(move || run_loop(state, event_nudge));
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

/// Route a single event to the appropriate handler based on its `_event` field.
fn dispatch(event: &serde_json::Value, state: &Arc<Mutex<DaemonState>>, nudge_tx: &Sender<()>) {
    let kind = event.get("_event").and_then(|v| v.as_str()).unwrap_or("");

    match kind {
        // A workspace gained focus, a window appeared/moved, or monitor focus
        // changed — any of these can change what the workspace items should show.
        "focused-workspace-changed" | "window-detected" | "focused-monitor-changed" => {
            refresh_workspaces(state);
        }
        // The focused window changed. This is also the only notification we
        // get when the focused window is closed, so nudge the watcher: it makes
        // the common case (closing the window you're looking at) repaint
        // promptly instead of waiting out the poll interval.
        "focus-changed" => {
            let _ = nudge_tx.send(());
        }
        // mode-changed / binding-triggered / anything else: nothing to draw.
        _ => {}
    }
}

/// A cheap fingerprint of every window AeroSpace knows about and the workspace
/// it lives on. Lines are sorted so a reordering of AeroSpace's window tree
/// doesn't read as a change.
pub fn window_signature() -> Option<String> {
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
