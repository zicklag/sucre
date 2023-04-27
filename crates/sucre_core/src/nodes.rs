//! Contains the [`Nodes`] store.

use std::sync::Arc;

use async_mutex::{Mutex, MutexGuardArc};
use bit_field::BitField;
use memmap2::{Advice, MmapMut};

use crate::NodeId;

/// Stores the interaction combinator nodes.
pub struct Nodes {
    mmap: Arc<Mutex<MmapMut>>,
    threads: usize,
}

impl Clone for Nodes {
    fn clone(&self) -> Self {
        let mmap = self
            .mmap
            .try_lock()
            .expect("Cannot clone `Chunks` while it is locked");
        let mut new_mmap = MmapMut::map_anon(mmap.len()).expect("Could not map memory");
        new_mmap.advise(Advice::Sequential).ok();
        new_mmap.copy_from_slice(&mmap);

        Self {
            mmap: Arc::new(Mutex::new(new_mmap)),
            threads: self.threads,
        }
    }
}

impl Nodes {
    /// Create a new node storage with the given `memory_size` and thread count.
    ///
    /// The `memory_size` is in bytes and will be rounded up to increments of the thread count.
    ///
    /// For now, the `memory_size` is a hard limit and if the graph grows beyond the `memory_size`
    /// during reduction it will panic.
    ///
    /// This will be improved in the future to allocate new chunks of memory when the existing
    /// memory is exhausted.
    ///
    /// TODO(perf): We need to do more careful investigation into the potential architecture of the
    /// caches and how it may effect our iteration strategy.
    pub fn new(memory_size: usize, threads: usize) -> Self {
        // Round the size up to an increment of L2 cache.
        let memory_size = memory_size + (threads - (memory_size % threads));

        let mmap = MmapMut::map_anon(memory_size).expect("Could not map memory");
        mmap.advise(Advice::Sequential).ok();
        Nodes {
            mmap: Arc::new(Mutex::new(mmap)),
            threads,
        }
    }

    /// Allocate a the given iterator of nodes, and get their node IDs.
    #[track_caller]
    pub fn allocate<N>(&self, nodes: N) -> Vec<NodeId>
    where
        N: IntoIterator<Item = NodeKind>,
    {
        let nodes_to_add = nodes.into_iter();
        let mut node_ids = Vec::with_capacity(nodes_to_add.size_hint().0);
        let mut mmap = self.mmap.try_lock().expect("Can't add nodes while locked");
        let mut node_bytes = mmap.iter_mut().enumerate();

        let mut offset = 0;
        let (mut byte_idx, mut node_byte) = node_bytes.next().expect("Out of memory node memory");
        'nodes: for node_kind in nodes_to_add {
            loop {
                if offset >= 4 {
                    (byte_idx, node_byte) = node_bytes.next().expect("Out of node memory");
                    offset = 0;
                }
                let bit_start = offset * 2;
                let bits = bit_start..(bit_start + 2);

                if node_byte.get_bits(bits.clone()) == NodeKind::Null as u8 {
                    node_byte.set_bits(bits, node_kind as u8);
                    let node_id = (byte_idx * NODES_PER_BYTE + offset) as NodeId;
                    node_ids.push(node_id);
                    offset += 1;
                    continue 'nodes;
                } else {
                    offset += 1;
                }
            }
        }

        node_ids
    }

    /// Get a chunk of the memory, one for each thread, the given number of threads.
    #[track_caller]
    pub fn lock(&self) -> MutexGuardArc<MmapMut> {
        self.mmap
            .try_lock_arc()
            .expect("Cannot lock `Nodes`: already locked.")
    }
}
/// The number of nodes stored in one byte.
pub const NODES_PER_BYTE: usize = 4;

/// The kind of node that a node in the graph is.
#[repr(u8)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum NodeKind {
    /// A node is not actually present here.
    Null = 0,
    /// The node is a constructor node.
    Constructor = 1,
    /// The node is a duplicator node.
    Duplicator = 2,
    /// The node is an eraser node.
    Eraser = 3,
}

impl From<u8> for NodeKind {
    #[inline(always)]
    fn from(value: u8) -> Self {
        use self::NodeKind::*;
        match value {
            0 => Null,
            1 => Constructor,
            2 => Duplicator,
            3 => Eraser,
            _ => panic!("Invalid node type"),
        }
    }
}

impl From<NodeKind> for u8 {
    #[inline(always)]
    fn from(value: NodeKind) -> Self {
        value as u8
    }
}
