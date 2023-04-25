//! A runtime for symmetric interaction combinators.

#![warn(missing_docs)]

use std::sync::atomic::AtomicU8;

use event_listener::Event;
use roaring::RoaringTreemap;

#[macro_use]
pub(crate) mod utils;

pub mod edges;
pub mod nodes;

use prelude::*;
use threadpool::ThreadPool;
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

/// Runtime capable of reducing [`Graph`]s.
pub struct Runtime {
    /// The worker thread pool for the runtime.
    pub threadpool: ThreadPool,
    /// The graph the runtime will operate on.
    pub graph: Graph,
}

impl Runtime {
    /// Create a new runtime with the given number of worker threads.
    pub fn new(memory_size: usize, threads: usize) -> Runtime {
        Self {
            threadpool: ThreadPool::new(threads),
            graph: Graph::new(memory_size, threads),
        }
    }

    /// Create a new runtime with one thread per CPU core.
    ///
    /// TODO: This is actually one thread per _logical_ CPU core on hyper-threaded machines.
    /// Eventually we will want to spawn one thread per physical CPU core, and pin each thread to
    /// it's core to optimize cache access.
    pub fn new_thread_per_core(memory_size: usize) -> Runtime {
        let threadpool = ThreadPool::default();
        Self {
            graph: Graph::new(memory_size, threadpool.max_count()),
            threadpool,
        }
    }

    /// Use the runtime to reduce the provided graph to it's normal form.
    pub fn reduce(&mut self) {
        /// Represents an active pair used during graph reduction.
        // TODO(perf): evaluate the most efficient synchronization primitive to use here in place of
        // `Event`.
        struct ActivePair {
            pub a: NodeId,
            pub b: NodeId,
            pub a_kind: (AtomicU8, Event),
            pub b_kind: (AtomicU8, Event),
        }

        // Initialize active pairs
        let mut active_pairs = Vec::new();

        loop {
            // Add all the active pairs to the buffer
            active_pairs.clear();
            active_pairs.extend(self.graph.edges.active_pairs().map(|(a, b)| ActivePair {
                a,
                b,
                a_kind: (0.into(), Event::new()),
                b_kind: (0.into(), Event::new()),
            }));

            // If there are no active pairs
            if active_pairs.is_empty() {
                // We've reached normal form, we're done.
                break;
            }

            // Sort active pairs by the first node
            active_pairs.sort_by_key(|x| x.a);
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        //                           500MB memory
        Runtime::new_thread_per_core(1024 * 1024 * 500)
    }
}
