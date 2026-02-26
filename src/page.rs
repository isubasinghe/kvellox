use std::alloc::{alloc, dealloc};
use std::fs::File;
use std::{alloc::Layout, cell::RefCell, ptr::NonNull, rc::Rc};
use monoio::buf::{IoBuf, IoBufMut};
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
            let layout = Layout::from_size_align(4096, 4096).unwrap();
            dealloc(self.buffer.as_ptr(), layout);
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

unsafe impl IoBuf for Page {
    fn read_ptr(&self) -> *const u8 {
        unsafe {
            self.data.as_ref().buffer.as_ptr()
        }
    }

    fn bytes_init(&self) -> usize {
        4096
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
        let descriptor = Box::new(PageDescriptor::new());
        Page {
            data: NonNull::new(Box::into_raw(descriptor)).unwrap(),
        }
    }

    pub async fn read_from_file(&mut self, file: &MonoFile, offset: u64) -> Result<(), Box<dyn std::error::Error>> {
        let (result, _) = file.read_exact_at(self.clone(), offset).await;
        result?;

        Ok(())
    }

    pub async fn write_to_file(&self, file: &MonoFile, offset: u64) -> Result<(), Box<dyn std::error::Error>> {
        let (result, _) = file.write_all_at(self.clone(), offset).await;
        result?;
        unsafe {
            (*self.data.as_ptr()).is_dirty = false;
        }
        Ok(())
    }

    pub fn write_bytes(&mut self, offset: u64, data: *mut u8, len: usize) {
        unsafe {
            std::ptr::copy_nonoverlapping(data, self.data.as_mut().buffer.offset(offset as isize).as_mut(), len); 
            self.data.as_mut().is_dirty = true;
        }
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
