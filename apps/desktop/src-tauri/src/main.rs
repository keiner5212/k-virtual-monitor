#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.iter().any(|arg| arg == "--pair") {
        if let Err(error) = kvm_desktop_lib::print_pairing() {
            eprintln!("kvm-desktop: {error}");
            std::process::exit(1);
        }
        return;
    }
    if arguments.iter().any(|arg| arg == "--headless") {
        if let Err(error) = kvm_desktop_lib::run_headless() {
            eprintln!("kvm-desktop: {error}");
            std::process::exit(1);
        }
        return;
    }
    kvm_desktop_lib::run();
}
