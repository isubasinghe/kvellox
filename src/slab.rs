use crate::entry::EntryHeader;

use super::static_data::*;
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::fs::OpenOptions;
use libc;
use super::page::{PageCacheManager, Page};
use std::error::Error;

use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, MetadataExt};

pub struct Slab <'a, const N: i64> {
    page_manager: PageCacheManager<'a>,
    size_on_disk: i64,
    free_list: Vec<usize>,
    idx: i64,
    nb_max_items: i64,
    nb_items: i64,
    file: File,
}

impl <'a, const N: i64> Slab<'a, N> {
    pub async fn new(root_path: &Path) -> Slab<N> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);

        #[cfg(target_os = "linux")]
        {
            options.custom_flags(libc::O_DIRECT);
        }

        #[cfg(target_os = "windows")]
        {
            // FILE_FLAG_NO_BUFFERING = 0x20000000
            // FILE_FLAG_WRITE_THROUGH = 0x80000000 (often used together)
            const FILE_FLAG_NO_BUFFERING: u32 = 0x20000000;
            options.custom_flags(FILE_FLAG_NO_BUFFERING);
        }

        #[cfg(target_os = "macos")]
        {
            // macOS doesn't support a direct I/O flag in open().
            // You must open normally, then disable caching via fcntl.
            let file = options.open(path)?;
            use std::os::unix::io::AsRawFd;
            unsafe {
                let fd = file.as_raw_fd();
                // F_NOCACHE = 48
                libc::fcntl(fd, libc::F_NOCACHE, 1);
            }
            return Ok(file);
        }

        let file_name = root_path.join(format!("db-size-{}.dbpart", N));

        let file = options.open(file_name).unwrap();

        let meta = file.metadata().unwrap();
        
        println!("Size: {}", meta.len());
        println!("Block size: {}", meta.blksize());
        println!("Blocks allocated: {}", meta.blocks());

        let mut page_cache_manager = PageCacheManager::new(PAGE_CACHE_SIZE as usize);
        if meta.len() != 0 {
            panic!("recovery not supported at the moment");
        }
        let mut slab = Slab{page_manager: page_cache_manager, size_on_disk: meta.len() as i64, free_list: Vec::new(), idx: 0, file, nb_max_items: (N/(2*PAGE_SIZE)), nb_items: 0};
        slab.preallocate(0, 2 * PAGE_SIZE).unwrap();
        slab.size_on_disk = 2 * PAGE_SIZE;
        slab
    }

    pub async fn advance_idx(&mut self) {

    }

    pub fn preallocate(&mut self, offset: i64, size: i64) -> Result<(), Box<dyn Error>> {
        let fd = self.file.as_raw_fd(); 
        let mode = libc::FALLOC_FL_KEEP_SIZE;
        let offset = offset;
        let len = size;
        let ret = unsafe {
            libc::fallocate(fd, mode, offset, len)
        };

        if ret == 0 {
            self.nb_max_items = N/size;
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

    pub async fn write_item(&mut self, key: &'a[u8], value: &'a [u8]) -> Result<(), Box<dyn Error>>{
        if self.free_list.len() != 0 {
            panic!("no support for this at the moment");
        }
        if self.idx >= self.nb_max_items {
            return self.resize();
        }
        let page_number = (self.idx * N) / PAGE_SIZE;
        let mut entry_header = EntryHeader{timestamp: 0, key_size: key.len() as i64, value_size: value.len() as i64, meta: 0};

        Ok(())
    }

    pub async fn read_item(idx: usize) {
    }

    pub async fn read_page() {
    }

    pub async fn write_page() {
    }
}
