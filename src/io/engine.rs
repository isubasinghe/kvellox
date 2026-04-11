use std::error::Error;
use std::os::unix::io::RawFd;

use io_uring::{opcode, squeue, types, IoUring};
use libc;

use crate::storage::pool::PAGE_SIZE;

pub struct ReadFile {
    pub file_index: u32,
    pub buf: *mut u8,
    pub offset: u64,
    pub buf_index: u16,
}

pub struct WriteFile {
    pub file_index: u32,
    pub buf: *const u8,
    pub offset: u64,
    pub buf_index: u16,
}

pub enum WorkUnit {
    ReadFile(ReadFile),
    WriteFile(WriteFile),
}

impl WorkUnit {
    pub fn build_entry(&self, token: u64) -> squeue::Entry {
        match self {
            WorkUnit::ReadFile(r) => {
                opcode::ReadFixed::new(
                    types::Fixed(r.file_index),
                    r.buf,
                    PAGE_SIZE as u32,
                    r.buf_index,
                )
                .offset(r.offset)
                .build()
                .user_data(token)
            }
            WorkUnit::WriteFile(w) => {
                opcode::WriteFixed::new(
                    types::Fixed(w.file_index),
                    w.buf,
                    PAGE_SIZE as u32,
                    w.buf_index,
                )
                .offset(w.offset)
                .build()
                .user_data(token)
            }
        }
    }
}

pub struct Engine {
    ring: IoUring,
    count: usize,
    ring_cap: usize,
}

impl Engine {
    pub fn new() -> Result<Engine, Box<dyn Error>> {
        let ring = IoUring::builder()
            .setup_single_issuer()
            .setup_coop_taskrun()
            .setup_taskrun_flag()
            .setup_submit_all()
            .build(256)?;
        Ok(Engine { ring, count: 0, ring_cap: 256 })
    }

    /// Registers a contiguous memory region as a fixed buffer with io_uring.
    /// Returns the buffer index to use with ReadFixed/WriteFixed opcodes.
    pub fn register_buffer(&mut self, base: *mut u8, size: usize) -> Result<u16, Box<dyn Error>> {
        let iov = libc::iovec {
            iov_base: base as *mut libc::c_void,
            iov_len: size,
        };
        unsafe { self.ring.submitter().register_buffers(&[iov])? };
        Ok(0)
    }

    /// Registers file descriptors with io_uring. Once registered, use
    /// types::Fixed(index) instead of types::Fd(fd) in SQEs to avoid
    /// per-operation fd table lookups.
    pub fn register_files(&mut self, fds: &[RawFd]) -> Result<(), Box<dyn Error>> {
        self.ring.submitter().register_files(fds)?;
        Ok(())
    }

    /// Pins io_uring's internal async worker threads to the given CPU set.
    /// Call this after pinning the slab worker thread so that any ops the
    /// kernel can't complete inline stay on the same NUMA node.
    pub fn set_iowq_affinity(&mut self, cpu_set: &libc::cpu_set_t) -> Result<(), Box<dyn Error>> {
        self.ring.submitter().register_iowq_aff(cpu_set)?;
        Ok(())
    }

    /// Push a SQE onto the submission ring. Does not notify the kernel —
    /// call submit() to flush.
    pub fn push(&mut self, task: WorkUnit, token: u64) {
        unsafe {
            self.ring
                .submission()
                .push(&task.build_entry(token))
                .expect("submission queue full");
        }
        self.count += 1;
    }

    /// Notify the kernel of all pushed SQEs. With COOP_TASKRUN this syscall
    /// also runs any pending task_work, making completions visible in the CQ.
    pub fn submit(&mut self) -> Result<usize, Box<dyn Error>> {
        Ok(self.ring.submitter().submit()?)
    }

    /// Drain all available CQEs, calling f for each one. Decrements the
    /// in-flight count per completion.
    pub fn drain_cqes(&mut self, mut f: impl FnMut(u64, i32)) {
        let mut cq = self.ring.completion();
        cq.sync();
        for cqe in cq {
            self.count = self.count.saturating_sub(1);
            f(cqe.user_data(), cqe.result());
        }
    }

    pub fn full(&self) -> bool {
        self.count >= self.ring_cap
    }

    pub fn in_flight(&self) -> usize {
        self.count
    }
}
