//! Direct subscription to AeroSpace server events.
//!
//! Instead of routing workspace/focus changes through a sketchybar listener item
//! and `sketchycli`, the daemon holds one persistent socket connection subscribed
//! to AeroSpace's event stream and dispatches each event straight to the matching
//! handler. See the socket protocol's `subscribe` mode:
//! https://nikitabobko.github.io/AeroSpace/guide#socket-protocol
//!
//! Observed event names (from `aerospace subscribe --all`):
//!   focused-workspace-changed, window-detected, focus-changed,
//!   focused-monitor-changed, mode-changed, binding-triggered

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::aerospace_socket::Subscription;
use crate::handlers::{handle_focus_refresh, handle_workspace_refresh, DaemonState};

/// Spawn the background thread that subscribes to AeroSpace events and drives the
/// bar directly. Reconnects with backoff if AeroSpace restarts or the stream drops.
pub fn spawn(state: Arc<Mutex<DaemonState>>) {
    thread::spawn(move || run_loop(state));
}

fn run_loop(state: Arc<Mutex<DaemonState>>) {
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
                        Ok(event) => dispatch(&event, &state),
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
fn dispatch(event: &serde_json::Value, state: &Arc<Mutex<DaemonState>>) {
    let kind = event.get("_event").and_then(|v| v.as_str()).unwrap_or("");

    match kind {
        // A workspace gained focus, a window appeared/moved, or monitor focus
        // changed — any of these can change what the workspace items should show.
        "focused-workspace-changed" | "window-detected" | "focused-monitor-changed" => {
            handle_workspace_refresh(state);
        }
        // The focused window changed — refresh the front_app item. We don't get
        // an app name in the event, so pass None and let the handler query
        // AeroSpace for the focused app.
        "focus-changed" => {
            handle_focus_refresh(None, state);
        }
        // mode-changed / binding-triggered / anything else: nothing to draw.
        _ => {}
    }
}
