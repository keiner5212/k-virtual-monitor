#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|arg| arg == "--headless") {
        if let Err(error) = kvm_desktop_lib::run_headless() {
            eprintln!("kvm-desktop: {error}");
            std::process::exit(1);
        }
        return;
    }
    kvm_desktop_lib::run();
}
