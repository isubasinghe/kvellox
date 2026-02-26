use std::alloc::{alloc, dealloc};
use std::{alloc::Layout, ptr::NonNull};

use crate::cache::S3Fifo;

pub struct PageCacheManager {
    pages: S3Fifo<i64, Page>,
}

pub struct PageDescriptor {
    pub buffer: NonNull<u8>,
    pub pin_count: u32,
    pub is_dirty: bool,
}

impl PageDescriptor {
    pub fn new() -> Self {
        unsafe {
            let layout = Layout::from_size_align(4096, 4096).unwrap();
            let ptr = alloc(layout);

            Self {
                buffer: NonNull::new(ptr).expect("OOM"),
                pin_count: 1,
                is_dirty: false,
            }
        }
    }
}

impl Drop for PageDescriptor {
    fn drop(&mut self) {
        unsafe {
            let layout = Layout::from_size_align(4096, 4096).unwrap();
            dealloc(self.buffer.as_ptr(), layout);
        }
    }
}

pub struct Page {
    pub data: NonNull<PageDescriptor>,
}

impl Clone for Page {
    fn clone(&self) -> Self {
        unsafe { (*self.data.as_ptr()).pin_count += 1 };
        Page { data: self.data }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        unsafe {
            let desc = self.data.as_mut();
            desc.pin_count -= 1;
            if desc.pin_count == 0 {
                let _ = Box::from_raw(self.data.as_ptr());
            }
        }
    }
}

impl Page {
    pub fn new() -> Page {
        let descriptor = Box::new(PageDescriptor::new());
        Page {
            data: NonNull::new(Box::into_raw(descriptor)).unwrap(),
        }
    }

    pub fn as_ptr(&self) -> *const u8 {
        unsafe { self.data.as_ref().buffer.as_ptr() }
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        unsafe { self.data.as_ref().buffer.as_ptr() }
    }

    pub fn write_bytes(&mut self, offset: u64, data: *const u8, len: usize) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                data,
                self.data.as_mut().buffer.as_ptr().add(offset as usize),
                len,
            );
            self.data.as_mut().is_dirty = true;
        }
    }

    pub fn is_dirty(&self) -> bool {
        unsafe { self.data.as_ref().is_dirty }
    }

    pub fn mark_clean(&self) {
        unsafe { (*self.data.as_ptr()).is_dirty = false }
    }
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
