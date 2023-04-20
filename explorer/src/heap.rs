use crate::prelude::*;

use sucre::heap::Heap;

#[derive(Clone)]
pub struct State {
    heap: Option<Heap>,
    create_thread_count: usize,
    create_page_size: usize,
    create_is_valid: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            heap: Default::default(),
            create_thread_count: 4,
            create_page_size: 32,
            create_is_valid: false,
        }
    }
}

pub fn ui(ui: &mut egui::Ui, state: &mut State) {
    state.create_is_valid = state.create_page_size > state.create_thread_count
        && state.create_page_size % state.create_thread_count == 0;

    ui.add_space(2.);
    ui.horizontal(|ui| {
        ui.heading("Heap explorer");
        if state.heap.is_none() {
            ui.colored_label(egui::Color32::RED, "No Heap");
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if state.heap.is_none() {
                ui.scope(|ui| {
                    ui.set_enabled(state.create_is_valid);
                    if ui.button("➕ Create Heap").clicked() {
                        state.heap =
                            Some(Heap::new(state.create_page_size, state.create_thread_count));
                    }
                });
                ui.add_space(ui.spacing().item_spacing.y * 3.);
            }

            if let Some((page_size, thread_count, page_count)) = state
                .heap
                .as_ref()
                .map(|heap| (heap.page_size(), heap.thread_count(), heap.page_count()))
            {
                if ui.button("🗑 Delete").clicked() {
                    state.heap = None;
                }

                ui.add_space(ui.spacing().icon_width);

                ui.label(format!("{thread_count} Threads"));

                ui.add_space(ui.spacing().icon_width);

                ui.label(format!("{page_size} Bytes Per Page"));

                ui.add_space(ui.spacing().icon_width);

                ui.label(format!("{page_count} Allocated Memory Pages"));
            } else {
                ui.label("Thread Count");
                egui::DragValue::new(&mut state.create_thread_count)
                    .suffix(" Threads")
                    .clamp_range(1..=(usize::MAX))
                    .ui(ui);

                ui.add_space(ui.spacing().icon_width);

                ui.label("Page Size");
                egui::DragValue::new(&mut state.create_page_size)
                    .suffix(" Bytes")
                    .ui(ui);
            }
        });
    });
    ui.separator();

    if let Some(heap) = &mut state.heap {
        
    }
}
