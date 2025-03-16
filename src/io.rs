use io_uring::{opcode, types, IoUring};
use std::future::{Future};
use std::task::Context;
use std::task::Poll;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::{fs, io};
use std::pin::Pin;
pub struct IOEngine {
    ring: IoUring,
    file: PathBuf
}

pub struct Page<const S:usize=4096> {
    page_idx: usize,
    data: [u8; S]
}

pub struct IOReadFuture {
}

impl Future for IOReadFuture {
    type Output = Vec<u8>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        todo!(); 
    }
}

impl IOEngine {
    fn new(path: PathBuf) {
    }

    fn __write_page(&self, page: Page) {
    }

    fn __read_page(idx: usize) -> Page {
        todo!()
    }
    
}
