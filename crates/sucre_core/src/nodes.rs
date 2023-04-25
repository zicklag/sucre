//! Contains the [`Nodes`] store.

use std::sync::Arc;

use async_mutex::{Mutex, MutexGuardArc};
use cache_size::CacheType;
use memmap2::MmapMut;

/// Stores the interaction combinator nodes.
pub struct Nodes {
    mmap: Arc<Mutex<MmapMut>>,
    chunk_size: usize,
    threads: usize,
}

impl Clone for Nodes {
    fn clone(&self) -> Self {
        let mmap = self
            .mmap
            .try_lock()
            .expect("Cannot clone `Chunks` while it is locked");
        let mut new_mmap = MmapMut::map_anon(mmap.len()).expect("Could not map memory");
        new_mmap.copy_from_slice(&mmap);

        Self {
            mmap: Arc::new(Mutex::new(new_mmap)),
            chunk_size: self.chunk_size,
            threads: self.threads,
        }
    }
}

impl Nodes {
    /// Create a new node storage with the given `memory_size` and thread count.
    ///
    /// The `memory_size` is in bytes and will be rounded up to increments of the detected L2 cache
    /// size and the thread count ( i.e. the `memory_size` will be made evenly divisible by both ).
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
        // TODO(perf): Find out a good deafult L2 cache size guess.
        let chunk_size = cache_size::cache_size(2, CacheType::Data).unwrap_or(1024 * 1024 * 4);

        // Round the size up to an increment of L2 cache.
        let size = memory_size + (chunk_size - (memory_size % chunk_size));
        let size = size + (threads - (size % threads));

        Nodes {
            mmap: Arc::new(Mutex::new(
                MmapMut::map_anon(size).expect("Could not map memory"),
            )),
            chunk_size,
            threads,
        }
    }

    /// Get an unsafe iterator over the chunks, for the given number of threads.
    pub fn chunk_iter(&self) -> ChunkIter {
        ChunkIter {
            mmap: self.mmap.clone(),
            threads: self.threads,
            l2_cache_size: self.chunk_size,
            current_chunk: 0,
        }
    }
}

/// Iterator over the chunks of [`Nodes`].
pub struct ChunkIter {
    mmap: Arc<Mutex<MmapMut>>,
    threads: usize,
    l2_cache_size: usize,
    current_chunk: usize,
}

impl Iterator for ChunkIter {
    type Item = Chunk;

    fn next(&mut self) -> Option<Self::Item> {
        // Lock the memory map
        let mmap = self
            .mmap
            .try_lock_arc()
            .expect("Cannot iterate while locked");

        let chunk_count = mmap.len() / self.l2_cache_size;

        //  We've gone through all our chunks
        if self.current_chunk == chunk_count {
            return None;
        }

        let start = self.current_chunk * self.l2_cache_size;
        let end = start + self.l2_cache_size;

        self.current_chunk += 1;

        Some(Chunk {
            mmap,
            start,
            end,
            threads: self.threads,
        })
    }
}

/// A chunk of [`Nodes`].
///
/// Nodes are processed in chunks, dividing the nodes in the chunk between worker threads and
/// completing the chunk before moving on to the next chunk.
pub struct Chunk {
    mmap: MutexGuardArc<MmapMut>,
    start: usize,
    end: usize,
    threads: usize,
}

impl Chunk {
    /// Iterate over the slices for each thread to process.
    pub fn slice_for_threads(&mut self) -> std::slice::Chunks<u8> {
        let slice_size = self.end - self.start;
        self.mmap[self.start..self.end].chunks(slice_size / self.threads)
    }
}

bitfield::bitfield! {
    /// A set of 4 nodes packed into a byte.
    #[derive(Copy, Clone, Hash, PartialEq, PartialOrd, Ord, Eq)]
    #[repr(transparent)]
    pub struct NodeByte(u8);
    impl Debug;
    /// Get the first node.
    #[inline(always)]
    pub from into NodeKind, a, set_a: 0, 1;
    /// Get the second node.
    #[inline(always)]
    pub from into NodeKind, b, set_b: 2, 3;
    /// Get the third node.
    #[inline(always)]
    pub from into NodeKind, c, set_c: 4, 5;
    /// Get the fourth node.
    #[inline(always)]
    pub from into NodeKind, d, set_d: 6, 7;
}

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
