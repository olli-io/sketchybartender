//! Lightweight CLI tool that forwards messages to the daemon via socket

use std::env;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn get_socket_path() -> PathBuf {
    let cache_dir = env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = env::var("HOME").expect("HOME not set");
            PathBuf::from(home).join(".cache")
        });

    cache_dir.join("sketchybar").join("helper.sock")
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: sketchycli <command> [args...]");
        std::process::exit(1);
    }

    // Forward all arguments (excluding program name) to daemon
    let message = args[1..].join(" ");

    // Forward to daemon
    let socket_path = get_socket_path();
    match UnixStream::connect(&socket_path) {
        Ok(mut stream) => {
            if let Err(e) = writeln!(stream, "{}", message) {
                eprintln!("Failed to send message: {}", e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Failed to connect to daemon at {:?}: {}", socket_path, e);
            eprintln!("Is sketchybartender daemon running?");
            std::process::exit(1);
        }
    }
}
