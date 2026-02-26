# io_uring Usage Guide

This document covers every io_uring call needed for kvell-rs, in the order you will
use them, with notes on why each matters for this design.

The Rust crate is `io-uring = "0.7"`. All types live in `io_uring::{opcode, types, IoUring}`.

---

## 1. Creating the Ring

```rust
use io_uring::{IoUring, types::IoPoll};

// Standard ring — explicit submit() syscall required
let ring = IoUring::new(256)?;

// SQPOLL ring — kernel thread polls SQ, no submit() syscall needed under load
let ring = IoUring::builder()
    .setup_sqpoll(2000)   // kernel thread idles for 2000ms before sleeping
    .build(256)?;         // 256 SQE slots
```

**SQPOLL notes:**
- The kernel spawns a dedicated thread that polls the submission queue.
- Under sustained load you never make a syscall to submit — just write SQEs and `sync()`.
- After the idle timeout the kernel thread sleeps. The next `submit()` call wakes it.
  Check `submitter().sq_need_wakeup()` to avoid unnecessary wakeup syscalls:
  ```rust
  if ring.submitter().sq_need_wakeup() {
      ring.submitter().submit()?;
  }
  ```
- Requires `CAP_SYS_NICE` or a recent kernel that relaxed the capability requirement.
- The kernel thread will pin to the same CPU as the engine thread if you call
  `setup_sqpoll_cpu(core_id)` on the builder.

**Queue depth (256):**
Choose a depth that covers your maximum in-flight I/O operations. 256 is a reasonable
starting point for one slab thread. Must be a power of two.

---

## 2. Registering the Slab File (`register_files`)

Registers file descriptors with the kernel so io_uring can cache them internally.
Eliminates the fd table lookup on every operation.

```rust
use std::os::unix::io::AsRawFd;

let slab_fd = slab_file.as_raw_fd();
ring.submitter().register_files(&[slab_fd])?;
```

After registration, refer to the file by its **index** in the registered array (0 in
this case) using `types::Fixed(0)` instead of `types::Fd(slab_fd)`.

**To update a registered fd** (e.g. if you reopen the slab file):
```rust
ring.submitter().register_files_update(0, &[new_fd])?;
//                                     ^
//                                     offset into the registered fd array
```

**To unregister all files:**
```rust
ring.submitter().unregister_files()?;
```

---

## 3. Registering the Ring Buffer (`register_buffers`)

Registers memory regions with the kernel. The kernel pins those pages once at
registration time. Subsequent `ReadFixed`/`WriteFixed` operations skip the per-call
page pinning that normal `Read`/`Write` requires.

This is the zero-copy path: DMA from NVMe lands directly into a response ring buffer
slot with no bounce buffer and no copy.

```rust
use io_uring::types::iovec; // re-exported from libc

let iov = libc::iovec {
    iov_base: ring_buffer.base.as_ptr() as *mut libc::c_void,
    iov_len:  ring_buffer.total_size(),  // N * slot_size bytes
};

// SAFETY: the memory must remain valid and pinned for the lifetime of the registration
ring.submitter().register_buffers(&[iov])?;
```

After registration, refer to this buffer by its **index** (0) in the `buf_index`
field of `ReadFixed`/`WriteFixed` SQEs.

**Important:** The registered memory must not be freed or remapped while the
registration is active. Unregister before dropping the ring buffer:
```rust
ring.submitter().unregister_buffers()?;
```

**Multiple buffers:** You can register multiple separate memory regions. Pass a slice
of `iovec` structs. Each gets its own index (0, 1, 2...). For kvell-rs, one buffer
covering the entire ring buffer slab is sufficient.

---

## 4. Issuing a Fixed Read (`ReadFixed`)

Read from the registered slab file directly into a registered ring buffer slot.

```rust
use io_uring::{opcode, types};

let slot_ptr: *mut u8 = response_queue.slot_ptr(slot_idx);
let disk_offset: u64  = slab.page_offset(page_idx);

let sqe = opcode::ReadFixed::new(
    types::Fixed(0),          // registered file index (not the raw fd)
    slot_ptr,                 // destination — must be within the registered buffer
    PAGE_SIZE as u32,         // bytes to read
    0,                        // registered buffer index
)
.offset(disk_offset)          // byte offset within the file
.user_data(token)             // u64 echoed back in the CQE — use for token matching
.build();

// Push to submission queue
// SAFETY: slot_ptr and disk_offset must be valid for the duration of the operation
unsafe {
    ring.submission().push(&sqe)?;
}
```

**`user_data`:** This u64 is echoed verbatim in the completion queue entry. Store the
request token here so you can match completions to waiting callers without a separate
lookup table.

**Alignment:** With `O_DIRECT`, the offset and length must be aligned to the logical
block size of the device (typically 512 or 4096 bytes). Using `PAGE_SIZE` (4096) for
both slot size and read size satisfies this automatically.

---

## 5. Issuing a Fixed Write (`WriteFixed`)

Write a dirty page from a ring buffer slot (or any registered buffer region) to the
slab file.

```rust
let slot_ptr: *const u8 = request_queue.slot_ptr(slot_idx); // or a page buffer ptr
let disk_offset: u64    = slab.page_offset(page_idx);

let sqe = opcode::WriteFixed::new(
    types::Fixed(0),          // registered file index
    slot_ptr,                 // source — must be within the registered buffer
    PAGE_SIZE as u32,         // bytes to write
    0,                        // registered buffer index
)
.offset(disk_offset)
.user_data(token)
.build();

unsafe {
    ring.submission().push(&sqe)?;
}
```

---

## 6. Submitting Operations

After pushing SQEs onto the submission queue, make them visible to the kernel.

```rust
// With SQPOLL: sync() makes SQEs visible to the kernel polling thread (no syscall)
ring.submission().sync();

// Without SQPOLL, or to wake a sleeping SQPOLL thread:
ring.submitter().submit()?;

// Submit and wait for at least N completions (useful for backpressure):
ring.submitter().submit_and_wait(1)?;
```

**Batching pattern for the engine loop:**
```rust
// 1. Drain the request queue and push all SQEs first
for request in drain_request_queue() {
    unsafe { ring.submission().push(&build_sqe(&request))?; }
}

// 2. Then submit the whole batch in one shot
ring.submission().sync();
if ring.submitter().sq_need_wakeup() {
    ring.submitter().submit()?;
}
```

This is the core throughput multiplier: one syscall (or zero with SQPOLL) for N
operations.

---

## 7. Reaping Completions (CQE)

```rust
// Non-blocking: process whatever completions are ready right now
ring.completion().sync();  // make new CQEs visible from kernel side

for cqe in ring.completion() {
    let token  = cqe.user_data();   // the u64 you put in user_data when building the SQE
    let result = cqe.result();      // bytes transferred on success, negative errno on error

    if result < 0 {
        // I/O error — result is a negative errno value
        let err = std::io::Error::from_raw_os_error(-result);
        push_error_response(token, err);
    } else {
        // Success — result is bytes read/written
        push_success_response(token, result as usize);
    }
}
```

**Blocking wait for completions:**
```rust
// Block until at least 1 completion is available
ring.submitter().submit_and_wait(1)?;
ring.completion().sync();
```

**CQE result values:**
- `>= 0`: number of bytes transferred
- `< 0`: negated `errno` (e.g. `-libc::EBADF`, `-libc::EIO`)
- Partial reads/writes are possible — `result < requested_len` — handle them

---

## 8. Full Engine Loop Skeleton

```rust
fn run(&mut self) {
    loop {
        // --- Phase 1: drain request queue, build SQEs ---
        let mut submitted = 0;
        while let Some(req) = self.request_queue.pop() {
            // Check page cache first
            if let Some(page) = self.page_cache.get(&req.page_idx) {
                // Cache hit: copy value out, push response, skip I/O
                self.push_response(req.token, page);
                continue;
            }

            // Cache miss: issue ReadFixed into a response slot
            let slot_ptr = self.response_queue.reserve_slot();
            let sqe = opcode::ReadFixed::new(
                types::Fixed(0),
                slot_ptr,
                PAGE_SIZE as u32,
                0,
            )
            .offset(self.slab.page_offset(req.page_idx))
            .user_data(req.token)
            .build();

            unsafe { self.ring.submission().push(&sqe).expect("SQ full"); }
            submitted += 1;
        }

        // --- Phase 2: submit batch ---
        if submitted > 0 {
            self.ring.submission().sync();
            if self.ring.submitter().sq_need_wakeup() {
                self.ring.submitter().submit().unwrap();
            }
        }

        // --- Phase 3: reap completions ---
        self.ring.completion().sync();
        for cqe in self.ring.completion() {
            let token = cqe.user_data();
            if cqe.result() < 0 {
                self.push_error(token, cqe.result());
            } else {
                self.commit_response_slot(token);
            }
        }
    }
}
```

---

## 9. Teardown Order

Order matters — unregister before closing the ring.

```rust
ring.submitter().unregister_buffers()?;  // unpin ring buffer pages
ring.submitter().unregister_files()?;    // release registered fds
drop(ring);                              // closes the io_uring fd
drop(slab_file);                         // now safe to close the slab fd
drop(ring_buffer);                       // now safe to munmap
```

---

## 10. Quick Reference

| Call | Purpose | Syscall? |
|---|---|---|
| `IoUring::builder().setup_sqpoll().build(n)` | Create ring with kernel polling thread | Yes (once) |
| `submitter().register_files(&[fd])` | Cache slab fd in kernel | Yes (once) |
| `submitter().register_buffers(&[iov])` | Pin ring buffer, enable zero-copy DMA | Yes (once) |
| `opcode::ReadFixed::new(Fixed(0), ptr, len, buf_idx).offset(off).user_data(tok)` | Build NVMe read SQE | No |
| `opcode::WriteFixed::new(Fixed(0), ptr, len, buf_idx).offset(off).user_data(tok)` | Build NVMe write SQE | No |
| `submission().push(&sqe)` | Enqueue SQE | No |
| `submission().sync()` | Make SQEs visible to SQPOLL thread | No |
| `submitter().sq_need_wakeup()` + `submit()` | Wake sleeping SQPOLL thread | Conditional |
| `completion().sync()` | Make new CQEs visible | No |
| `cqe.user_data()` / `cqe.result()` | Read token and result from CQE | No |
| `submitter().unregister_buffers()` | Unpin registered memory | Yes |
| `submitter().unregister_files()` | Release registered fds | Yes |
