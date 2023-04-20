//! Runtime heap memory implementation.

use std::slice::Chunks;

use crate::prld::*;

/// The default page size: 20MiB.
pub const DEFAULT_PAGE_SIZE: usize = 1024 * 1024 * 20;

/// The runtime memory, made up of a set of memory pages.
#[derive(Debug, Clone)]
pub struct Heap {
    /// The number of bytes in a memory page.
    page_size: usize,
    /// The number of threads to create the heap for.
    thread_count: usize,
    /// The memory pages.
    pages: Pages,
    /// The slots used to allocate and deallocate memory from the heap's [`Pages`].
    allocated_slots: AllocatedSlots,
}

// SAFETY: These are not implemented automatically due to the use of the `UnsafeCell`, but we are
// not doing anything that should not be `Sync + Send` with it, so this should be fine.
unsafe impl Sync for Heap {}
unsafe impl Send for Heap {}

impl Heap {
    /// Create a new heap with the given page size.
    pub fn new(page_size: usize, thread_count: usize) -> Self {
        // Not sure if this is necessary, but let's try to avoid surprises.
        assert!(thread_count > 0, "Thread count cannot be 0");
        assert!(page_size > thread_count, "Page size is too small");
        assert!(
            page_size % thread_count == 0,
            "Page size must be evenly divisible by the thread count"
        );

        Self {
            page_size,
            thread_count,
            pages: Pages::new(page_size),
            allocated_slots: AllocatedSlots::new(page_size, thread_count),
        }
    }

    /// Get the thread count of this heap.
    pub fn thread_count(&self) -> usize {
        self.thread_count
    }

    /// Get the number of allocated memory pages
    pub fn page_count(&self) -> usize {
        self.pages.read().len()
    }

    /// Get the page size of this heap.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Load the data from the given slice into the heap, and clear out any data previously in the
    /// heap.
    pub fn load_from_slice(&mut self, data: &[u8]) {
        let pages = self.pages.unique().get_mut();

        let page_count = data.len() / self.page_size;

        for i in 0..page_count {
            let start = i * self.page_size;
            let end = (start + self.page_size).max(data.len());

            let mut page = MemoryPage::new(self.page_size);
            page.raw.copy_from_slice(&data[start..end]);

            pages.push(page);
        }

        todo!("Populate allocated regions in self.allocated_slots")
    }

    /// Snapshot the data in the heap to a raw buffer.
    pub fn save(&mut self) -> MmapMut {
        let pages = self.pages.unique().get_mut();
        let mut buffer = MmapMut::map_anon(self.page_size * pages.len()).unwrap();

        for (i, page) in pages.iter_mut().enumerate() {
            let start = self.page_size * i;
            let end = start + self.page_size;
            buffer[start..end].copy_from_slice(&page.raw)
        }

        buffer
    }
}

/// A collection of memory pages.
///
/// Used as the memory storage for a [`Heap`].
#[derive(Debug)]
pub struct Pages(DualArc<UnsafeCell<Vec<MemoryPage>>>);
newtype!(Pages, DualArc<UnsafeCell<Vec<MemoryPage>>>);

impl Clone for Pages {
    fn clone(&self) -> Self {
        Self(DualArc::new_unique(UnsafeCell::new(self.read().clone())))
    }
}

impl Pages {
    /// Create a new [`Pages`] struct with one page allocated up front.
    pub fn new(page_size: usize) -> Self {
        Self(DualArc::new_unique(UnsafeCell::new(vec![MemoryPage::new(
            page_size,
        )])))
    }

    /// Borrow the pages for reading.
    ///
    /// This will panic if the pages are currently shared with [`self::share`].
    pub fn read(&self) -> &Vec<MemoryPage> {
        // If we have unique access to the pages
        if let DualArc::Unique(arc) = &self.0 {
            // SAFE: While we have this reference to a unique arc, we know that there is nothing
            // else that also has a mutable reference.
            unsafe { &*arc.get() }
        } else {
            panic!("Cannot clone pages while possibly borrowed across multiple threads.");
        }
    }


    /// Get shared borrow of the memory pages' [`UnsafeCell`].
    ///
    /// This will allow you to unsafely modify the memory pages concurrently across multiple
    /// threads.
    pub fn share(&mut self) -> Arc<UnsafeCell<Vec<MemoryPage>>> {
        self.0.
    }
}

/// A raw page of memory.
///
/// A page stores a collection of [`Endpoint`]s, and when the current pages have run out of room, a
/// new one will be allocated.
#[derive(Debug)]
pub struct MemoryPage {
    raw: MmapMut,
    count: usize,
}

impl MemoryPage {
    /// Create a new memory page with the given number of cells.
    pub fn new(size: usize) -> Self {
        assert_eq!(
            size % ENDPOINT_SIZE,
            0,
            "Memory page size must be divisible by {}",
            ENDPOINT_SIZE
        );

        Self {
            raw: MmapMut::map_anon(size).unwrap(),
            count: size / ENDPOINT_SIZE,
        }
    }

    /// Iterate over the endpoints in the memory page.
    pub fn iter(&self) -> MemoryPageIter {
        MemoryPageIter {
            chunks: self.raw.chunks(ENDPOINT_SIZE),
        }
    }
}

/// Iterator over a [`MemoryPage`].
pub struct MemoryPageIter<'a> {
    chunks: Chunks<'a, u8>,
}

impl<'a> Iterator for MemoryPageIter<'a> {
    type Item = Endpoint;

    fn next(&mut self) -> Option<Self::Item> {
        self.chunks
            .next()
            .map(|x| unsafe { *(x.as_ptr() as *const Endpoint) })
    }
}

impl<'a> IntoIterator for &'a MemoryPage {
    type Item = Endpoint;

    type IntoIter = MemoryPageIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl std::ops::Index<usize> for MemoryPage {
    type Output = Endpoint;

    fn index(&self, index: usize) -> &Self::Output {
        assert!(index < self.count, "Index out of bounds");
        unsafe { &*(self.raw.as_ptr() as *const Endpoint).add(index) }
    }
}

impl std::ops::IndexMut<usize> for MemoryPage {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        assert!(index < self.count, "Index out of bounds");
        unsafe { &mut *(self.raw.as_mut_ptr() as *mut Endpoint).add(index) }
    }
}

impl Clone for MemoryPage {
    fn clone(&self) -> Self {
        let mut new_mmap = MmapMut::map_anon(self.raw.len()).unwrap();
        new_mmap.copy_from_slice(&self.raw);

        Self {
            raw: new_mmap,
            count: self.count,
        }
    }
}
