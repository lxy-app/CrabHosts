#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = crabhosts_system_client::serve_helper() {
        eprintln!("CrabHosts Helper failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("crabhosts-macos-helper is supported only on macOS");
    std::process::exit(1);
}
