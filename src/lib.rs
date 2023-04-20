//! A runtime for symmetric interaction combinators.

#![warn(missing_docs)]

#[macro_use]
pub(crate) mod utils;
pub mod allocator;
pub mod heap;
pub mod memory;
pub mod runtime;

/// An internal prelude used by modules in this crate, as differentiated from the external prelude
/// which is meant for users of this crate.
mod prld {
    pub use atomic_refcell::AtomicRefCell;
    pub use memmap2::MmapMut;
    pub use parking_lot::Mutex;
    pub use roaring::RoaringBitmap as Bitmap;
    pub use std::cell::UnsafeCell;
    pub use threadpool::ThreadPool;
    pub use triomphe::{Arc, UniqueArc};

    pub use crate::{allocator::*, heap::*, memory::*, runtime::*, utils::*};

    /// Shortcut for [`Default::default()`]
    pub fn default<D: Default>() -> D {
        Default::default()
    }
}

#[doc(inline)]
pub use runtime::Runtime;
