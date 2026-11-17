fn main() {
    // Link the private SkyLight framework for WindowServer window notifications
    // (see src/window_events.rs) plus the Carbon event loop that delivers them.
    println!("cargo:rustc-link-search=framework=/System/Library/PrivateFrameworks");
    println!("cargo:rustc-link-lib=framework=SkyLight");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=framework=Carbon");
}
