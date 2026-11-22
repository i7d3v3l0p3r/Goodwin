#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod discovery;
mod package;
mod station;
mod window;

use eframe::egui;

fn main() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../icons/goodwin.png"))
        .expect("the embedded Goodwin icon is a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(icon)
            .with_inner_size([580.0, 470.0])
            .with_min_inner_size([460.0, 370.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Установщик Goodwin",
        options,
        Box::new(|creation| {
            window::configure(&creation.egui_ctx);
            Ok(Box::new(window::InstallerWindow::new(&creation.egui_ctx)))
        }),
    )
}
