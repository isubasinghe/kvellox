use crate::entry::EntryHeader;
use super::static_data::*;
use std::path::Path;
use std::fs::OpenOptions;
use libc;
use super::page::{PageCacheManager, Page};
use std::error::Error;
use monoio::fs::File as MonoFile;
use monoio::buf::IoBuf;
use std::collections::{HashSet, HashMap};
use futures::channel::oneshot;
use std::rc::Rc;
use std::cell::RefCell;

use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub enum SlabOp {
    Write {
        blob: Vec<u8>,
        resp: oneshot::Sender<Result<(), String>>,
        idx: usize,
    },
    Read {
        idx: usize,
        resp: oneshot::Sender<Result<Option<Vec<u8>>, String>>,
    },
    Tombstone {
        idx: usize,
    }
}

pub struct Slab <const N: i64> {
    page_manager: PageCacheManager,
    size_on_disk: i64,
    free_list: Vec<usize>,
    idx: i64,
    nb_max_items: i64,
    file: Rc<MonoFile>,
    pending_ops: Vec<SlabOp>,
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
            file: Rc::new(mono_file), 
            nb_max_items: (2*PAGE_SIZE)/N,
            pending_ops: Vec::with_capacity(64),
        };

        slab.rebuild().await;

        let target_size = std::cmp::max(slab.size_on_disk, 100 * 1024 * 1024); // 100MB initial
        if target_size > slab.size_on_disk {
             slab.preallocate(0, target_size).unwrap();
             slab.size_on_disk = target_size;
             slab.nb_max_items = target_size / N;
        }
        
        slab
    }

    pub async fn rebuild(&mut self) -> Vec<(Vec<u8>, usize)> {
        let number_pages = self.size_on_disk / PAGE_SIZE;
        
        let mut futures = FuturesUnordered::new();
        let mut results = Vec::new();
        
        for n in 0..number_pages {
            let offset = n * PAGE_SIZE;
            let file_ref = self.file.clone(); 
            futures.push(async move {
                let mut page = Page::new();
                if let Err(e) = page.read_from_file(&file_ref, offset as u64).await {
                    return Err((n, e));
                }
                Ok((n, page))
            });
        }

        while let Some(result) = futures.next().await {
            match result {
                Ok((page_idx, page)) => {
                    self.page_manager.page_hit(page_idx, page.clone());
                    
                    let items_per_page = PAGE_SIZE / N;
                    for i in 0..items_per_page {
                        let item_offset = (i * N) as usize;
                        let abs_idx = (page_idx * items_per_page) + i;
                        
                        let page_ptr = page.read_ptr();
                        let item_ptr = unsafe { page_ptr.add(item_offset) };
                        
                        let is_empty = unsafe { 
                            let slice = std::slice::from_raw_parts(item_ptr, EntryHeader::SIZE); 
                            slice.iter().all(|&b| b == 0)
                        };

                        if is_empty {
                             self.free_list.push(abs_idx as usize);
                        } else {
                            if abs_idx >= self.idx {
                                self.idx = abs_idx + 1;
                            }
                            
                            let header_slice = unsafe { std::slice::from_raw_parts(item_ptr, EntryHeader::SIZE) };
                            let header = EntryHeader::from_bytes(header_slice);
                            
                            if header.is_tombstone() {
                                self.free_list.push(abs_idx as usize);
                            } else {
                                let key_size = header.key_size as usize;
                                let key_ptr = unsafe { item_ptr.add(EntryHeader::SIZE) };
                                let key_slice = unsafe { std::slice::from_raw_parts(key_ptr, key_size) };
                                results.push((key_slice.to_vec(), abs_idx as usize));
                            }
                        }
                    }
                }
                Err((_page_idx, _e)) => {
                    panic!("Failed to rebuild");
                }
            }
        }
        results
    }

    pub fn preallocate(&mut self, offset: i64, size: i64) -> Result<(), Box<dyn Error>> {
        let fd = self.file.as_raw_fd(); 
        let ret = unsafe {
            libc::fallocate(fd, 0, offset, size)
        };

        if ret == 0 {
            self.size_on_disk = size;
            self.nb_max_items = size / N;
            Ok(())
        } else {
            Err(Box::new(std::io::Error::last_os_error()))
        }
    }

    pub fn resize(&mut self) -> Result<(), Box<dyn Error>> {
        let new_size = if self.size_on_disk < 10_000_000_000 { 
            self.size_on_disk * 2
        } else {
            self.size_on_disk + 10_000_000_000
        };
        self.preallocate(0, new_size)
    }

    pub fn queue_write(&mut self, blob: Vec<u8>, resp: oneshot::Sender<Result<(), String>>) -> usize {
        if self.free_list.is_empty() && self.idx >= self.nb_max_items {
            self.resize().unwrap();
        }

        let idx = if let Some(i) = self.free_list.pop() {
            i 
        } else {
            let i = self.idx;
            self.idx += 1;
            i as usize
        };

        self.pending_ops.push(SlabOp::Write { blob, resp, idx });
        idx
    }

    pub fn queue_read(&mut self, idx: usize, resp: oneshot::Sender<Result<Option<Vec<u8>>, String>>) {
        self.pending_ops.push(SlabOp::Read { idx, resp });
    }

    pub fn queue_tombstone(&mut self, idx: usize) {
        self.pending_ops.push(SlabOp::Tombstone { idx });
    }

    pub fn return_slot(&mut self, idx: usize) {
        self.free_list.push(idx);
    }

    pub fn drain_ops(&mut self) -> Vec<SlabOp> {
        std::mem::replace(&mut self.pending_ops, Vec::new())
    }

    pub fn has_pending_ops(&self) -> bool {
        !self.pending_ops.is_empty()
    }

    pub fn pending_ops_count(&self) -> usize {
        self.pending_ops.len()
    }
}

pub async fn process_batch<const N: i64>(slab: Rc<RefCell<Slab<N>>>, ops: Vec<SlabOp>) {
    if ops.is_empty() {
        return;
    }

    let mut pages_needed = HashSet::new();
    let mut op_locations = HashMap::new();

    // Identify needed pages
    for (i, op) in ops.iter().enumerate() {
        let idx = match op {
            SlabOp::Write { idx, .. } => *idx,
            SlabOp::Read { idx, .. } => *idx,
            SlabOp::Tombstone { idx } => *idx,
        };
        
        let entry_offset = (idx as i64) * N;
        let page_number = entry_offset / PAGE_SIZE;
        let page_offset = entry_offset % PAGE_SIZE;
        
        pages_needed.insert(page_number);
        op_locations.insert(i, (page_number, page_offset));
    }

    // Phase 1: Load missing pages
    let mut missing_pages = Vec::new();
    let file = {
        let mut s = slab.borrow_mut();
        // Check which pages are missing from cache
        for &page_num in pages_needed.iter() {
            if s.page_manager.get_page(page_num).is_none() {
                missing_pages.push(page_num);
            }
        }
        s.file.clone()
    };

    if !missing_pages.is_empty() {
        let mut load_futures = FuturesUnordered::new();
        for page_num in missing_pages {
            let file_ref = file.clone();
            load_futures.push(async move {
                let mut page = Page::new();
                let res = page.read_from_file(&file_ref, (page_num * PAGE_SIZE) as u64).await;
                (page_num, page, res)
            });
        }

        while let Some((p_num, page, res)) = load_futures.next().await {
            if res.is_ok() {
                slab.borrow_mut().page_manager.page_hit(p_num, page);
            }
        }
    }

    // Phase 2: Apply operations and identify dirty pages
    let mut dirty_pages = HashSet::new();
    let mut responses = Vec::new();

    {
        let mut s = slab.borrow_mut();
        for (i, op) in ops.into_iter().enumerate() {
            let (page_num, page_offset) = op_locations[&i];
            match op {
                SlabOp::Write { mut blob, resp, idx: _ } => {
                    if let Some(mut page) = s.page_manager.get_page(page_num) {
                        page.write_bytes(page_offset as u64, blob.as_mut_ptr(), blob.len());
                        dirty_pages.insert(page_num);
                        responses.push(Some((resp, Ok(()))));
                    } else {
                        let _ = resp.send(Err("Page load failed".to_string()));
                        responses.push(None); 
                    }
                },
                SlabOp::Read { idx: _, resp } => {
                    if let Some(page) = s.page_manager.get_page(page_num) {
                        let page_ptr = page.read_ptr();
                        let item_ptr = unsafe { page_ptr.add(page_offset as usize) };
                        let header_slice = unsafe { std::slice::from_raw_parts(item_ptr, EntryHeader::SIZE) };
                        let header = EntryHeader::from_bytes(header_slice);
                        
                        if header.key_size == 0 && header.value_size == 0 {
                            let _ = resp.send(Ok(None));
                        } else {
                            let val_offset = EntryHeader::SIZE + header.key_size as usize;
                            let val_ptr = unsafe { item_ptr.add(val_offset) };
                            let val_slice = unsafe { std::slice::from_raw_parts(val_ptr, header.value_size as usize) };
                            let _ = resp.send(Ok(Some(val_slice.to_vec())));
                        }
                    } else {
                        let _ = resp.send(Err("Page load failed".to_string()));
                    }
                    responses.push(None);
                },
                SlabOp::Tombstone { idx: _ } => {
                    if let Some(mut page) = s.page_manager.get_page(page_num) {
                        let page_ptr = page.read_ptr();
                        let item_ptr = unsafe { page_ptr.add(page_offset as usize) };
                        let header_slice = unsafe { std::slice::from_raw_parts(item_ptr, EntryHeader::SIZE) };
                        let mut header = EntryHeader::from_bytes(header_slice);
                        
                        header.set_tombstone();
                        let mut header_bytes = header.to_bytes();
                        page.write_bytes(page_offset as u64, header_bytes.as_mut_ptr(), EntryHeader::SIZE);
                        dirty_pages.insert(page_num);
                    }
                    responses.push(None);
                }
            }
        }
    }

    // Phase 3: Write dirty pages
    let file = slab.borrow().file.clone();
    let mut write_futures = FuturesUnordered::new();
    
    // Create a snapshot of pages to write
    let mut pages_to_write = Vec::new();
    {
        let mut s = slab.borrow_mut();
        for page_num in dirty_pages {
            if let Some(page) = s.page_manager.get_page(page_num) {
                // We clone the page to keep it alive during write.
                // Note: Page clone just increments pin count, it points to same memory.
                pages_to_write.push((page_num, page.clone()));
            }
        }
    }

    for (page_num, page) in pages_to_write {
        let file_ref = file.clone();
        write_futures.push(async move {
            page.write_to_file(&file_ref, (page_num * PAGE_SIZE) as u64).await
        });
    }

    let mut flush_failed = false;
    while let Some(res) = write_futures.next().await {
        if res.is_err() { flush_failed = true; }
    }

    // Phase 4: Send responses
    for r in responses {
        if let Some((tx, res)) = r {
            let final_res = if flush_failed { Err("Flush failed".to_string()) } else { res };
            let _ = tx.send(final_res);
        }
    }
}
