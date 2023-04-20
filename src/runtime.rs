//! SIC runtime implementation.

use crate::prld::*;

/// The SIC runtime.
pub struct Runtime {
    /// The runtime heap.
    heap: Heap,
    /// The number of threads used by the runtime.
    thread_count: usize,
    /// Worker thread pool used by the runtime.
    thread_pool: ThreadPool,
}

/// A thread identifier in a runtime.
pub struct ThreadId(pub usize);
newtype!(ThreadId, usize);
impl From<usize> for ThreadId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        #[cfg(not(feature = "num_cpus"))]
        let thread_count = 1;
        #[cfg(feature = "num_cpus")]
        let thread_count = num_cpus::get();

        Self {
            heap: Heap::new(DEFAULT_PAGE_SIZE, thread_count),
            thread_count,
            thread_pool: ThreadPool::with_name("sucre_runtime_workers".into(), thread_count),
        }
    }
}

impl Runtime {
    /// Create a new runtime.
    pub fn new() -> Self {
        Self::default()
    }
}
