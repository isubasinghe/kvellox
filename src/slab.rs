use crate::entry::EntryHeader;
use super::static_data::*;
use std::time::{SystemTime, UNIX_EPOCH};
use std::path::Path;
use std::fs::OpenOptions;
use libc;
use super::page::{PageCacheManager, Page};
use std::error::Error;
use monoio::fs::File as MonoFile;
use monoio::buf::IoBuf;

use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, MetadataExt};

pub struct Slab <const N: i64> {
    page_manager: PageCacheManager,
    size_on_disk: i64,
    free_list: Vec<usize>,
    idx: i64,
    nb_max_items: i64,
    file: MonoFile,
}

use futures::stream::{FuturesUnordered, StreamExt};

impl < const N: i64> Slab< N> {
    pub async fn new(root_path: &Path) -> Slab<N> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);

        #[cfg(target_os = "linux")]
        {
            options.custom_flags(libc::O_DIRECT);
        }

        #[cfg(target_os = "windows")]
        {
            const FILE_FLAG_NO_BUFFERING: u32 = 0x20000000;
            options.custom_flags(FILE_FLAG_NO_BUFFERING);
        }

        let file_name = root_path.join(format!("db-size-{}.dbpart", N));

        let std_file = options.open(file_name).unwrap();

        let meta = std_file.metadata().unwrap();
        
        let page_cache_manager = PageCacheManager::new(PAGE_CACHE_SIZE as usize);

        let mono_file = MonoFile::from_std(std_file).unwrap();

        let mut slab = Slab{
            page_manager: page_cache_manager, 
            size_on_disk: meta.len() as i64, 
            free_list: Vec::new(), 
            idx: 0, 
            file: mono_file, 
            nb_max_items: (2*PAGE_SIZE)/N, 
        };

        slab.rebuild().await;

        let target_size = std::cmp::max(slab.size_on_disk, 2 * PAGE_SIZE);
        if target_size > slab.size_on_disk {
             slab.preallocate(0, target_size).unwrap();
             slab.size_on_disk = target_size;
             slab.nb_max_items = target_size / N;
        }
        
        slab
    }

    pub async fn rebuild(&mut self) {
        let number_pages = self.size_on_disk / (PAGE_SIZE);
        let items_per_page = PAGE_SIZE / N;
        let max_items = number_pages * items_per_page;
        self.nb_max_items = max_items;
        
        let mut futures = FuturesUnordered::new();
        
        // Spawn reads
        for n in 0..number_pages {
            let offset = n * PAGE_SIZE;
            let file_ref = &self.file; 
            futures.push(async move {
                let mut page = Page::new();
                if let Err(e) = page.read_from_file(file_ref, offset as u64).await {
                    return Err((n, e));
                }
                Ok((n, page))
            });
        }

        // Process pages as they load
        while let Some(result) = futures.next().await {
            match result {
                Ok((page_idx, page)) => {
                    self.page_manager.page_hit(page_idx, page.clone());
                    
                    // Iterate over items in the page
                    let items_per_page = PAGE_SIZE / N;
                    for i in 0..items_per_page {
                        let item_offset = (i * N) as usize;
                        let abs_idx = (page_idx * items_per_page) + i;
                        
                        // Access raw bytes of the item
                        let page_ptr = page.read_ptr();
                        let item_ptr = unsafe { page_ptr.add(item_offset) };
                        
                        let is_empty = unsafe { 
                            let slice = std::slice::from_raw_parts(item_ptr, EntryHeader::SIZE); 
                            slice.iter().all(|&b| b == 0)
                        };

                        if is_empty {
                             self.free_list.push(abs_idx as usize);
                        } else {
                            // Valid Item
                            if abs_idx >= self.idx {
                                self.idx = abs_idx + 1;
                            }
                            
                            // Parse Header
                            let header_slice = unsafe { std::slice::from_raw_parts(item_ptr, EntryHeader::SIZE) };
                            let _header = EntryHeader::from_bytes(header_slice);
                            
                            // TODO: Extract Key and Insert into Index
                        }
                    }
                }
                Err((_page_idx, _e)) => {
                    panic!("Failed to rebuild");
                }
            }
        }
    }

    pub fn preallocate(&mut self, offset: i64, size: i64) -> Result<(), Box<dyn Error>> {
        let fd = self.file.as_raw_fd(); 
        let mode = 0;
        let offset = offset;
        let len = size;
        let ret = unsafe {
            libc::fallocate(fd, mode, offset, len)
        };

        if ret == 0 {
            self.size_on_disk = size;
            self.nb_max_items = size / N;
            Ok(())
        }else {
            Err(Box::new(std::io::Error::last_os_error()))
        }
    }

    pub fn resize(&mut self) -> Result<(), Box<dyn Error>> {
        let new_size: i64;
        if self.size_on_disk < 10000000000 { 
            new_size = self.size_on_disk * 2;
        }else {
            new_size = self.size_on_disk + 10000000000;
        }
        self.preallocate(0, new_size)
    }
    
    pub async fn write_item<'a>(&mut self, key: &'a[u8], value: &'a [u8]) -> Result<(), Box<dyn Error>>{
        let item_idx = if let Some(idx) = self.free_list.pop() {
            idx as i64
        } else {
            if self.idx >= self.nb_max_items {
                self.resize()?;
            }
            let idx = self.idx;
            self.idx += 1;
            idx
        };

        let entry_offset = item_idx * N;
        let page_number = entry_offset / PAGE_SIZE;
        let page_offset = entry_offset % PAGE_SIZE;

        let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let entry_header = EntryHeader{timestamp: time, key_size: key.len() as i64, value_size: value.len() as i64, meta: 0};

        let header_bytes = entry_header.to_bytes();
        let total_size = EntryHeader::SIZE + key.len() + value.len();
        
        let mut buffer = Vec::with_capacity(total_size);
        buffer.extend_from_slice(&header_bytes);
        buffer.extend_from_slice(key);
        buffer.extend_from_slice(value);

        let maybe_page = self.page_manager.get_page(page_number);
        let mut page = match maybe_page {
            Some(page) => page,
            None => {
                self.read_page(page_number).await?
            }
        };
        
        page.write_bytes(page_offset as u64, buffer.as_mut_ptr(), buffer.len());
        let page_start_offset = page_number * PAGE_SIZE;
        page.write_to_file(&self.file, page_start_offset as u64).await?;
        
        Ok(())
    }

    pub async fn read_item(idx: usize) {
    }

    pub async fn read_page(&mut self, page_index: i64) -> Result<Page, Box<dyn Error>> {
        let offset = page_index*PAGE_SIZE;
        let mut page = Page::new();
        page.read_from_file(&self.file, offset as u64).await?;
        self.page_manager.page_hit(page_index, page.clone());
        Ok(page)
    }

    pub async fn write_page() {
    }
}
