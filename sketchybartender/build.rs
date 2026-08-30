use serde::Deserialize;
use std::collections::HashSet;
use std::env;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

#[derive(Deserialize)]
struct IconEntry {
    #[serde(rename = "iconName")]
    icon_name: String,
    #[serde(rename = "appNames")]
    app_names: Vec<String>,
}

fn main() {
    // Link the private SkyLight framework for WindowServer window notifications
    // (see src/window_events.rs) plus the Carbon event loop that delivers them.
    println!("cargo:rustc-link-search=framework=/System/Library/PrivateFrameworks");
    println!("cargo:rustc-link-lib=framework=SkyLight");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=Carbon");

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("icon_map.rs");

    // Tell Cargo to rerun if the JSON changes
    println!("cargo:rerun-if-changed=src/icon_map.json");

    // Read and parse the JSON file
    let json_path = Path::new("src/icon_map.json");
    let file = File::open(json_path).expect("Failed to open icon_map.json");
    let reader = BufReader::new(file);
    let entries: Vec<IconEntry> =
        serde_json::from_reader(reader).expect("Failed to parse icon_map.json");

    // Track seen app names to avoid duplicates (first occurrence wins)
    let mut seen: HashSet<String> = HashSet::new();

    // Separate exact matches from prefix patterns
    let mut exact_matches: Vec<(String, String)> = Vec::new();
    let mut prefix_patterns: Vec<(String, String)> = Vec::new();

    for entry in &entries {
        for app_name in &entry.app_names {
            if app_name.ends_with('*') {
                // Wildcard pattern - store without the asterisk
                let prefix = app_name[..app_name.len() - 1].to_string();
                if seen.insert(prefix.clone()) {
                    prefix_patterns.push((prefix, entry.icon_name.clone()));
                }
            } else if app_name.contains('|') {
                // Handle pipe-separated names (e.g., "MATLAB |MATLABWindow")
                for name in app_name.split('|') {
                    let name = name.trim().to_string();
                    if !name.is_empty() && seen.insert(name.clone()) {
                        exact_matches.push((name, entry.icon_name.clone()));
                    }
                }
            } else {
                let name = app_name.clone();
                if seen.insert(name.clone()) {
                    exact_matches.push((name, entry.icon_name.clone()));
                }
            }
        }
    }

    // Generate the output file
    let mut out_file = BufWriter::new(File::create(&dest_path).unwrap());

    // Generate the PHF map for exact matches
    writeln!(out_file, "static ICON_MAP: phf::Map<&'static str, &'static str> = ").unwrap();
    let mut builder = phf_codegen::Map::new();
    for (app_name, icon_name) in &exact_matches {
        builder.entry(&**app_name, &format!("\"{}\"", icon_name));
    }
    writeln!(out_file, "{};", builder.build()).unwrap();

    // Generate the prefix patterns array
    writeln!(out_file).unwrap();
    writeln!(
        out_file,
        "static PREFIX_PATTERNS: &[(&str, &str)] = &["
    )
    .unwrap();
    for (prefix, icon_name) in &prefix_patterns {
        writeln!(out_file, "    (\"{}\", \"{}\"),", prefix, icon_name).unwrap();
    }
    writeln!(out_file, "];").unwrap();
}
