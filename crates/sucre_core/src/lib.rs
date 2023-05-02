//! A runtime for symmetric interaction combinators.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

use bit_field::BitField;
use crossbeam_channel::{unbounded as channel, Sender};
use rayon::{prelude::*, ThreadPool, ThreadPoolBuilder};

#[macro_use]
pub(crate) mod utils;

pub mod edges;
pub mod nodes;

use prelude::*;
pub mod prelude {
    //! The prelude.
    pub use crate::{edges::*, nodes::*};
}

/// The unsigned integer type used throughout the codebase.
///
/// This is currently [`u64`], but we use a type alias just in case we want to change it later, or
/// make a 32-bit variant.
pub type Uint = u64;

/// The type used for node identifiers.
pub type NodeId = Uint;

/// An interaction combinator graph.
///
/// This stores chunks of nodes and the edges between the nodes.
#[derive(Clone)]
pub struct Graph {
    /// The chunks of nodes stored in the graph.
    pub nodes: Nodes,
    /// The edges connecting the nodes.
    pub edges: Edges,
}

impl std::fmt::Debug for Graph {
    // TODO: This is only practical for extremely small numbers of nodes and edges
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !f.alternate() {
            f.debug_struct("Graph").finish()
        } else {
            writeln!(f, "Nodes:")?;

            let mut nodes = self.nodes.iter_non_null();

            'nodes: loop {
                write!(f, "    ")?;
                for _ in 0..16 {
                    let Some((node_id, node_kind)) = nodes.next() else { break 'nodes; };
                    write!(
                        f,
                        "{:<3}: {:<8}",
                        node_id,
                        match node_kind {
                            NodeKind::Null => "N",
                            NodeKind::Constructor => "γ",
                            NodeKind::Duplicator => "δ",
                            NodeKind::Eraser => "E",
                            NodeKind::Root => "R",
                            NodeKind::Passthrough => "P",
                        }
                    )?;
                }

                writeln!(f)?;
            }
            writeln!(f)?;

            writeln!(f, "Edges:")?;
            let mut edges = self.edges.iter();
            'edges: loop {
                for _ in 0..16 {
                    let Some(edge) = edges.next() else { break 'edges; };
                    let (lp, rp) = if edge.a_port == 0 && edge.b_port == 0 {
                        ("(", ")")
                    } else {
                        (" ", " ")
                    };
                    write!(
                        f,
                        "{lp}{}:{}→{}:{}{rp} ",
                        edge.a, edge.a_port, edge.b, edge.b_port
                    )?;
                }
                writeln!(f)?;
            }

            write!(f, "")
        }
    }
}

impl Graph {
    /// Initialize a new graph.
    pub fn new(memory_size: usize) -> Self {
        Graph {
            nodes: Nodes::new(memory_size),
            edges: Edges::new(memory_size),
        }
    }
}
/// Represents an active pair used during graph reduction.
///
/// Users will not need to interact with this most-likely.
// TODO(perf): evaluate the most efficient synchronization primitive to use here in place of
// `Event`, if Event isn't already the best option.
pub struct ActivePair {
    /// The first node in the pair.
    pub a: NodeId,
    /// The second node in the pair.
    pub b: NodeId,
    /// The kind of node `a`.
    ///
    /// If this is `0` it means that it hasn't been detected yet.
    pub a_kind: AtomicU8,
    /// The kind of node `b`.
    ///
    /// If this is `0` it means that it hasn't been detected yet.
    pub b_kind: AtomicU8,
}

/// The number of bytes in the chunks that are processed when processing nodes.
const CHUNK_SIZE: usize = 1024;

/// Runtime capable of reducing [`Graph`]s.
pub struct Runtime {
    /// The worker thread pool for the runtime.
    pub threadpool: ThreadPool,
    /// The graph the runtime will operate on.
    pub graph: Graph,
    /// A cache of active pairs that is used to re-use memory neede while running
    /// [`reduce()`][Self::reduce].
    active_pairs: Vec<ActivePair>,
}

impl Runtime {
    /// Create a new runtime with the given number of worker threads.
    pub fn new(memory_size: usize, thread_count: usize) -> Runtime {
        Self {
            threadpool: ThreadPoolBuilder::new()
                .num_threads(thread_count)
                .build()
                .unwrap(),
            graph: Graph::new(memory_size),
            active_pairs: Vec::new(),
        }
    }

    /// Create a new runtime with one thread per CPU core.
    ///
    /// TODO: This is actually one thread per _logical_ CPU core on hyper-threaded machines.
    /// Eventually we will want to spawn one thread per physical CPU core, and pin each thread to
    /// it's core to optimize cache access.
    #[cfg(feature = "thread_per_core")]
    pub fn new_thread_per_core(memory_size: usize) -> Runtime {
        let thread_count = num_cpus::get_physical();
        let threadpool = ThreadPoolBuilder::new()
            .num_threads(thread_count)
            .build()
            .unwrap();
        Self {
            graph: Graph::new(memory_size),
            threadpool,
            active_pairs: Vec::new(),
        }
    }

    /// Reduce the graph only the given number of steps before stopping.
    ///
    /// Returns the number of steps that were run before either reaching `n` steps or reaching
    /// normal form.
    pub fn reduce_steps(&mut self, n: usize) -> usize {
        // Use our threadpool for all rayon operations ( such as par_chunks_mut, etc. )
        let mut steps_run = 0;

        self.threadpool.install(|| {
            while steps_run < n && Self::reduce_one_step(&mut self.graph, &mut self.active_pairs) {
                steps_run += 1;
            }
        });

        steps_run
    }

    /// Reduce the graph by one step. Returns `false` if there is nothing to do.
    fn reduce_one_step(graph: &mut Graph, active_pairs: &mut Vec<ActivePair>) -> bool {
        // Clear the old active pairs list
        active_pairs.clear();

        // Add active pairs to the buffer in parallel
        active_pairs.par_extend(graph.edges.par_active_pairs().map(|(a, b)| ActivePair {
            a,
            b,
            a_kind: 0.into(),
            b_kind: 0.into(),
        }));

        // If there are no active pairs
        if active_pairs.is_empty() {
            // We've reached normal form, we're done.
            return false;
        }

        // Iterate over the chunks of nodes in parallel, applying the first pass, to resolve
        // the active nodes kinds.
        graph
            .nodes
            .mmap
            .par_chunks(CHUNK_SIZE)
            .enumerate()
            .for_each(|(chunk_id, nodes)| {
                // Calculate chunk position
                let chunk_size = nodes.len();
                let chunk_node_idx_start = chunk_id * chunk_size * NODES_PER_BYTE;
                let chunk_node_idx_end = chunk_node_idx_start + chunk_size * NODES_PER_BYTE;

                // For each active pair
                for active_pair in active_pairs.iter() {
                    // For both node a and b in the pair
                    for (node, node_kind) in [
                        (active_pair.a, &active_pair.a_kind),
                        (active_pair.b, &active_pair.b_kind),
                    ] {
                        // If the node is within this chunk
                        if node >= chunk_node_idx_start as u64 && node < chunk_node_idx_end as u64 {
                            // Get it's position in the chunk
                            let offset = node as usize % NODES_PER_BYTE;
                            let rounded = node as usize - offset;
                            let node_byte_idx = (rounded / NODES_PER_BYTE) - chunk_node_idx_start;

                            let bit_start = offset * BITS_PER_NODE;
                            let bit_end = bit_start + BITS_PER_NODE;
                            let kind = nodes[node_byte_idx].get_bits(bit_start..bit_end);

                            // And store that in the active pairs list
                            node_kind.store(kind, Relaxed);
                        }
                    }
                }
            });

        // Create channel for sending pending allocations
        let (pending_allocation_sender, pending_allocation_receiver) =
            channel::<PendingAllocation>();

        // Create channel for sending edge mutations
        let (edge_mutation_sender, edge_mutation_receiver) = channel::<EdgeMutation>();

        // Iterate over the chunks of nodes in parallel, applying the second pass, to apply
        // annihilations, duplications, and erases.
        graph
            .nodes
            .mmap
            .par_chunks_mut(CHUNK_SIZE)
            .enumerate()
            .for_each(|(chunk_id, nodes)| {
                // Calculate chunk position
                let this_chunk_size = nodes.len(); // The last chunk might be smaller than `CHUNK_SIZE`
                let chunk_node_idx_start = chunk_id * CHUNK_SIZE * NODES_PER_BYTE;
                let chunk_node_idx_end = chunk_node_idx_start + this_chunk_size * NODES_PER_BYTE;

                // For each active pair
                for active_pair in active_pairs.iter() {
                    // For both node a and b in the pair
                    for (node, node_kind, other_node, other_node_kind) in [
                        (
                            active_pair.a,
                            NodeKind::from(active_pair.a_kind.load(Relaxed)),
                            active_pair.b,
                            NodeKind::from(active_pair.b_kind.load(Relaxed)),
                        ),
                        (
                            active_pair.b,
                            NodeKind::from(active_pair.b_kind.load(Relaxed)),
                            active_pair.a,
                            NodeKind::from(active_pair.a_kind.load(Relaxed)),
                        ),
                    ] {
                        // If the node is within this chunk
                        if node >= chunk_node_idx_start as u64 && node < chunk_node_idx_end as u64 {
                            // Get it's position in the chunk
                            let offset = node as usize % NODES_PER_BYTE;
                            let rounded = node as usize - offset;
                            let node_byte_idx = rounded / NODES_PER_BYTE - chunk_node_idx_start;
                            let bit_start = offset * BITS_PER_NODE;
                            let bit_end = bit_start + BITS_PER_NODE;
                            let bits = bit_start..bit_end;

                            // Perform any operations required based on the kind of nodes in the pair.
                            match (node_kind, other_node_kind) {
                                //
                                // Annihilation rule
                                //
                                (NodeKind::Duplicator, NodeKind::Duplicator)
                                | (NodeKind::Constructor, NodeKind::Constructor) => {
                                    // Convert the node to a passthrough node
                                    nodes[node_byte_idx].set_bits(bits, NodeKind::Passthrough as _);

                                    // The lower node is responsible for modifying the edges
                                    if node < other_node {
                                        edge_mutation_sender
                                            .send(EdgeMutation::Annihilate {
                                                a: node,
                                                b: other_node,
                                            })
                                            .unwrap();
                                    }
                                }

                                //
                                // Duplication rule
                                //
                                (NodeKind::Constructor, NodeKind::Duplicator)
                                | (NodeKind::Duplicator, NodeKind::Constructor) => {
                                    println!("Duplicate");
                                    // The lower node is repsonsible for modifying the edges appropriately
                                    if node < other_node {
                                        // Switch this node to the kind of the other node
                                        nodes[node_byte_idx].set_bits(bits, other_node_kind as u8);

                                        //     // Connect this node's port 0 to whatever was connected to it's own port 1
                                        //     edge_mutation_sender
                                        //         .send(EdgeMutation::Reconnect(Edge {
                                        //             a: node,
                                        //             b: node,
                                        //             a_port: 0,
                                        //             b_port: 1,
                                        //         }))
                                        //         .unwrap();

                                        //     // Connect the other node's port 0 whatever was connected to it's port 2
                                        //     edge_mutation_sender
                                        //         .send(EdgeMutation::Reconnect(Edge {
                                        //             a: other_node,
                                        //             b: node,
                                        //             a_port: 0,
                                        //             b_port: 2,
                                        //         }))
                                        //         .unwrap();

                                        //     // Allocate the other two nodes
                                        //     pending_allocation_sender
                                        //         .send(PendingAllocation {
                                        //             kind: node_kind,
                                        //             edge_mutations: [
                                        //                 Some(EdgeMutation::Reconnect(Edge {
                                        //                     a: 0, // This will be filled in with the allocated node
                                        //                     b: other_node,
                                        //                     a_port: 0,
                                        //                     b_port: 1,
                                        //                 })),
                                        //                 Some(EdgeMutation::InsertEdge(Edge {
                                        //                     a: 0,
                                        //                     b: node,
                                        //                     a_port: 1,
                                        //                     b_port: 2,
                                        //                 })),
                                        //                 Some(EdgeMutation::InsertEdge(Edge {
                                        //                     a: 0,
                                        //                     b: other_node,
                                        //                     a_port: 2,
                                        //                     b_port: 2,
                                        //                 })),
                                        //             ],
                                        //         })
                                        //         .unwrap();
                                        //     pending_allocation_sender
                                        //         .send(PendingAllocation {
                                        //             kind: node_kind,
                                        //             edge_mutations: [
                                        //                 Some(EdgeMutation::Reconnect(Edge {
                                        //                     a: 0,
                                        //                     b: other_node,
                                        //                     a_port: 0,
                                        //                     b_port: 1,
                                        //                 })),
                                        //                 Some(EdgeMutation::InsertEdge(Edge {
                                        //                     a: 0,
                                        //                     b: other_node,
                                        //                     a_port: 2,
                                        //                     b_port: 1,
                                        //                 })),
                                        //                 Some(EdgeMutation::InsertEdge(Edge {
                                        //                     a: 0,
                                        //                     b: node,
                                        //                     a_port: 1,
                                        //                     b_port: 1,
                                        //                 })),
                                        //             ],
                                        //         })
                                        //         .unwrap();
                                    }
                                }

                                //
                                // Duplicators/constructors connected to erasers
                                //
                                // Turn the duplicator/constructor into an eraser, and
                                // connect it to port 1, then connect the pre-existing
                                // eraser to the duplicator/constructor's port 2.
                                //
                                (
                                    NodeKind::Duplicator | NodeKind::Constructor,
                                    NodeKind::Eraser,
                                ) => {
                                    // Convert this node to an eraser
                                    nodes[node_byte_idx].set_bits(bits, NodeKind::Eraser as u8);

                                    edge_mutation_sender
                                        .send(EdgeMutation::EraseConstructor {
                                            constructor: node,
                                            eraser: other_node,
                                        })
                                        .unwrap();
                                }
                                (
                                    NodeKind::Eraser,
                                    NodeKind::Duplicator | NodeKind::Constructor,
                                ) => {} // Already handled by the rule above

                                //
                                // Erasers just delete each-other completely
                                //
                                (NodeKind::Eraser, NodeKind::Eraser) => {
                                    // Delete this node
                                    nodes[node_byte_idx].set_bits(bits, 0);

                                    // The lower node is in charge of deleting the edge
                                    if node < other_node {
                                        edge_mutation_sender
                                            .send(EdgeMutation::EraseEdge(Edge {
                                                a: node,
                                                b: other_node,
                                                a_port: 0,
                                                b_port: 0,
                                            }))
                                            .unwrap();
                                    }
                                }

                                //
                                // Root nodes only have one port, but since a root node
                                // never _interacts_ with other nodes, it's only port isn't
                                // _active_. Port zero is always considered an active port,
                                // therefore anything connected to a root node must do so on
                                // port 1.
                                //
                                // If a root node becomes active, then, it means we
                                // connected something to a root node's port 0 which is an
                                // error.
                                //
                                (NodeKind::Root, _) | (_, NodeKind::Root) => {
                                    panic!(
                                        "It is invalid to connect something to a root \
                                                node's port 0. Connect it to port 1 instead."
                                    )
                                }

                                // Null nodes should not be active
                                (NodeKind::Null, _) | (_, NodeKind::Null) => {
                                    unreachable!("Null nodes should not be active");
                                }

                                // Passthrough nodes should never be active
                                (NodeKind::Passthrough, _) | (_, NodeKind::Passthrough) => {
                                    unreachable!("Passthrough nodes should not be active");
                                }
                            }
                        }
                    }
                }

                // Check for any pending allocations from other chunks
                let nodes = nodes.iter_mut();
                handle_pending_allocations(
                    nodes,
                    chunk_node_idx_start,
                    pending_allocation_receiver.try_iter(),
                    &pending_allocation_sender,
                    // &edge_mutation_sender,
                )
            });

        // Handle any remaining allocations that weren't finished during the chunk evaluation.
        // Check for any pending allocations from other chunks
        let nodes = graph.nodes.mmap.iter_mut();
        handle_pending_allocations(
            nodes,
            0,
            pending_allocation_receiver.try_iter(),
            &pending_allocation_sender,
            // &edge_mutation_sender,
        );

        // If we still have pending allocations
        if !pending_allocation_receiver.is_empty() {
            // All of our nodes are allocated and we don't have room. In the future we
            // should allocate another page of memory to store the nodes in.
            todo!("Expand node memory: ran out of room.");
        }

        // Apply all edge mutations
        // TODO(perf): do this in parallel while the other threads are processing chunks if
        // possible and benchmarks indicate that it improves perf.
        graph
            .edges
            .apply_mutations(edge_mutation_receiver.try_iter());

        true
    }

    /// Use the runtime to reduce the provided graph to it's normal form.
    ///
    /// Returns the number of steps that were run before reaching normal form.
    pub fn reduce(&mut self) -> usize {
        self.reduce_steps(usize::MAX)
    }
}

/// Helper struct for allocations that need to be deferred.
struct PendingAllocation {
    kind: NodeKind,
    // edge_mutations: [Option<EdgeMutation>; 3],
}

/// Helper to attempt to allocate any pending allocations into the given iterator of nodes.
fn handle_pending_allocations<'a, N, A>(
    nodes: N,
    node_start_idx: usize,
    pending_allocations: A,
    pending_allocation_sender: &Sender<PendingAllocation>,
    // edge_mutation_sender: &Sender<EdgeMutation>,
) where
    N: IntoIterator<Item = &'a mut u8>,
    A: IntoIterator<Item = PendingAllocation>,
{
    let mut nodes = nodes.into_iter().enumerate();
    'allocation: for pending_allocation in pending_allocations.into_iter() {
        // Get the next byte in our chunk
        let Some((node_byte_idx, node_byte)) = nodes.next() else {
            // We're out of nodes, and can't fulfill the allocation, so send it
            // back to the pending channel.
            pending_allocation_sender.send(pending_allocation).unwrap();
            break;
        };

        // For every node in the byte
        for i in 0..NODES_PER_BYTE {
            let bits_per_node = 8 / NODES_PER_BYTE;
            let bits_start = i * bits_per_node;
            let bits = bits_start..(bits_start + bits_per_node);

            // If this node is empty
            if node_byte.get_bits(bits.clone()) == NodeKind::Null as u8 {
                // Make the allocation
                node_byte.set_bits(bits, pending_allocation.kind as u8);

                // Get the newly alllocated node's id
                let node_id = (node_start_idx + node_byte_idx * NODES_PER_BYTE + i) as u64;

                // // For every edge mutation for this allocation
                // for mutation in pending_allocation.edge_mutations {
                //     let Some(mut mutation) = mutation else { continue; };

                //     // Replace the `a` node in the mutation with the allocated node's id
                //     match &mut mutation {
                //         EdgeMutation::Reconnect(edge)
                //         | EdgeMutation::Bridge(edge)
                //         | EdgeMutation::InsertEdge(edge)
                //         | EdgeMutation::RemoveEdge(edge) => {
                //             edge.a = node_id;
                //         }
                //     }

                //     // Queue the mutation
                //     edge_mutation_sender.send(mutation).unwrap();
                // }

                break 'allocation;
            }
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        // 500MiB memory
        let memory_size = 1024 * 1024 * 500;

        #[cfg(feature = "thread_per_core")]
        return Runtime::new_thread_per_core(memory_size);
        #[cfg(not(feature = "thread_per_core"))]
        return Runtime::new(memory_size, 1);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // #[test]
    // fn reduce_x_dot_xx_app_x_dot_x() {
    //     let mut runtime = Runtime::new_thread_per_core(10);

    //     // Create some nodes
    //     let nodes = runtime.graph.nodes.allocate([
    //         NodeKind::Constructor,
    //         NodeKind::Constructor,
    //         NodeKind::Constructor,
    //         NodeKind::Duplicator,
    //         NodeKind::Constructor,
    //         NodeKind::Root,
    //     ]);
    //     let [a, b, c, d, e, f] = std::array::from_fn(|i| nodes[i]);

    //     // Connect them in the form of the interaction net for `(λx.xx)(λx.x)`
    //     [
    //         (a, 1, a, 2),
    //         (a, 0, b, 1),
    //         (b, 0, c, 0),
    //         (c, 1, d, 0),
    //         (c, 2, e, 2),
    //         (d, 1, e, 1),
    //         (e, 0, d, 2),
    //         (b, 2, f, 0),
    //     ]
    //     .into_iter()
    //     .for_each(|(a, ap, b, bp)| {
    //         // runtime.graph.edges.insert(Edge {
    //         //     a,
    //         //     b,
    //         //     a_port: ap,
    //         //     b_port: bp,
    //         // });
    //     });

    //     println!("{:#?}\n\n", runtime.graph);

    //     // Reduce the graph
    //     while runtime.reduce_steps(1) {
    //         println!("{:#?}\n\n", runtime.graph);
    //     }

    //     panic!();
    // }
}
