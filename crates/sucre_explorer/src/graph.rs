use std::sync::{Arc, Mutex};

use eframe::epaint::{ahash::HashMap, text::LayoutJob};
use rand::Rng;
use sucre::{
    prelude::{Edge, NodeKind},
    Graph, NodeId, Runtime, Uint,
};

use crate::prelude::*;

#[derive(Clone)]
pub struct State {
    graph_code: String,
    runtime: Option<Arc<Mutex<Runtime>>>,
    graph_display_simulation: SimulationState,
    graph_display_zoom: f32,
    graph_display_pan: egui::Vec2,
}

#[derive(Default, Clone)]
struct SimulationState {
    node_positions: HashMap<NodeId, egui::Vec2>,
    edge_positions: HashMap<Edge, egui::Vec2>,
    node_velocities: HashMap<NodeId, egui::Vec2>,
    edge_velocities: HashMap<Edge, egui::Vec2>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForceEdge {
    a_port: Uint,
    b_port: Uint,
}

impl Default for State {
    fn default() -> Self {
        Self {
            graph_code: "\
const a
const b
const c
dup d
const e
root f
---
a1-a2
a0-b1
b0-c0
c1-d0
c2-e2
d1-e1
e0-d2
b2-f0"
                .into(),
            runtime: None,
            graph_display_simulation: Default::default(),
            graph_display_zoom: 1.0,
            graph_display_pan: egui::vec2(0., 0.),
        }
    }
}

pub fn ui_tab(ui: &mut egui::Ui, state: &mut State) {
    egui::SidePanel::right("sidebar").show(ui.ctx(), |ui| {
        ui.heading("Make Graph");
        ui.separator();
        ui.add_space(2.);
        ui.label("Graph expression:");
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut layouter = |ui: &egui::Ui, string: &str, wrap_width: f32| {
                let mut layout_job = LayoutJob::simple(
                    string.into(),
                    egui::FontId::monospace(12.),
                    egui::Color32::WHITE,
                    wrap_width,
                );
                layout_job.wrap.max_width = wrap_width;
                ui.fonts(|f| f.layout_job(layout_job))
            };

            egui::TextEdit::multiline(&mut state.graph_code)
                .font(egui::TextStyle::Monospace) // for cursor height
                .code_editor()
                .lock_focus(true)
                .desired_rows(10)
                .desired_width(f32::INFINITY)
                .layouter(&mut layouter)
                .show(ui);

            ui.add_space(2.);

            ui.add_space(2.);
            let ast = graph_parser::graph(&state.graph_code);

            if let Err(e) = &ast {
                ui.label(egui::RichText::new(e.to_string()).color(egui::Color32::RED));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.set_enabled(ast.is_ok());
                if ui.button("Make Graph").clicked() {
                    if let Ok(ast) = ast {
                        let mem_size = 20;
                        let mut runtime = Runtime::new(mem_size, 4);
                        runtime.graph = ast.into_graph(mem_size);
                        state.graph_display_simulation = Default::default();
                        state.runtime = Some(Arc::new(Mutex::new(runtime)));
                    }
                }
            });
        });

        ui.add_space(8.);
        ui.separator();
        ui.heading("Reduce Graph");
        ui.separator();

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.set_enabled(state.runtime.is_some());
            ui.horizontal(|ui| {
                if ui
                    .button("Reduce")
                    .on_hover_text("Reduce the graph to normal form")
                    .clicked()
                {
                    if let Some(r) = &state.runtime {
                        let mut r = r.lock().unwrap();
                        r.reduce();
                    }
                }

                if ui
                    .button("Step")
                    .on_hover_text("Reduce the graph by one step")
                    .clicked()
                {
                    if let Some(r) = &state.runtime {
                        let mut r = r.lock().unwrap();
                        r.reduce_steps(1);
                    }
                }
            });
        });
    });

    egui::CentralPanel::default()
        .frame(egui::Frame::canvas(ui.style()))
        .show(ui.ctx(), |ui| {
            let (rect, response) =
                ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
            let painter = ui.painter_at(rect);

            ui.ctx().request_repaint();
            if let Some(runtime) = &state.runtime {
                let runtime = runtime.lock().unwrap();
                let g = &runtime.graph;
                let radius = 20.;

                painter.text(
                    egui::pos2(20., 40.),
                    egui::Align2::LEFT_TOP,
                    format!("{:#?}", runtime.graph),
                    egui::FontId::monospace(10.),
                    egui::Color32::WHITE,
                );

                let sim = &mut state.graph_display_simulation;
                sim.update(g, radius, ui.input(|i| i.stable_dt));

                // Update the pan and zoom
                state.graph_display_zoom += ui.input(|i| i.scroll_delta.y * 0.003);
                state.graph_display_pan += response.drag_delta();

                // Render the graph
                let z = state.graph_display_zoom;
                let p = state.graph_display_pan;

                let world_to_screen =
                    |pos: egui::Vec2| -> egui::Pos2 { rect.center() + pos * z + p };

                for (node_id, node_kind) in
                    g.nodes.iter().enumerate().filter(|x| x.1 != NodeKind::Null)
                {
                    let node_id = node_id as NodeId;
                    let pos = sim
                        .node_positions
                        .get(&node_id)
                        .copied()
                        .unwrap_or_default();

                    painter.add(epaint::PathShape::convex_polygon(
                        vec![
                            world_to_screen(pos + egui::vec2(0.0, -radius)),
                            world_to_screen(pos + egui::vec2(radius, radius)),
                            world_to_screen(pos + egui::vec2(-radius, radius)),
                        ],
                        match node_kind {
                            NodeKind::Null => unreachable!(),
                            NodeKind::Constructor => egui::Color32::DARK_GREEN,
                            NodeKind::Duplicator => egui::Color32::DARK_BLUE,
                            NodeKind::Eraser => egui::Color32::DARK_RED,
                            NodeKind::Root => egui::Color32::from_rgb(150, 50, 50),
                        },
                        (0.0, egui::Color32::TRANSPARENT),
                    ));
                    painter.text(
                        world_to_screen(pos),
                        egui::Align2::CENTER_CENTER,
                        format!(
                            "{}{}",
                            match node_kind {
                                NodeKind::Null => "N",
                                NodeKind::Constructor => "γ",
                                NodeKind::Duplicator => "δ",
                                NodeKind::Eraser => "ε",
                                NodeKind::Root => "R",
                            },
                            node_id
                        ),
                        egui::FontId::proportional((radius * z).max(1.)),
                        egui::Color32::WHITE,
                    );
                }

                for edge in g.edges.iter() {
                    let (start, end) = (
                        sim.node_positions.get(&edge.a).copied().unwrap_or_default(),
                        sim.node_positions.get(&edge.b).copied().unwrap_or_default(),
                    );
                    let middle = sim.edge_positions.get(&edge).copied().unwrap_or_default();
                    let middle = world_to_screen(middle);
                    let (mut start, mut end) = (world_to_screen(start), world_to_screen(end));

                    start += match edge.a_port {
                        0 => egui::vec2(0.0, -radius) * z,
                        1 => egui::vec2(-radius, radius) * z,
                        2 => egui::vec2(radius, radius) * z,
                        _ => unreachable!(),
                    };
                    end += match edge.b_port {
                        0 => egui::vec2(0.0, -radius) * z,
                        1 => egui::vec2(-radius, radius) * z,
                        2 => egui::vec2(radius, radius) * z,
                        _ => unreachable!(),
                    };

                    let color = match (edge.a_port, edge.b_port) {
                        (0, 0) => egui::Color32::RED,
                        _ => egui::Color32::GRAY,
                    };
                    painter.add(epaint::QuadraticBezierShape {
                        points: [start, middle, end],
                        closed: false,
                        fill: egui::Color32::TRANSPARENT,
                        stroke: (2.0, color).into(),
                    });
                }
            }
        });
}

peg::parser! {
  grammar graph_parser() for str {
    rule _ = (" " / "\t")*
    rule __ = (" " / "\t" / "\n")*

    rule name() = ['a'..='z' | 'A'..='Z']
    rule port_number() -> u8 =
        "0" { 0 } / "1" { 1 } / "2" { 2 }
    rule node_kind() -> NodeKind =
        "root" { NodeKind::Root } /
        "const" { NodeKind::Constructor } /
        "dup" { NodeKind::Duplicator }
    rule node() -> (String, NodeKind) =
        kind:node_kind() _ name:$name() { (name.into(), kind) }

    rule nodes() -> Vec<(String, NodeKind)> =
        nodes:node() ** (_ "\n")
        { nodes.into_iter().collect() }

    rule edge() -> ((String, u8), (String, u8)) =
        a:$name() ap:port_number() "-" b:$name() bp:port_number()
        {
            ((a.into(), ap), (b.into(), bp))
        }

    rule edges() -> Vec<((String, u8), (String, u8))> =
        e:edge() ** (_ "\n") __
        { e.into_iter().collect() }

    pub rule graph() -> GraphAst
      = __ nodes:nodes() _ "\n" _ "-"+ _ "\n" _ edges:edges() __ {
        GraphAst {
            nodes,
            edges,
        }
      }
  }
}

#[derive(Debug)]
pub struct GraphAst {
    nodes: Vec<(String, NodeKind)>,
    edges: Vec<((String, u8), (String, u8))>,
}

impl GraphAst {
    pub fn into_graph(self, memory_size: usize) -> Graph {
        let mut graph = Graph::new(memory_size);

        // Allocate all of the nodes
        let ast_nodes = self.nodes.into_iter().collect::<Vec<_>>();
        let node_ids = graph.nodes.allocate(ast_nodes.iter().map(|x| x.1));
        let node_names = ast_nodes
            .into_iter()
            .map(|x| x.0)
            .zip(node_ids.into_iter())
            .collect::<HashMap<_, _>>();

        // Create the edges
        for ((a, ap), (b, bp)) in self.edges {
            graph.edges.insert(Edge {
                a: *node_names.get(&a).unwrap(),
                b: *node_names.get(&b).unwrap(),
                a_port: ap as _,
                b_port: bp as _,
            });
        }

        graph
    }
}

impl SimulationState {
    // Note: This code could probably be better de-duplicated between the node and edge looping sections.
    pub fn update(&mut self, graph: &Graph, radius: f32, dt: f32) {
        const COOLOFF: f32 = 0.98;
        const SCALE: f32 = 30.0;

        // TODO: avoid cloning the entire node vec like this
        let nodes = graph
            .nodes
            .iter()
            .enumerate()
            .filter(|x| x.1 != NodeKind::Null)
            .map(|x| x.0 as NodeId)
            .collect::<Vec<_>>();

        let mut rng = rand::thread_rng();
        for node in nodes.iter().copied() {
            self.node_positions.entry(node).or_insert_with(|| {
                egui::vec2(rng.gen_range(-100.0..=100.0), rng.gen_range(-100.0..=100.0))
            });
        }
        for edge in graph.edges.iter() {
            self.edge_positions.entry(edge).or_insert_with(|| {
                egui::vec2(rng.gen_range(-100.0..=100.0), rng.gen_range(-100.0..=100.0))
            });
        }

        let old_state = self.clone();
        for node_id in nodes.iter().copied() {
            let mut force = egui::Vec2::ZERO;
            let pos = old_state.node_positions.get(&node_id).copied().unwrap();

            // Calculate repulsion
            force += {
                let mut f = egui::Vec2::ZERO;

                for other_node in nodes.iter().copied() {
                    if other_node == node_id {
                        continue;
                    }

                    let other_pos = old_state.node_positions.get(&other_node).copied().unwrap();

                    f += -((SCALE * SCALE) / (pos - other_pos).length())
                        * (other_pos - pos).normalized()
                }

                for edge in graph.edges.iter() {
                    let edge_pos = old_state.edge_positions.get(&edge).copied().unwrap();

                    f += -((SCALE * SCALE) / (pos - edge_pos).length())
                        * (edge_pos - pos).normalized()
                }

                f
            };

            // Calculate attraction
            force += {
                let mut f = egui::Vec2::ZERO;

                for port in 0..3 {
                    let Some((other_node, other_port)) = graph.edges.get(node_id, port) else {continue;};

                    let edge_pos = old_state
                        .edge_positions
                        .get(
                            &Edge {
                                a: node_id,
                                a_port: port,
                                b: other_node,
                                b_port: other_port,
                            }
                            .canonical(),
                        )
                        .copied()
                        .unwrap_or_default();

                    f += ((pos - edge_pos).length_sq() / SCALE) * (edge_pos - pos).normalized();
                }

                f
            };

            let vel = self.node_velocities.entry(node_id).or_default();
            *vel += force * dt;
            *vel *= COOLOFF;
            self.node_positions.insert(node_id, pos + *vel * dt);
        }

        for edge in graph.edges.iter() {
            let mut force = egui::Vec2::ZERO;
            let pos = old_state.edge_positions.get(&edge).copied().unwrap();

            // Calculate repulsion
            force += {
                let mut f = egui::Vec2::ZERO;

                for other_edge in graph.edges.iter() {
                    if edge == other_edge {
                        continue;
                    };

                    let other_pos = old_state.edge_positions.get(&other_edge).copied().unwrap();

                    f += -((SCALE * SCALE) / (pos - other_pos).length().max(0.1))
                        * (other_pos - pos).normalized()
                }

                for node in nodes.iter() {
                    let node_pos = old_state.node_positions.get(node).copied().unwrap();

                    for i in 0..3 {
                        let offset = match i {
                            0 => egui::vec2(0.0, radius),
                            1 => egui::vec2(-radius, 0.0),
                            2 => egui::vec2(radius, 0.0),
                            _ => unreachable!(),
                        };
                        let node_pos = node_pos + offset;

                        f += -((SCALE * SCALE) / (pos - node_pos).length())
                            * (node_pos - pos).normalized();
                    }
                }

                f
            };

            // Calculate attraction
            force += {
                let mut f = egui::Vec2::ZERO;

                for node in [edge.a, edge.b] {
                    let node_pos = old_state.node_positions.get(&node).copied().unwrap();

                    f += ((pos - node_pos).length_sq() / SCALE) * (node_pos - pos).normalized();
                }

                f
            };

            let vel = self.edge_velocities.entry(edge).or_default();
            *vel += force * dt;
            *vel *= COOLOFF;
            self.edge_positions.insert(edge, pos + *vel * dt);
        }
    }
}
