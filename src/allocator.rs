//! Memory allocator implementation.

use crate::prld::*;

/// Struct containing the free slots for the allocation of [`Endpoint`]s in a [`Heap`].
///
/// This contains the allocation logic, but doesn't actually touch any physical memory. The [`Heap`]
/// is repsonsible for allocating the physical memory in response to the calculations performed by
/// the [`AllocatorSlots`].
///
/// # Strategy
///
/// Each thread is given an equal partition of each memory page that it is allowed to allocate to.
/// This allows each thread to make allocations concurrently without blocking each-other, assuming
/// they first obtain the lock for their thread ID.
///
/// Deallocations on the other hand must be done with a borrow of all of the slots, because we may
/// want to de-allocate memory from any thread, so deallocation should be done in bulk after
/// collecting a list of items to deallocate.
#[derive(Clone, Debug)]
pub struct AllocatedSlots {
    /// The vector will have one item for every thread.
    ///
    /// For each thread, there are three bitmaps, containing the free slots for allocations with a
    /// size and alignment of 1, 2, or 3 respectively.
    ///
    /// All allocations must be made with an equal size and alignment of either 1, 2, or three.
    threads: Vec<AllocatorSlotsPartition>,
    /// The page size of the heap this is related to.
    page_size: usize,
    /// The thread count of the heap this is related to.
    thread_count: usize,
}

/// A set of free slots for allocation for a specific thread.
#[derive(Clone, Debug)]
pub struct AllocatorSlotsPartition(Arc<Mutex<Bitmap>>);
newtype!(AllocatorSlotsPartition, Arc<Mutex<Bitmap>>);

impl Default for AllocatorSlotsPartition {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Bitmap::new())))
    }
}

impl AllocatedSlots {
    /// Create a new free slot list for the given number of threads.
    pub fn new(page_size: usize, thread_count: usize) -> Self {
        Self {
            thread_count,
            page_size,
            threads: (0..thread_count).map(|_| default()).collect(),
        }
    }

    /// Allocate a slot for the given `size` for the given `thread_id`.
    ///
    /// The align is always the same as the size, and must be either `1`, `2`, or `3`.
    pub fn allocate(&self, thread_id: ThreadId, size: usize) -> usize {
        // Lock the bitsets for the given thread.
        let mut bitset = self.threads[thread_id.0].lock();

        Self::allocate_with_bitset(
            &mut bitset,
            self.page_size,
            self.thread_count,
            thread_id,
            size,
        )
    }

    /// Make an allocation int
    pub fn allocate_with_bitset(
        bitset: &mut Bitmap,
        page_size: usize,
        thread_count: usize,
        thread_id: ThreadId,
        size: usize,
    ) -> usize {
        assert!(
            size > 0 && size < 4,
            "All allocations must be of size 1, 2, or 3, but got {size}"
        );

        // First, we find a free spot for an allocation of the given size.
        let mut iter = bitset.iter(); // This iterates over existing allocations.
        let mut last_taken_spot = -1;
        let thread_memory_index = 'search: loop {
            let next_end = iter.next().expect("Ran out of available memory slots!") as i32;

            // If the gap between the last taken spot and the next taken spot is big enough to fit
            // our allocation.
            let next_start = last_taken_spot + 1;
            let gap_size = next_end - next_start;
            if gap_size >= size as i32 {
                // See if we can find an empty spot that is of the required alignment
                for i in 0..=(gap_size - size as i32) {
                    // If the align matches
                    if next_start + i % size as i32 == 0 {
                        // We found a spot! Allocate the slots.
                        let start = (next_start + i) as u32;
                        let end = start + size as u32;
                        let written_bits = bitset.insert_range(start..end);
                        assert!(
                            written_bits == size as u64,
                            "The spot chosen for allocation wasn't actually open."
                        );
                        break 'search start;
                    }
                }
            }

            // If we didn't find a spot yet, update the last taken spot
            last_taken_spot = next_start;
        };

        // The thread memory index does **not** match the heap memory index. Each thread gets an
        // equal chunk of each memory page that it is allowed to allocate to. So when you get to a
        // thread_memory_index of `page_size / thread_count * thread_id + 1` the heap memory index
        // makes a jump to the next memory page.
        {
            let thread_memory_index = thread_memory_index as usize;

            // The ammount of memory in each page that this thread is allowed to allocate to.
            let chunk_size = page_size / thread_count;
            // The page that this thread memory index lands in.
            let page_id = thread_memory_index / chunk_size;
            // The offset of the chunk in the page
            let chunk_page_offset = thread_id.0 * chunk_size;
            // The offset of the slot in the chunk
            let slot_chunk_offset = thread_memory_index % chunk_size;

            // The final memory index
            page_id * page_size + chunk_page_offset + slot_chunk_offset
        }
    }

    /// Deallocate the given `loc` for the provided size.
    ///
    /// This will panic if a lock on the thread slots for the given `loc` cannot be obtained without
    /// blocking.
    ///
    /// **Note:** If we end up needing a blocking version of this method we should create a
    /// `deallocate_blocking` method for that.
    ///
    /// # Correctness
    ///
    /// In order to get correct and efficient behavior the `size` must be the same size that the
    /// location was allocated with.
    ///
    /// # Panics
    ///
    /// `loc` must be in increments of `size`, because the align and size must always match,
    /// otherwise this will panic.
    pub fn deallocate(&self, loc: usize, size: usize) {
        assert!(
            size > 0 && size < 4,
            "The size for a deallocation must be either 1, 2, or 3."
        );
        assert!(
            loc % size == 0,
            "The location ( {loc} ) must be divisible by the size ( {size} ) when deallocating."
        );

        let page_size = self.page_size;
        let thread_count = self.thread_count;

        // Determine the thread_id of the thread that owns this `loc`
        let chunk_size = page_size / thread_count;
        let page_id = loc / page_size;
        let page_offset = loc % page_size;
        let thread_id = page_offset / chunk_size;
        let chunk_offset = page_offset % chunk_size;
        let thread_memory_index = page_id * chunk_size + chunk_offset;
        let thread_memory_index = thread_memory_index as u64;

        let mut bitset = self.threads[thread_id]
            .try_lock()
            .expect("Cannot deallocate, thread slots are locked.");

        let start = thread_memory_index as u32;
        let end = start + size as u32;
        bitset.remove_range(start..end);
    }
}
