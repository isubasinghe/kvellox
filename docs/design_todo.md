# kvell-rs Design TODO

## Overview

One engine thread per slab. The API layer routes requests to the correct engine thread
via a pair of SPSC ring buffers. Each engine thread owns its slab file, its io_uring
instance, and its page cache. No cross-thread coordination on the hot path.

```
API Thread(s)
    │
    │  key hash → slab_id
    ▼
┌─────────────────────────────────────────────┐
│  request_queue: SlabRingBuffer<Request>     │  API produces, engine consumes
└─────────────────────────────────────────────┘
    │
    ▼
Engine Thread N
  ├── io_uring instance (SQPOLL)
  ├── Slab file (registered fd)
  ├── Ring buffer slab (registered buffers)
  └── S3Fifo page cache
    │
    ▼
┌─────────────────────────────────────────────┐
│  response_queue: SlabRingBuffer<Response>   │  Engine produces, API consumes
└─────────────────────────────────────────────┘
    │
    ▼
API Thread(s)  (matches response via token)
```

---

## 1. SPSC Ring Buffer (`src/spsc.rs`)

The ring buffer is a single contiguous allocation of `N * slot_size` bytes. Slot size
must be a power of two. Slot access uses a bitshift instead of modulo:

```
slot_ptr(idx) = base + ((idx & mask) << slot_shift)
```

This same memory region is registered with io_uring as a fixed buffer, so DMA from
NVMe lands directly into a response slot with no copy.

### Tasks

- [ ] Define `SlabRingBuffer` struct
  - `base: NonNull<u8>` — base of contiguous allocation
  - `slot_shift: u32` — log2(slot_size), used for `<<` instead of `*`
  - `mask: usize` — capacity - 1, capacity must be power of 2
  - `head: CachePadded<AtomicUsize>` — written by producer only
  - `tail_cache: CachePadded<UnsafeCell<usize>>` — stale tail copy on producer cache line
  - `tail: CachePadded<AtomicUsize>` — written by consumer only
  - `head_cache: CachePadded<UnsafeCell<usize>>` — stale head copy on consumer cache line

- [ ] Allocate backing memory with `mmap`
  - Use `MAP_ANONYMOUS | MAP_PRIVATE`
  - Use `MAP_HUGETLB` where available — a 256-slot × 4096-byte ring = 1MB fits in one 2MB huge page
  - Fall back to regular mmap if huge pages unavailable

- [ ] Implement `push(&self, f: impl FnOnce(*mut u8))` — producer writes into slot via callback
  - Load own `head` with `Relaxed`
  - Check stale `tail_cache` first; only fetch real `tail` with `Acquire` if apparently full
  - Invoke callback with `slot_ptr(head)` so caller writes directly into the slot
  - Store `head + 1` with `Release`

- [ ] Implement `pop(&self, f: impl FnOnce(*const u8))` — consumer reads from slot via callback
  - Load own `tail` with `Relaxed`
  - Check stale `head_cache` first; only fetch real `head` with `Acquire` if apparently empty
  - Invoke callback with `slot_ptr(tail)` so caller reads directly from the slot
  - Store `tail + 1` with `Release`

- [ ] Expose `as_iovec(&self) -> libc::iovec` for io_uring buffer registration
  - Returns iovec covering the full `N * slot_size` allocation

- [ ] Add `is_empty` and `is_full` checks
- [ ] Implement `Drop` to munmap the allocation
- [ ] Mark `Send` but not `Sync` — producer and consumer must be on separate threads

---

## 2. Request and Response Types (`src/io/types.rs`)

### Tasks

- [ ] Define `Request` enum
  ```rust
  pub enum Request {
      Get    { token: u64, key_len: u32 },   // key bytes follow in slot
      Put    { token: u64, key_len: u32, value_len: u32 },  // key+value bytes follow
      Delete { token: u64, key_len: u32 },
  }
  ```
  Store the discriminant + header at the start of the slot, data inline after it.
  Slot size must be large enough to hold the largest anticipated key+value pair.

- [ ] Define `Response` enum
  ```rust
  pub enum Response {
      Value { token: u64, value_len: u32 },  // value bytes follow in slot
      Ack   { token: u64 },
      Miss  { token: u64 },
      Err   { token: u64, code: u32 },
  }
  ```

- [ ] Define token generation strategy
  - Simple monotonic `AtomicU64` per API thread, or
  - High bits = thread_id, low bits = sequence number (avoids collisions across threads)

---

## 3. In-Memory Index (`src/index.rs`)

Each slab owns its own index. No cross-slab index lookups. The index maps a key to its
location on disk: `key → (page_idx: u64, slot_within_page: u16)`.

### Tasks

- [ ] Choose index structure per slab
  - `HashMap<Vec<u8>, PageSlot>` for point lookups (simplest)
  - `BTreeMap<Vec<u8>, PageSlot>` if range scan support is needed (KVell paper uses this)
  - Consider a custom hash map with inline key storage to avoid Vec allocation per key

- [ ] Define `PageSlot`
  ```rust
  pub struct PageSlot {
      pub page_idx: u64,
      pub offset:   u32,   // byte offset within the 4KB page
  }
  ```

- [ ] Implement `lookup(key) -> Option<PageSlot>`
- [ ] Implement `insert(key, slot)`
- [ ] Implement `remove(key) -> bool`

---

## 4. Slab File (`src/storage/slab.rs`)

Each slab is a single flat file on disk divided into 4KB pages. A slab is owned
exclusively by one engine thread — no locking required.

### Tasks

- [ ] Open/create slab file with `O_DIRECT | O_RDWR | O_CREAT`
  - `O_DIRECT` required for io_uring fixed-buffer DMA (bypasses page cache)
  - Pre-allocate file size with `fallocate` to avoid fragmentation

- [ ] Implement page layout
  - Page 0: slab metadata (magic number, slab_id, entry count, etc.)
  - Pages 1..N: data pages, each 4096 bytes
  - Within a data page: packed entries (EntryHeader + key + value), entries do not span pages

- [ ] Implement `allocate_page() -> u64` — returns next free page index
- [ ] Implement `page_offset(page_idx: u64) -> u64` — `page_idx * PAGE_SIZE`
- [ ] Store the raw `RawFd` for io_uring file registration

---

## 5. Engine Thread (`src/io/engine.rs`)

One engine thread per slab. Runs a tight loop: drain request queue, issue io_uring
operations, reap completions, push responses.

### Tasks

- [ ] Build `Engine` struct
  ```rust
  pub struct Engine {
      ring:           IoUring,
      slab:           Slab,
      index:          Index,
      page_cache:     S3Fifo<u64, Page>,
      request_queue:  Arc<SlabRingBuffer>,   // consumer side
      response_queue: Arc<SlabRingBuffer>,   // producer side
  }
  ```

- [ ] Initialize io_uring with `IORING_SETUP_SQPOLL`
  - Set `sq_thread_idle` to control how long kernel thread spins before sleeping
  - Call `register_files(&[slab.fd])` at startup
  - Call `register_buffers(&[response_queue.as_iovec()])` at startup

- [ ] Implement engine loop
  1. Drain `request_queue` into a local batch (avoid holding the queue while doing I/O)
  2. For each request:
     - Check `page_cache` first (cache hit → push response immediately, no I/O)
     - Cache miss → build `ReadFixed` SQE pointing at a response queue slot
     - Embed `token` in `user_data` of the SQE
  3. Submit batch to io_uring (`sync()` if SQPOLL, `submit()` otherwise)
  4. Reap CQEs, push responses to `response_queue`

- [ ] Handle write path
  - `Put`: write entry into an in-memory page, mark page dirty
  - Periodically flush dirty pages to disk via `WriteFixed` SQEs
  - Update index after successful flush

- [ ] Handle `Delete` via tombstone flag in `EntryHeader`

- [ ] Spawn engine thread pinned to a specific CPU core
  - Use `core_affinity` crate or `libc::sched_setaffinity`

---

## 6. API Layer (`src/lib.rs` or `src/api.rs`)

The outward-facing interface. Routes requests to the correct engine thread by hashing
the key to a slab_id. Waits for the matching response token.

### Tasks

- [ ] Define `KvStore` struct
  ```rust
  pub struct KvStore {
      engines: Vec<EngineSide>,   // one per slab
      num_slabs: usize,
  }

  struct EngineSide {
      request_queue:  Arc<SlabRingBuffer>,
      response_queue: Arc<SlabRingBuffer>,
  }
  ```

- [ ] Implement key routing
  ```rust
  fn slab_for_key(key: &[u8], num_slabs: usize) -> usize {
      // FxHash or xxHash of key, mod num_slabs
  }
  ```

- [ ] Implement `get(&self, key: &[u8]) -> Option<Vec<u8>>`
  - Route to slab, push `Request::Get` into request queue
  - Spin on response queue waiting for matching token
  - Extract value bytes from response slot

- [ ] Implement `put(&self, key: &[u8], value: &[u8])`
- [ ] Implement `delete(&self, key: &[u8])`

- [ ] Consider async API (`async fn get`) for callers using tokio/async-std
  - The spin-wait on response queue can be replaced with a waker registered in the token

---

## 7. Startup and Configuration

- [ ] Define `Config` struct
  - `num_slabs: usize` — should match number of NVMe queues / CPU cores
  - `slab_dir: PathBuf` — directory for slab files
  - `ring_capacity: usize` — number of slots per SPSC queue (power of 2)
  - `slot_size: usize` — bytes per slot (power of 2, must fit largest KV pair + header)
  - `page_cache_size: usize` — pages per slab cache

- [ ] Implement `KvStore::open(config: Config) -> Result<KvStore>`
  - Create/open slab files
  - Allocate SPSC ring buffers
  - Spawn and pin engine threads

---

## 8. Missing Dependencies to Add

```toml
libc        = "0.2"     # mmap, iovec, sched_setaffinity
anyhow      = "1"       # error handling in engine
```

Optional but recommended:
```toml
core_affinity = "0.8"   # ergonomic CPU pinning
rustix        = "0.38"  # safer syscall wrappers (alternative to libc)
```
