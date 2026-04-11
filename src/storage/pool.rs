use std::io;
use std::ptr::null_mut;

use libc::{
    mlock, mmap, munmap, MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_PRIVATE, PROT_READ,
    PROT_WRITE,
};

pub const PAGE_SIZE: usize = 4096;

/// Contiguous mmap-backed pool of PAGE_SIZE slots. The entire allocation is
/// registered with io_uring once as a fixed buffer so all reads and writes
/// avoid per-operation kernel memory mapping. The pool is not thread-safe —
/// it must only be accessed from the owning slab thread.
pub struct PagePool {
    base: *mut u8,
    capacity: usize,
    free_slots: Vec<u16>,
}

impl PagePool {
    pub fn new(capacity: usize) -> Result<Self, io::Error> {
        let total_size = capacity * PAGE_SIZE;

        let ptr = unsafe {
            mmap(
                null_mut(),
                total_size,
                PROT_READ | PROT_WRITE,
                MAP_ANONYMOUS | MAP_PRIVATE | MAP_HUGETLB,
                -1,
                0,
            )
        };

        let ptr = if ptr == MAP_FAILED {
            unsafe {
                mmap(
                    null_mut(),
                    total_size,
                    PROT_READ | PROT_WRITE,
                    MAP_ANONYMOUS | MAP_PRIVATE,
                    -1,
                    0,
                )
            }
        } else {
            ptr
        };

        if ptr == MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        // Lock pages in RAM and fault them in on the calling thread's NUMA node.
        // Must be called after the worker thread is pinned to its core so that
        // first-touch policy places the physical pages on the correct node.
        let ret = unsafe { mlock(ptr, total_size) };
        if ret != 0 {
            unsafe { munmap(ptr, total_size) };
            return Err(io::Error::last_os_error());
        }

        // Highest index first so pop() hands out slot 0 first.
        let free_slots = (0..capacity as u16).rev().collect();

        Ok(PagePool {
            base: ptr as *mut u8,
            capacity,
            free_slots,
        })
    }

    /// Allocate a slot. Returns the slot index and a pointer to its
    /// PAGE_SIZE-aligned buffer, or None if the pool is exhausted.
    pub fn alloc(&mut self) -> Option<(u16, *mut u8)> {
        let slot = self.free_slots.pop()?;
        let ptr = unsafe { self.base.add(slot as usize * PAGE_SIZE) };
        Some((slot, ptr))
    }

    /// Return a slot to the pool.
    pub fn free(&mut self, slot: u16) {
        self.free_slots.push(slot);
    }

    pub fn base_ptr(&self) -> *mut u8 {
        self.base
    }

    pub fn total_size(&self) -> usize {
        self.capacity * PAGE_SIZE
    }
}

impl Drop for PagePool {
    fn drop(&mut self) {
        unsafe {
            munmap(self.base as *mut libc::c_void, self.capacity * PAGE_SIZE);
        }
    }
}
