use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::*;
use std::marker::PhantomData;
use libc::{mmap, munmap, MAP_ANONYMOUS, MAP_FAILED, MAP_HUGETLB, MAP_PRIVATE, PROT_READ, PROT_WRITE};

#[repr(align(64))]
struct CachePadded<T>(T);

pub struct WriteQueue;

pub struct ReaderQueue;
pub struct InitQueue;

pub trait QueueActor {
}

impl QueueActor for WriteQueue {}
impl QueueActor for ReaderQueue {}
impl QueueActor for InitQueue {}

pub struct SlabRingBuffer<T: QueueActor> {
    base: NonNull<u8>,
    head: NonNull<CachePadded<AtomicUsize>>,
    tail_cache: CachePadded<usize>,
    tail: NonNull<CachePadded<AtomicUsize>>,
    head_cache: CachePadded<usize>,
    mask: usize,
    slot_shift: usize,
    total_size: usize,
    capacity: usize,
    counts: NonNull<AtomicUsize>,
    _mark: PhantomData<T>
}

pub enum SlabError {
   SlotSizeNotPowerOfTwo,
   CapacityNotPowerOfTwo,
}

pub fn new_slab_ring_buffer(capacity: usize, slot_size: usize) -> Result<(SlabRingBuffer<WriteQueue>, SlabRingBuffer<ReaderQueue>), SlabError> {
    SlabRingBuffer::<InitQueue>::new(capacity, slot_size)
}

impl SlabRingBuffer<InitQueue> {
    pub fn new(capacity: usize, slot_size: usize) -> Result<(SlabRingBuffer<WriteQueue>, SlabRingBuffer<ReaderQueue>), SlabError>{
        if !slot_size.is_power_of_two() {
            return Err(SlabError::SlotSizeNotPowerOfTwo);
        }

        if !capacity.is_power_of_two() {
            return Err(SlabError::CapacityNotPowerOfTwo);
        }


        let total_size = capacity * slot_size;
        let ptr = unsafe {
            mmap(
                    std::ptr::null_mut(),
                    total_size,
                    PROT_READ | PROT_WRITE,
                    MAP_ANONYMOUS | MAP_PRIVATE | MAP_HUGETLB,
                    -1, 
                    0
                )
        };

        let ptr = if ptr == MAP_FAILED {
            unsafe {
                mmap(
                    std::ptr::null_mut(),
                    total_size,
                    PROT_READ | PROT_WRITE,
                    MAP_ANONYMOUS | MAP_PRIVATE,
                    -1,
                    0
                )
            }
        }else {
            ptr
        };
        
        assert!(ptr != MAP_FAILED, "mmap failed: {}", std::io::Error::last_os_error());

        let base = NonNull::new(ptr as *mut u8).unwrap();
        let slot_shift = slot_size.trailing_zeros() as usize;
        let mask = total_size - 1;

        let head = NonNull::new(Box::into_raw(Box::new(CachePadded(AtomicUsize::new(0))))).unwrap();
        let tail = NonNull::new(Box::into_raw(Box::new(CachePadded(AtomicUsize::new(0))))).unwrap();
        let counts = NonNull::new(Box::into_raw(Box::new(AtomicUsize::new(2)))).unwrap();

        let writer: SlabRingBuffer<WriteQueue> = SlabRingBuffer {
            base, head, tail_cache: CachePadded(0),
            tail, head_cache: CachePadded(0),
             mask, slot_shift, total_size, capacity, counts,
            _mark: PhantomData,
        };
        let reader: SlabRingBuffer<ReaderQueue> = SlabRingBuffer {
            base, head, tail_cache: CachePadded(0),
            tail, head_cache: CachePadded(0),
            mask, slot_shift, total_size, capacity, counts,
            _mark: PhantomData,
        };

        Ok((writer, reader))
    }
}

impl<T: QueueActor> Drop for SlabRingBuffer<T> {
    fn drop(&mut self) {
        let counts = unsafe { self.counts.as_ref() }.fetch_sub(1, AcqRel);
        if counts <= 1 {
            unsafe {
                munmap(self.base.as_ptr() as *mut libc::c_void, self.total_size);
                drop(Box::from_raw(self.head.as_ptr()));
                drop(Box::from_raw(self.tail.as_ptr()));
                drop(Box::from_raw(self.counts.as_ptr()));
            }
        }
    }
}


impl SlabRingBuffer<WriteQueue> {
    pub fn try_write_item(&mut self, f: impl FnOnce(*mut u8) -> bool) -> bool {

        let head = unsafe { self.head.as_ref() }.0.load(Relaxed);
        if head - self.tail_cache.0 == self.capacity {
            let tail = unsafe { self.tail.as_ref() }.0.load(Acquire);
            self.tail_cache.0 = tail;

            if head - tail == self.capacity {
                return false
            }
        }
        let offset = (head << self.slot_shift) & self.mask;
        let ptr = unsafe { self.base.as_ptr().add(offset) };
        let wrote = f(ptr);
        if wrote {
            unsafe { self.head.as_ref() }.0.store(head+1, Release);
        }
        wrote
    }
}


impl SlabRingBuffer<ReaderQueue> {
    pub fn try_read_item(&mut self, f: impl FnOnce(*const u8)->bool ) -> bool {
        let tail = unsafe { self.tail.as_ref() }.0.load(Relaxed);
        if self.head_cache.0 == tail {
            let head = unsafe { self.head.as_ref() }.0.load(Acquire);
            self.head_cache.0 = head;

            if head == tail {
                return false
            }
        }

        let offset = (tail << self.slot_shift) & self.mask;
        let ptr = unsafe { self.base.as_ptr().add(offset) };
        let read = f(ptr);

        if read {
            unsafe { self.tail.as_ref() }.0.store(tail+1, Release);
        }
        read 
    }
}
