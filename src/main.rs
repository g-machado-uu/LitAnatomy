// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod citekey;
mod db;
mod export;
mod import;
mod latex;
mod model;
mod net;
mod pdf;
mod settings;
mod ui;
#[cfg(test)]
mod ui_tests;

fn main() -> eframe::Result {
    // Optional: a library file to open instead of the one in Settings.
    let library = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LitAnatomy")
            .with_app_id("litanatomy")
            .with_inner_size([1360.0, 880.0])
            .with_min_inner_size([980.0, 620.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../packaging/litanatomy.png"))
                    .expect("bundled icon is a valid PNG"),
            ),
        ..Default::default()
    };
    // On Wayland, eframe only draws when the compositor asks for a frame, so
    // also waiting for vsync when presenting adds nothing, and is a known way
    // to hang with some drivers (NVIDIA).
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        options.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
    }
    eframe::run_native(
        "LitAnatomy",
        options,
        Box::new(|cc| Ok(Box::new(app::LitAnatomyApp::new(cc, library)))),
    )
}
