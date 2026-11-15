use terra_gui::App;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([980.0, 620.0])
            .with_title("terra-forge"),
        ..Default::default()
    };
    eframe::run_native(
        "terra-forge",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}

use eframe::egui;
