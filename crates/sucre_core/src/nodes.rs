//! Contains the [`Nodes`] store.

use std::sync::Arc;

use async_mutex::{Mutex, MutexGuardArc};
use bit_field::BitField;
use memmap2::{Advice, MmapMut};

use crate::NodeId;

/// Stores the interaction combinator nodes.
pub struct Nodes {
    mmap: Arc<Mutex<MmapMut>>,
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
        }
    }
}

impl Nodes {
    /// Create a new node storage with the given `memory_size` and thread count.
    ///
    /// For now, the `memory_size` is a hard limit and if the graph grows beyond the `memory_size`
    /// during reduction it will panic.
    ///
    /// This will be improved in the future to allocate new chunks of memory when the existing
    /// memory is exhausted.
    ///
    /// TODO(perf): We need to do more careful investigation into the potential architecture of the
    /// caches and how it may effect our iteration strategy.
    pub fn new(memory_size: usize) -> Self {
        let mmap = MmapMut::map_anon(memory_size).expect("Could not map memory");
        mmap.advise(Advice::Sequential).ok();
        Nodes {
            mmap: Arc::new(Mutex::new(mmap)),
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
                if offset >= NODES_PER_BYTE {
                    (byte_idx, node_byte) = node_bytes.next().expect("Out of node memory");
                    offset = 0;
                }
                let bit_start = offset * BITS_PER_NODE;
                let bits = bit_start..(bit_start + BITS_PER_NODE);

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

    /// Get the kind of the node with the given ID.
    pub fn get(&self, node_id: NodeId) -> NodeKind {
        let node_id: usize = node_id.try_into().unwrap();
        let mmap = self.mmap.try_lock().unwrap();
        let byte_idx = node_id / NODES_PER_BYTE;
        let offset = node_id % NODES_PER_BYTE;
        let bit_start = offset * BITS_PER_NODE;
        let bits = bit_start..(bit_start + BITS_PER_NODE);
        NodeKind::from(mmap[byte_idx].get_bits(bits))
    }

    /// Get a chunk of the memory, one for each thread, the given number of threads.
    #[track_caller]
    pub fn lock(&self) -> MutexGuardArc<MmapMut> {
        self.mmap
            .try_lock_arc()
            .expect("Cannot lock `Nodes`: already locked.")
    }

    /// Iterate over the nodes in memory.
    ///
    /// **Note:** This will iterate over the _entire_ memory, returning a [`NodeKind::Null`] for all
    /// null nodes, without skipping them.
    pub fn iter(&self) -> NodeIter {
        NodeIter {
            mmap: self.lock(),
            node_idx: 0,
        }
    }

    /// Get an iterator over the non-null nodes, and their IDs.
    pub fn iter_non_null(&self) -> NonNullIter {
        NonNullIter {
            iter: self.iter(),
            id: 0,
        }
    }
}

/// An iterator over non-null [`Nodes`].
pub struct NonNullIter {
    iter: NodeIter,
    id: usize,
}

impl Iterator for NonNullIter {
    type Item = (NodeId, NodeKind);

    fn next(&mut self) -> Option<Self::Item> {
        let r = self
            .iter
            .by_ref()
            .filter(|x| *x != NodeKind::Null)
            .map(|kind| (self.id as NodeId, kind))
            .next();
        self.id += 1;

        r
    }
}

/// An iterater over [`Nodes`].
pub struct NodeIter {
    mmap: MutexGuardArc<MmapMut>,
    node_idx: usize,
}

impl Iterator for NodeIter {
    type Item = NodeKind;

    fn next(&mut self) -> Option<Self::Item> {
        if self.node_idx * NODES_PER_BYTE > self.mmap.len() {
            return None;
        }
        let byte_idx = self.node_idx / NODES_PER_BYTE;
        let offset = self.node_idx % NODES_PER_BYTE;
        let node_byte = self.mmap[byte_idx];
        let bit_start = offset * BITS_PER_NODE;
        let bits = bit_start..(bit_start + BITS_PER_NODE);

        self.node_idx += 1;
        Some(NodeKind::from(node_byte.get_bits(bits)))
    }
}

/// The number of nodes stored in one byte.
pub const NODES_PER_BYTE: usize = 8 / BITS_PER_NODE;
/// The number of bits that make up one node label.
pub const BITS_PER_NODE: usize = 4;

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
    /// The node is a root node that represents the result of the computation.
    Root = 4,
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
            4 => Root,
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
