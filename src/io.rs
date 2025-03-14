use io_uring::{opcode, types, IoUring};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::{fs, io};

pub struct IOEngine {
    ring: IoUring,
    file: PathBuf
}

pub struct Page<const S:usize=4096> {
    page_idx: usize,
    data: [u8; S]
}

impl IOEngine {
    fn new(path: PathBuf) {
    }

    async fn write_page(&self, page: Page) {
    }

    async fn read_page(idx: usize) -> Page {
        todo!()
    }
}
