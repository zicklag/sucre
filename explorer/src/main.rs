use eframe::egui;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "Sucre Explorer",
        native_options,
        Box::new(|cc| Box::new(ExplorerApp::new(cc))),
    )
    .unwrap();
}

mod utils;
mod prelude {
    pub use crate::utils::*;
    pub use eframe::egui;
    pub use egui::Widget as EguiWidget;
}

#[derive(Default)]
struct ExplorerApp {
    tab: ExplorerTab,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ExplorerTab {
    #[default]
    Heap,
}

impl ExplorerApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_pixels_per_point(1.1);
        // Customize egui here with cc.egui_ctx.set_fonts and cc.egui_ctx.set_visuals.
        // Restore app state using cc.storage (requires the "persistence" feature).
        // Use the cc.gl (a glow::Context) to create graphics shaders and buffers that you can use
        // for e.g. egui::PaintCallback.
        Self::default()
    }
}

impl eframe::App for ExplorerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Tools:");
                ui.selectable_value(&mut self.tab, ExplorerTab::Heap, "Heap");
            });
        });
        egui::CentralPanel::default().show(ctx, |_ui| match self.tab {
            ExplorerTab::Heap => todo!(),
        });
    }
}
