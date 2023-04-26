//! A runtime for symmetric interaction combinators.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

use bit_field::BitField;
use crossbeam_channel::unbounded;
use rayon::{ThreadPool, ThreadPoolBuilder};

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

impl Graph {
    /// Initialize a new graph.
    pub fn new(memory_size: usize, threads: usize) -> Self {
        Graph {
            nodes: Nodes::new(memory_size, threads),
            edges: Edges::new(),
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
            graph: Graph::new(memory_size, thread_count),
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
            graph: Graph::new(memory_size, thread_count),
            threadpool,
            active_pairs: Vec::new(),
        }
    }

    /// Use the runtime to reduce the provided graph to it's normal form.
    pub fn reduce(&mut self) {
        use rayon::prelude::*;
        let thread_count = self.threadpool.current_num_threads();

        loop {
            // Add all the active pairs to the buffer
            self.active_pairs.clear();
            self.active_pairs
                .extend(self.graph.edges.active_pairs().map(|(a, b)| ActivePair {
                    a,
                    b,
                    a_kind: 0.into(),
                    b_kind: 0.into(),
                }));

            // If there are no active pairs
            if self.active_pairs.is_empty() {
                // We've reached normal form, we're done.
                break;
            }

            // Use our threadpool for all rayon operations ( such as par_chunks_mut, etc. )
            self.threadpool.install(|| {
                // Lock the nodes
                let mut mmap = self.graph.nodes.lock();
                let mem_size = mmap.len();

                // Iterate over the chunks of nodes in parallel, applying the first pass, to resolve
                // the active nodes kinds.
                mmap.par_chunks(CHUNK_SIZE)
                    .enumerate()
                    .for_each(|(chunk_id, nodes)| {
                        // Calculate chunk position
                        let chunk_size = nodes.len();
                        let chunk_node_idx_start = chunk_id * chunk_size * NODES_PER_BYTE;
                        let chunk_node_idx_end = chunk_node_idx_start + chunk_size * NODES_PER_BYTE;

                        // For each active pair
                        for active_pair in &self.active_pairs {
                            // For both node a and b in the pair
                            for (node, node_kind) in [
                                (active_pair.a, &active_pair.a_kind),
                                (active_pair.b, &active_pair.b_kind),
                            ] {
                                // If the node is within this chunk
                                if node >= chunk_node_idx_start as u64
                                    && node < chunk_node_idx_end as u64
                                {
                                    // Get it's position in the chunk
                                    let offset = active_pair.a as usize % NODES_PER_BYTE;
                                    let rounded = active_pair.a as usize - offset;
                                    let node_byte_idx = rounded - chunk_node_idx_start;

                                    let bit_start = offset * 2;
                                    let bit_end = bit_start + 2;
                                    let kind = nodes[node_byte_idx].get_bits(bit_start..bit_end);

                                    // And store that in the active pairs list
                                    node_kind.store(kind, Relaxed);
                                }
                            }
                        }
                    });

                struct PendingAllocation {
                    kind: NodeKind,
                    edge_mutation: EdgeMutation,
                }

                // Create channel for sending pending allocations
                let (pending_allocation_sender, pending_allocation_receiver) =
                    unbounded::<PendingAllocation>();

                // Create channel for sending edge mutations
                let (edge_mutation_sender, edge_mutation_receiver) = unbounded::<EdgeMutation>();

                // Iterate over the chunks of nodes in parallel, applying the second pass, to apply
                // annihilations, duplications, and erases.
                mmap.par_chunks_mut(mem_size / thread_count)
                    .enumerate()
                    .for_each(|(chunk_id, nodes)| {
                        // Calculate chunk position
                        let chunk_size = nodes.len();
                        let chunk_node_idx_start = chunk_id * chunk_size * NODES_PER_BYTE;
                        let chunk_node_idx_end = chunk_node_idx_start + chunk_size * NODES_PER_BYTE;

                        // For each active pair
                        for active_pair in &self.active_pairs {
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
                                if node >= chunk_node_idx_start as u64
                                    && node < chunk_node_idx_end as u64
                                {
                                    // Get it's position in the chunk
                                    let offset = active_pair.a as usize % NODES_PER_BYTE;
                                    let rounded = active_pair.a as usize - offset;
                                    let node_byte_idx = rounded - chunk_node_idx_start;
                                    let bit_start = offset * 2;
                                    let bit_end = bit_start + 2;
                                    let bits = bit_start..bit_end;

                                    // Perform any operations required based on the kind of nodes in the pair.
                                    match (node_kind, other_node_kind) {
                                        //
                                        // Annihilation rule
                                        //
                                        (NodeKind::Duplicator, NodeKind::Duplicator)
                                        | (NodeKind::Constructor, NodeKind::Constructor) => {}

                                        //
                                        // Duplication rule
                                        //
                                        (NodeKind::Constructor, NodeKind::Duplicator)
                                        | (NodeKind::Duplicator, NodeKind::Constructor) => {}

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
                                            nodes[node_byte_idx]
                                                .set_bits(bits, NodeKind::Eraser as u8);

                                            // Connect whatever node was connected to our port 2, to
                                            // our new port 0.
                                            edge_mutation_sender
                                                .send(EdgeMutation::Bridge {
                                                    a: node,
                                                    a_port: 0,
                                                    b: node,
                                                    b_port: 2,
                                                })
                                                .unwrap();
                                        }
                                        (
                                            NodeKind::Eraser,
                                            NodeKind::Duplicator | NodeKind::Constructor,
                                        ) => {
                                            // This node will stay an eraser, and we'll connect it
                                            // the node connected to the duplicator/constructor's port 1.
                                            edge_mutation_sender
                                                .send(EdgeMutation::Bridge {
                                                    a: node,
                                                    a_port: 0,
                                                    b: other_node,
                                                    b_port: 1,
                                                })
                                                .unwrap();
                                        }

                                        //
                                        // Erasers just delete each-other completely
                                        //
                                        (NodeKind::Eraser, NodeKind::Eraser) => {
                                            // Delete this node
                                            nodes[node_byte_idx].set_bits(bits, 0);

                                            // The lower node is in charge of deleting the edge
                                            if node < other_node {
                                                edge_mutation_sender
                                                    .send(EdgeMutation::RemoveEdge(Edge {
                                                        a: node,
                                                        b: other_node,
                                                        a_port: 0,
                                                        b_port: 0,
                                                    }))
                                                    .unwrap();
                                            }
                                        }

                                        //
                                        // Null nodes should not be active
                                        //
                                        (NodeKind::Null, _) | (_, NodeKind::Null) => {
                                            panic!("Null nodes should not be active")
                                        }
                                    }
                                }
                            }
                        }
                    });
            });
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
