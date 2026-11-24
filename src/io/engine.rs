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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    const PROFILE_DEPTH: usize = 32;

    #[repr(C, align(4096))]
    #[derive(Clone)]
    struct ProfilePage([u8; PAGE_SIZE]);

    fn profile_usage() -> std::io::Result<libc::rusage> {
        let mut usage = std::mem::MaybeUninit::uninit();
        // getrusage initializes the entire output on success.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { usage.assume_init() })
    }

    fn profile_phase(
        engine: &mut Engine,
        buffers: &mut [ProfilePage],
        buf_index: u16,
        pages: usize,
        write: bool,
    ) -> Result<f64, Box<dyn Error>> {
        let start = Instant::now();
        for first in (0..pages).step_by(PROFILE_DEPTH) {
            let count = (pages - first).min(PROFILE_DEPTH);
            for (slot, buffer) in buffers[..count].iter_mut().enumerate() {
                let page = first + slot;
                let offset = (page * PAGE_SIZE) as u64;
                let task = if write {
                    buffer.0.fill((page % 251) as u8);
                    buffer.0[..8].copy_from_slice(&(page as u64).to_le_bytes());
                    WorkUnit::WriteFile(WriteFile {
                        file_index: 0,
                        buf: buffer.0.as_ptr(),
                        offset,
                        buf_index,
                    })
                } else {
                    buffer.0.fill(0xff);
                    WorkUnit::ReadFile(ReadFile {
                        file_index: 0,
                        buf: buffer.0.as_mut_ptr(),
                        offset,
                        buf_index,
                    })
                };
                engine.push(task, page as u64);
            }

            engine.submit()?;
            let mut completed = [false; PROFILE_DEPTH];
            while engine.in_flight() != 0 {
                engine.drain_cqes(|token, result| {
                    assert_eq!(result, PAGE_SIZE as i32, "short or failed I/O: {token}");
                    assert!((first as u64..(first + count) as u64).contains(&token));
                    let slot = token as usize - first;
                    assert!(!completed[slot], "duplicate completion: {token}");
                    completed[slot] = true;
                });
                if engine.in_flight() != 0 {
                    // Drive COOP_TASKRUN and wait without busy-polling.
                    engine.ring.submitter().submit_and_wait(1)?;
                }
            }
            assert!(completed[..count].iter().all(|done| *done));
            if !write {
                for (slot, buffer) in buffers[..count].iter().enumerate() {
                    let page = first + slot;
                    assert_eq!(&buffer.0[..8], &(page as u64).to_le_bytes());
                    assert!(buffer.0[8..].iter().all(|byte| *byte == (page % 251) as u8));
                }
            }
        }
        Ok(start.elapsed().as_secs_f64())
    }

    /// One workload, run separately for timing, syscalls, counters and kernel traces.
    /// Run `python3 scripts/profile-engine.py` for a combined report, or:
    /// `cargo test --release --lib io::engine::tests::profile_engine -- --exact --ignored --nocapture`
    ///
    /// ENGINE_PROFILE_PAGES controls file size (default 4096 x 4 KiB).
    /// ENGINE_PROFILE_DIR selects the filesystem (default target/engine-profile).
    /// ENGINE_PROFILE_DIRECT=0 opts into buffered I/O; O_DIRECT is the default.
    /// Timings include buffer preparation and read verification, but exclude setup
    /// and the separately reported sync_data. They describe this workload, not
    /// a device's maximum throughput. Buffered reads follow writes and may be cached.
    #[test]
    #[ignore = "opt-in disk profiling workload; requires io_uring and locked-buffer allowance"]
    fn profile_engine() -> Result<(), Box<dyn Error>> {
        let pages = std::env::var("ENGINE_PROFILE_PAGES")
            .unwrap_or_else(|_| "4096".into())
            .parse::<usize>()?;
        let bytes = pages.checked_mul(PAGE_SIZE).ok_or("file size overflow")?;
        if pages == 0 || bytes > i64::MAX as usize {
            return Err("ENGINE_PROFILE_PAGES must describe a positive, valid file size".into());
        }
        let direct = match std::env::var("ENGINE_PROFILE_DIRECT").as_deref() {
            Ok("0") => false,
            Ok("1") | Err(std::env::VarError::NotPresent) => true,
            _ => return Err("ENGINE_PROFILE_DIRECT must be 0 or 1".into()),
        };
        let directory = std::env::var_os("ENGINE_PROFILE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/engine-profile"));
        fs::create_dir_all(&directory)?;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = directory.join(format!("engine-{}-{nonce}.dat", std::process::id()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(if direct { libc::O_DIRECT } else { 0 })
            .open(&path)?;
        // The open descriptor remains valid; the scratch file is removed even
        // if a completion assertion fails or the profiler terminates the test.
        fs::remove_file(&path)?;
        file.set_len(bytes as u64)?;

        // Declaration order matters: the ring must drop before its buffers/file,
        // including when an assertion fails with requests still in flight.
        let mut buffers = vec![ProfilePage([0; PAGE_SIZE]); PROFILE_DEPTH];
        let mut engine = Engine::new()?;
        let buf_index =
            engine.register_buffer(buffers.as_mut_ptr().cast::<u8>(), PROFILE_DEPTH * PAGE_SIZE)?;
        engine.register_files(&[file.as_raw_fd()])?;

        let before = profile_usage()?;
        let write_seconds = profile_phase(&mut engine, &mut buffers, buf_index, pages, true)?;
        let sync_start = Instant::now();
        file.sync_data()?;
        let sync_seconds = sync_start.elapsed().as_secs_f64();
        let read_seconds = profile_phase(&mut engine, &mut buffers, buf_index, pages, false)?;
        let after = profile_usage()?;
        let seconds = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1_000_000.0;
        let mib = bytes as f64 / (1024.0 * 1024.0);

        // A stable prefix lets the report runner find JSON among libtest output.
        println!(
            "\nENGINE_PROFILE {{\"pages\":{pages},\"bytes_per_phase\":{bytes},\"queue_depth\":{PROFILE_DEPTH},\"direct_io\":{direct},\"write_seconds\":{write_seconds:.9},\"read_seconds\":{read_seconds:.9},\"sync_seconds\":{sync_seconds:.9},\"write_mib_s\":{:.3},\"read_mib_s\":{:.3},\"write_iops\":{:.3},\"read_iops\":{:.3},\"process_user_seconds\":{:.6},\"process_system_seconds\":{:.6},\"minor_faults\":{},\"major_faults\":{},\"voluntary_context_switches\":{},\"involuntary_context_switches\":{}}}",
            mib / write_seconds,
            mib / read_seconds,
            pages as f64 / write_seconds,
            pages as f64 / read_seconds,
            seconds(after.ru_utime) - seconds(before.ru_utime),
            seconds(after.ru_stime) - seconds(before.ru_stime),
            after.ru_minflt - before.ru_minflt,
            after.ru_majflt - before.ru_majflt,
            after.ru_nvcsw - before.ru_nvcsw,
            after.ru_nivcsw - before.ru_nivcsw,
        );
        Ok(())
    }
}
