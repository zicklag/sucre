use std::marker::PhantomData;

use crate::prelude::*;

pub struct Widget<F: StatefulWidget<S>, S: Sync + Send + Default + Clone + 'static> {
    f: F,
    _phantom: PhantomData<S>,
}

impl<F: FnOnce(&mut egui::Ui, &mut S), S: Sync + Send + Default + Clone + 'static> Widget<F, S> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            _phantom: PhantomData,
        }
    }
}

pub trait StatefulWidget<State: Clone + Default + Sync + Send + 'static> {
    fn render(self, ui: &mut egui::Ui, state: &mut State) -> egui::Response;
}

impl<S: Sync + Send + Default + Clone + 'static, F> StatefulWidget<S> for F
where
    F: FnOnce(&mut egui::Ui, &mut S),
{
    fn render(self, ui: &mut egui::Ui, state: &mut S) -> egui::Response {
        ui.scope(|ui| self(ui, state)).response
    }
}

impl<S: Sync + Send + Default + Clone + 'static, T: StatefulWidget<S>> egui::Widget
    for Widget<T, S>
{
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let mut state = ui.data_mut(|map| map.get_temp_mut_or_default::<S>(ui.id()).clone());

        let response = <T as StatefulWidget<S>>::render(self.f, ui, &mut state);

        ui.data_mut(|map| {
            map.insert_temp(ui.id(), state);
        });
        response
    }
}
