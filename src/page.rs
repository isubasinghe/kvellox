use std::alloc::{alloc, dealloc};
use std::fs::File;
use std::{alloc::Layout, cell::RefCell, ptr::NonNull, rc::Rc};
use monoio::buf::IoBufMut;
use monoio::fs::File as MonoFile;



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
    pub data: NonNull<PageDescriptor>
}

impl Clone for Page {
    fn clone(&self) -> Self {
       unsafe { (*self.data.as_ptr()).pin_count += 1; };
       Page{data: self.data}
    }
}

unsafe impl IoBufMut for Page {
    fn write_ptr(&mut self) -> *mut u8 {
        unsafe {
            self.data.as_ref().buffer.as_ptr()
        }
        
    }

    fn bytes_total(&mut self) -> usize {
        4096
    }

    unsafe fn set_init(&mut self, _: usize) {
     
    }

}
impl Page {
    pub fn new() -> Page {
        Page { data: NonNull::new(&mut PageDescriptor::new()).unwrap() }
    }

    pub async fn read_from_file(&mut self, file: &File, offset: u64) -> Result<(), Box<dyn std::error::Error>> {
        let f = file.try_clone()?;
        let mono_file = MonoFile::from_std(f)?;
        let (result, _) = mono_file.read_exact_at(self.clone(), offset).await;
        result?;

        Ok(())
    }

    pub async fn write_to_file(&mut self, file: &File, offset: u64) {
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
