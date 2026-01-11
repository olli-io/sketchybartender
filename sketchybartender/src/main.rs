mod aerospace;
mod config;
mod daemon;
mod handlers;
mod icon_map;
mod mach_client;
mod providers;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use handlers::DaemonState;

fn main() {
    // Load configuration
    let config = config::Config::load();

    // Shared state
    let state = Arc::new(Mutex::new(DaemonState::new(config.clone())));

    // Spawn timer threads for periodic updates using configured intervals
    let clock_interval = config.clock_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(clock_interval));
            handlers::handle_clock_refresh();
        }
    });

    let battery_interval = config.battery_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(battery_interval));
            handlers::handle_battery_refresh(None);
        }
    });

    let brew_interval = config.brew_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(brew_interval));
            handlers::handle_brew_refresh();
        }
    });

    let teams_interval = config.teams_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(teams_interval));
            handlers::handle_teams_refresh();
        }
    });

    let system_interval = config.system_interval;
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(system_interval));
            handlers::handle_system_refresh();
        }
    });

    // Start the daemon socket listener
    daemon::start_daemon(state);
}
