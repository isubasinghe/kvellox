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

#[derive(Debug)]
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

impl<T: QueueActor> SlabRingBuffer<T> {
    pub fn base_ptr(&self) -> *mut u8 {
        self.base.as_ptr()
    }

    pub fn total_size(&self) -> usize {
        self.total_size
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

unsafe impl Send for SlabRingBuffer<WriteQueue> {}
unsafe impl Send for SlabRingBuffer<ReaderQueue> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    const PAGE_SIZE: usize = 4096;
    const U32S_PER_PAGE: usize = PAGE_SIZE / std::mem::size_of::<u32>();
    const TOTAL_U32S: u32 = (U32S_PER_PAGE * 2) as u32;

    fn produce_two_pages(mut writer: SlabRingBuffer<WriteQueue>) {
        let mut counter: u32 = 0;
        for page in 0..2 {
            loop {
                let ok = writer.try_write_item(|ptr| {
                    let slot = ptr as *mut u32;
                    for i in 0..U32S_PER_PAGE {
                        unsafe { slot.add(i).write(counter) };
                        counter += 1;
                    }
                    true
                });
                if ok {
                    break;
                }
                std::hint::black_box(page);
                std::hint::spin_loop();
            }
        }
    }

    fn consume_two_pages(mut reader: SlabRingBuffer<ReaderQueue>) -> u32 {
        let mut expected: u32 = 0;
        let mut pages_read: u32 = 0;

        while pages_read < 2 {
            let read = reader.try_read_item(|ptr| {
                let slot = ptr as *const u32;
                for i in 0..U32S_PER_PAGE {
                    let val = unsafe { slot.add(i).read() };
                    assert_eq!(val, expected, "at page {} offset {}", pages_read, i);
                    expected += 1;
                }
                true
            });
            if read {
                pages_read += 1;
            } else {
                std::hint::black_box(pages_read);
                std::hint::spin_loop();
            }
        }
        expected
    }

    #[test]
    fn producer_spins_then_consumer_drains() {
        let (mut writer, reader) = new_slab_ring_buffer(1, PAGE_SIZE).unwrap();

        let producer_spinning = Arc::new(AtomicBool::new(false));
        let spinning = producer_spinning.clone();

        let producer = std::thread::spawn(move || {
            // First write succeeds — buffer has one slot.
            let ok = writer.try_write_item(|ptr| {
                let slot = ptr as *mut u32;
                for i in 0..U32S_PER_PAGE {
                    unsafe { slot.add(i).write(i as u32) };
                }
                true
            });
            assert!(ok, "first write should succeed");

            // Signal that we're about to spin on the second write.
            spinning.store(true, Ordering::Release);

            // Second write spins — buffer is full until consumer drains.
            let mut counter: u32 = U32S_PER_PAGE as u32;
            loop {
                let ok = writer.try_write_item(|ptr| {
                    let slot = ptr as *mut u32;
                    for i in 0..U32S_PER_PAGE {
                        unsafe { slot.add(i).write(counter) };
                        counter += 1;
                    }
                    true
                });
                if ok {
                    break;
                }
                std::hint::spin_loop();
            }
        });

        // Wait until the producer is actually spinning.
        while !producer_spinning.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }

        let consumer = std::thread::spawn(move || consume_two_pages(reader));

        producer.join().expect("producer panicked");
        let final_val = consumer.join().expect("consumer panicked");
        assert_eq!(final_val, TOTAL_U32S);
    }

    #[test]
    fn consumer_spins_then_producer_writes() {
        let (writer, reader) = new_slab_ring_buffer(1, PAGE_SIZE).unwrap();

        let consumer_ready = Arc::new(AtomicBool::new(false));
        let ready = consumer_ready.clone();

        let consumer = std::thread::spawn(move || {
            // Spin 1000 times on an empty queue before signalling.
            for i in 0..1000u32 {
                std::hint::black_box(i);
                std::hint::spin_loop();
            }
            ready.store(true, Ordering::Release);

            consume_two_pages(reader)
        });

        // Wait until the consumer has finished its idle spin.
        while !consumer_ready.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }

        let producer = std::thread::spawn(move || produce_two_pages(writer));

        producer.join().expect("producer panicked");
        let final_val = consumer.join().expect("consumer panicked");
        assert_eq!(final_val, TOTAL_U32S);
    }
}
