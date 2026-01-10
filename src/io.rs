use super::page::PageCacheManager;
use io_uring::{opcode, types, IoUring};
use std::fs;
use std::fs::File;
use std::future::Future;
use std::io::{self, Read};
use std::os::unix::fs::FileExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::AtomicU8;
use std::sync::Arc;
use std::sync::Mutex;
use std::task::Poll;
use std::task::{Context, Waker};

pub struct IOEngine {
    ring: IoUring,
    file: PathBuf,
}

pub struct Page {
    page_idx: u64,
    data: [u8; 3990], // 4096 - 8 - 8
    next_page: u64,
    dirty: bool
}

pub struct IOReadFuture {
    shared_state: Arc<Mutex<SharedState>>,
}

impl Future for IOReadFuture {
    type Output = Vec<u8>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut shared_state = self.shared_state.lock().unwrap();
        if shared_state.completed {
            return Poll::Ready(vec![]);
        }
        shared_state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

// Shared state data between the future and the waiting thread
pub struct SharedState {
    waker: Option<Waker>,
    data: Option<Vec<u8>>,
    completed: bool,
}

pub struct FileReader {
    fd: i32,
    page_size: u64,
    last_page_idx: u64,
}

impl FileReader {
    fn new(path: PathBuf) -> FileReader {
        let file = File::create_new(path).unwrap();
        let fd = file.as_raw_fd();
        let mut buffer = [0u8; 8];
        file.read_exact_at(&mut buffer, 0).unwrap();
        let page_size = u64::from_le_bytes(buffer);
        file.read_exact_at(&mut buffer, 8).unwrap();
        let last_page_idx = u64::from_le_bytes(buffer);
        FileReader {
            fd,
            page_size,
            last_page_idx,
        }
    }
}

impl IOEngine {
    fn new(path: PathBuf) {}

    fn __write_page(&self, page: Page) -> IOReadFuture {
        todo!()
    }

    fn __read_page(page_idx: usize) -> Page {
        let seek = todo!();
        todo!()
    }

    fn get_page_cache_hash(&self, page: Page) {
        let h = 0;
    }

}
