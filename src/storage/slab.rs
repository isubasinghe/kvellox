use libc;
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::mem::MaybeUninit;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::io::engine::*;
use crate::spsc::{ReaderQueue, SlabRingBuffer};
use crate::storage::page::{Page, PageCacheManager};
use crate::storage::pool::{PagePool, PAGE_SIZE};

const RING_DEPTH: usize = 256;

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum SlabOp {
    Read = 0,
    Write = 1,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct WriteData {
    key: *const u8,
    value: *const u8,
}

/// Fixed-size request passed through the SPSC queue.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SlabRequest {
    /// Caller-assigned token echoed back in the completion so the caller can
    /// correlate responses.
    pub token: u64,
    /// For writes: pointer to the source data. Null for reads.
    pub data: *const u8,
    /// Page index of concerned operation
    pub page_idx: u32, 
    /// padding
    _padding: [u8; 5],
}

impl SlabRequest {

    #[inline]
    pub fn is_read(&self) -> bool {
        self.data.is_null()
    }
}

const _: () = assert!(
    std::mem::size_of::<SlabRequest>() == 32,
    "SlabRequest must be 32 bytes"
);

enum OpKind {
    Read,
    Write,
}

/// Stored alongside a Read when the read was issued on behalf of a
/// write to an uncached page. On read CQE we copy the data into the
/// page and issue a WriteFixed.
struct DeferredWrite {
    data: *const u8,
    data_len: u32,
    slot_offset: u32,
    write_token: u64,
}

/// Per-operation context stored in a fixed-size table indexed by slot.
/// The slot index is used as io_uring user_data — O(1) lookup on CQE.
struct OpContext {
    page_idx: u32,
    generation: u32,
    caller_token: u64,
    page: Page,
    buf: *mut u8,
    kind: OpKind,
    deferred_write: Option<DeferredWrite>,
}

/// Fixed-size table of in-flight operation contexts. Slot indices are used
/// as io_uring user_data. Zero allocations on the hot path.
struct ContextTable {
    slots: [MaybeUninit<OpContext>; RING_DEPTH],
    free: [u8; RING_DEPTH],
    free_top: u8,
}

impl ContextTable {
    fn new() -> Self {
        let mut free = [0u8; RING_DEPTH];
        for i in 0..RING_DEPTH {
            free[i] = i as u8;
        }
        ContextTable {
            slots: unsafe { MaybeUninit::uninit().assume_init() },
            free,
            free_top: RING_DEPTH as u8,
        }
    }

    /// Allocate a slot and store the context. Returns the slot index for
    /// use as io_uring user_data.
    fn alloc(&mut self, ctx: OpContext) -> u8 {
        debug_assert!(self.free_top > 0, "context table full");
        self.free_top -= 1;
        let idx = self.free[self.free_top as usize];
        self.slots[idx as usize].write(ctx);
        idx
    }

    /// Take the context out of a slot and return it to the free list.
    fn take(&mut self, idx: u8) -> OpContext {
        let ctx = unsafe { self.slots[idx as usize].assume_init_read() };
        self.free[self.free_top as usize] = idx;
        self.free_top += 1;
        ctx
    }
}

/// Tracks which page_idx have bufferred writes
/// This allows for merging in multiple requests before it hits the io\_uring queue
pub struct InFlightWrites {
    keys: Vec<u32>,
}

/// Tracks which page_idxs have in-flight reads for dedup, and coalesces
/// duplicate read tokens.
struct InFlightReads {
    /// Sorted page_idx keys for binary search dedup.
    keys: Vec<u32>,
    
    /// Flat list of (page_idx, caller_token) for duplicate reads that arrived
    /// while the first read was still in-flight. Typically near-empty.
    waiters: Vec<(u32, u64)>,
}

impl InFlightReads {
    fn new() -> Self {
        InFlightReads {
            keys: Vec::with_capacity(RING_DEPTH*2), // guarantee no realloc on shift right
            waiters: Vec::with_capacity(RING_DEPTH*2),
        }
    }

    fn contains(&self, page_idx: u32) -> bool {
        self.keys.binary_search(&page_idx).is_ok()
    }

    fn insert(&mut self, page_idx: u32) {
        if let Err(pos) = self.keys.binary_search(&page_idx) {
            self.keys.insert(pos, page_idx);
        }
    }

    fn remove(&mut self, page_idx: u32) {
        if let Ok(pos) = self.keys.binary_search(&page_idx) {
            self.keys.remove(pos);
        }
    }

    fn add_waiter(&mut self, page_idx: u32, token: u64) {
        self.waiters.push((page_idx, token));
    }

    fn drain_waiters(&mut self, page_idx: u32, mut f: impl FnMut(u64)) {
        let mut i = 0;
        while i < self.waiters.len() {
            if self.waiters[i].0 == page_idx {
                let (_, token) = self.waiters.swap_remove(i);
                f(token);
            } else {
                i += 1;
            }
        }
    }
}

pub struct Slab {
    engine: Engine,
    slab_size: usize,
    _file: File,        // keeps the fd alive for the duration of the registration
    file_index: u32,    // index of the registered slab file
    buf_index: u16,     // index of the registered page pool buffer
    pool: Box<PagePool>,
    cache: PageCacheManager,
    contexts: ContextTable,
    inflight_reads: InFlightReads,
}

impl Slab {
    pub fn new(
        path: &Path,
        _slab_id: u64,
        slab_size: u64,
        pool_capacity: usize,
    ) -> Result<Slab, Box<dyn Error>> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        options.custom_flags(libc::O_DIRECT);

        let file_name = path.join(format!("db-size-{}.dbpart", slab_size));
        let file = options.open(file_name)?;
        let raw_fd = file.as_raw_fd();

        let pool = Box::new(PagePool::new(pool_capacity)?);

        let mut engine = Engine::new()?;
        let buf_index = engine.register_buffer(pool.base_ptr(), pool.total_size())?;
        engine.register_files(&[raw_fd])?;

        Ok(Slab {
            engine,
            slab_size: slab_size as usize,
            _file: file,
            file_index: 0,
            buf_index,
            pool,
            cache: PageCacheManager::new(pool_capacity),
            contexts: ContextTable::new(),
            inflight_reads: InFlightReads::new(),
        })
    }

    
    pub fn read_page(&mut self, req: *const SlabRequest) {
    }

    pub fn page_for_read(&mut self, page_idx: u64) -> (bool, Page) {
        if self.inflight_reads.contains(page_idx as u32) { 
            // might not exist in cache
            // operation ongoing
        }

        let pg_maybe = self.cache.get_page(page_idx as i64);
        
        let mut must_read = false;
        
        let pg = match pg_maybe {
            Some(pg) => pg,
            None => {
                must_read = true;
                let pg = Page::new(self.pool.as_mut());
                self.inflight_reads.insert(page_idx as u32);
                pg
            }
        };
        return (must_read, pg);
        

    }

    #[inline]
    pub fn read_page_inner(&mut self, idx: i64) -> Option<WorkUnit> {
        let (must_read, pg) = self.page_for_read(idx as u64);

        if must_read { 
            let work = WorkUnit::ReadFile(ReadFile { file_index: 0, buf: pg.as_mut_ptr(), offset: 0, buf_index: 0 });
            return Some(work);
        }
        None
    }

    pub fn write_page(&mut self, req: *const SlabRequest) {
        let mut maybe_work = None;
        unsafe { 
            maybe_work = self.read_page_inner((*req).page_idx as i64);
        }
        match maybe_work {
            Some(work) => {
            },
            None => {
                // chain onto existing read-work
            }
        }
    }

    pub fn handle_completions(&mut self) {
    }

    /// Main event loop. Reads requests from the SPSC queue, accumulates SQEs,
    /// and flushes to the kernel when the ring is full or every 100ms.
    /// Returns when shutdown is set.
    pub fn run(
        &mut self,
        queue: &mut SlabRingBuffer<ReaderQueue>,
        shutdown: &AtomicBool,
    ) {
        const SUBMIT_INTERVAL: Duration = Duration::from_millis(100);

        let mut last_submit = Instant::now();
        let mut pending: usize = 0;

        loop {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            // Drain the SPSC into the SQ ring until empty or ring is full.
            while !self.engine.full() {
                let got = queue.try_read_item(|ptr| {
                    let req = unsafe { &*(ptr as *const SlabRequest) };
                    if req.is_read() {
                        self.read_page(req);
                    }else {
                        self.write_page(req);
                    }
                    
                    true
                });
            }
        }
    }
}
