#![warn(missing_docs)]
//! A runtime for symmetric interaction combinators.

/// The default page size, 1MiB
pub const DEFAULT_PAGE_SIZE: usize = 1024 * 1024;

/// The SIC runtime.
pub struct Runtime {
    /// The runtime heap.
    heap: Heap,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            heap: Heap::new(DEFAULT_PAGE_SIZE),
        }
    }
}

impl Runtime {
    /// Create a new runtime.
    pub fn new() -> Self {
        Self::default()
    }
}

/// The runtime memory, made up of a set of memory pages.
pub struct Heap {
    /// The number of bytes in a memory page.
    page_size: usize,
    /// The memory pages.
    pages: Vec<MemoryPage>,
}

impl Heap {
    /// Create a new heap with the given page size.
    pub fn new(page_size: usize) -> Self {
        Self {
            page_size,
            pages: vec![MemoryPage::new(
                page_size / std::mem::size_of::<MemoryCell>(),
            )],
        }
    }
}

impl std::ops::Index<usize> for Heap {
    type Output = MemoryCell;

    fn index(&self, index: usize) -> &Self::Output {
        let page = index / self.page_size;
        let page_idx = index % self.page_size;
        &self.pages[page][page_idx]
    }
}

impl std::ops::IndexMut<usize> for Heap {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let page = index / self.page_size;
        let page_idx = index % self.page_size;
        &mut self.pages[page][page_idx]
    }
}

/// A raw page of memory.
///
/// A page is a collection of [`MemoryCell`].
pub struct MemoryPage {
    cells: Vec<MemoryCell>,
}

impl MemoryPage {
    /// Create a new memory page with the given number of cells.
    pub fn new(size: usize) -> Self {
        Self {
            cells: Vec::with_capacity(size),
        }
    }
}

impl std::ops::Index<usize> for MemoryPage {
    type Output = MemoryCell;

    fn index(&self, index: usize) -> &Self::Output {
        &self.cells[index]
    }
}

impl std::ops::IndexMut<usize> for MemoryPage {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.cells[index]
    }
}

bitfield::bitfield! {
  /// A raw memory cell.
  ///
  /// The cell is made up of 128 bits.
  ///
  /// The bit assi
  #[repr(transparent)]
  pub struct MemoryCell(u128);
  impl Debug;

  /// Set the cell kind
  pub from into MemoryCellKind, cell_kind, set_cell_kind: 3, 0;
}

/// The different kinds of memory cells.
#[derive(Debug)]
pub enum MemoryCellKind {
    /// A constructor node.
    Constructor,
    /// A duplicator node.
    Duplicator,
    /// An eraser node.
    Eraser,
    /// An external data node.
    ExternData,
    /// And external function node.
    ExternFn,
}

impl From<u128> for MemoryCellKind {
    fn from(value: u128) -> Self {
        match value {
            0 => Self::Constructor,
            1 => Self::Duplicator,
            2 => Self::Eraser,
            3 => Self::ExternData,
            4 => Self::ExternFn,
            _ => panic!("Invalid value for `MemoryCellKind`: {value}"),
        }
    }
}

impl From<MemoryCellKind> for u128 {
    fn from(val: MemoryCellKind) -> Self {
        match val {
            MemoryCellKind::Constructor => 0,
            MemoryCellKind::Duplicator => 1,
            MemoryCellKind::Eraser => 2,
            MemoryCellKind::ExternData => 3,
            MemoryCellKind::ExternFn => 4,
        }
    }
}
