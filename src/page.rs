use std::alloc::{alloc, dealloc};
use std::{alloc::Layout, cell::RefCell, ptr::NonNull, rc::Rc};

use super::cache::S3Fifo;

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
            self.pin_count -= 1;
            if self.pin_count == 0 {
                let layout = Layout::from_size_align(4096, 4096).unwrap();
                dealloc(self.buffer.as_ptr(), layout);
            }
        }
    }
}

pub struct Page {
    data: NonNull<PageDescriptor>
}

impl Clone for Page {
    fn clone(&self) -> Self {
       unsafe { (*self.data.as_ptr()).pin_count += 1; };
       Page{data: self.data}
    }
}

impl Page {
    pub fn new() -> Page {
        Page { data: NonNull::new(&mut PageDescriptor::new()).unwrap() }
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

    pub fn page_hit(&mut self, page_idx: i64, page: Page) {
        self.pages.put(page_idx, page)
    }
}
