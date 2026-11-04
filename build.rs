// Embed the app icon in the Windows .exe (shown in Explorer, shortcuts and the taskbar).
fn main() {
    println!("cargo:rerun-if-changed=packaging/litanatomy.ico");
    // build.rs runs on the host, so check the target via the env var, not cfg!.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("packaging/litanatomy.ico")
            .compile()
            .expect("failed to embed the Windows icon (needs rc.exe from the Windows SDK)");
    }
}
