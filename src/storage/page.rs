use std::alloc::{alloc, dealloc, Layout};
use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::cache::S3Fifo;
use crate::storage::pool::{PagePool, PAGE_SIZE};

pub struct PageCacheManager {
    pages: S3Fifo<i64, Page>,
}

enum PageBacking {
    Pool { slot: u16, pool: *mut PagePool },
    Heap,
}

pub struct PageDescriptor {
    pub buffer: NonNull<u8>,
    backing: PageBacking,
    pub is_dirty: Cell<bool>,
    generation: Cell<u64>,
}

impl PageDescriptor {
    /// Allocate a descriptor. Uses a pool slot if available, otherwise falls
    /// back to a heap allocation so the caller is never stalled.
    fn new(pool: *mut PagePool) -> Self {
        if let Some((slot, ptr)) = unsafe { (*pool).alloc() } {
            return PageDescriptor {
                buffer: NonNull::new(ptr).expect("pool returned null"),
                backing: PageBacking::Pool { slot, pool },
                is_dirty: Cell::new(false),
                generation: Cell::new(0),
            };
        }
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
        let ptr = unsafe { alloc(layout) };
        PageDescriptor {
            buffer: NonNull::new(ptr).expect("heap alloc failed"),
            backing: PageBacking::Heap,
            is_dirty: Cell::new(false),
            generation: Cell::new(0),
        }
    }
}

impl Drop for PageDescriptor {
    fn drop(&mut self) {
        match self.backing {
            PageBacking::Pool { slot, pool } => unsafe { (*pool).free(slot) },
            PageBacking::Heap => {
                let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).unwrap();
                unsafe { dealloc(self.buffer.as_ptr(), layout) };
            }
        }
    }
}

/// Reference-counted handle to a PageDescriptor. Cloning increments the
/// count; when the last handle drops, the backing buffer is returned to the
/// pool (or freed if heap-allocated). The ref count doubles as a pin: while
/// any in-flight I/O holds a clone, the buffer cannot be reclaimed.
#[derive(Clone)]
pub struct Page(Rc<PageDescriptor>);

impl Page {
    pub fn new(pool: *mut PagePool) -> Page {
        Page(Rc::new(PageDescriptor::new(pool)))
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.0.buffer.as_ptr()
    }

    pub fn as_mut_ptr(&self) -> *mut u8 {
        self.0.buffer.as_ptr()
    }

    pub fn write_bytes(&self, offset: u64, data: *const u8, len: usize) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                data,
                self.0.buffer.as_ptr().add(offset as usize),
                len,
            );
        }
        self.0.is_dirty.set(true);
        self.0.generation.set(self.0.generation.get().wrapping_add(1));
    }

    pub fn is_dirty(&self) -> bool {
        self.0.is_dirty.get()
    }

    pub fn mark_clean(&self) {
        self.0.is_dirty.set(false);
    }

    /// Number of live handles to this buffer. A count of 1 means only the
    /// cache holds a reference — safe to evict.
    pub fn ref_count(&self) -> usize {
        Rc::strong_count(&self.0)
    }

    pub fn generation(&self) -> u64 {
        self.0.generation.get()
    }
}

/// Pack a page index and generation into a single u64 for use as io_uring
/// user_data. page_idx occupies the upper 32 bits, generation (truncated to
/// u32) the lower 32 bits. Wrapping of the generation counter is fine — it
/// only needs to detect staleness within the lifetime of a single in-flight
/// operation, not across restarts.
pub fn encode_token(page_idx: u32, generation: u64) -> u64 {
    ((page_idx as u64) << 32) | (generation as u32 as u64)
}

pub fn decode_token(token: u64) -> (u32, u32) {
    let page_idx = (token >> 32) as u32;
    let generation = (token & 0xFFFF_FFFF) as u32;
    (page_idx, generation)
}

impl PageCacheManager {
    pub fn new(num_pages: usize) -> PageCacheManager {
        let pages_cache = S3Fifo::new(num_pages);
        PageCacheManager { pages: pages_cache }
    }

    pub fn get_page(&mut self, page_number: i64) -> Option<Page> {
        self.pages.get_mut(&page_number).cloned()
    }

    pub fn insert(&mut self, page_idx: i64, page: Page) {
        self.pages.put(page_idx, page)
    }
}
