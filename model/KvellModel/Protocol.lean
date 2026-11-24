import KvellModel.PageCache
import KvellModel.Transport
import KvellModel.Worker
import KvellModel.Kernel

namespace KvellModel.Protocol

/-! Composition: each function is one atomic worker/kernel event. Component
transitions return `none` when disabled; intermediate states are never exposed.
Only this module coordinates machines. Local state and transitions live in
PageCache, Transport, Worker, and Kernel. -/

structure State where
  cache : PageCache.State
  io : Transport.State
  worker : Worker.State
  kernel : Kernel.State
  visible : Array Image
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable

def init (n : Bounds) : State :=
  let kernel := Kernel.State.init n.pages n.cells
  ⟨PageCache.State.init n.buffers, Transport.State.init n.slots, Worker.State.init n.requests, kernel, kernel.disk⟩

def accept (s : State) (t p : Nat) (kind : Kind) (c : Nat) (v : Bool) : Option State := do
  let image ← s.visible[p]?
  guard (c < image.size)
  return { s with worker := (← s.worker.accept t p kind c v) }

def dispatchCached (s : State) (t b : Nat) : Option State := do
  let r ← s.worker.requests[t]?
  let p ← s.cache.get b
  guard (s.worker.dispatchable t ∧ p.id = r.page)
  let worker ← if r.kind == .read && p.ready then
    s.worker.readReply t p.data (← s.visible[p.id]?)
  else s.worker.waitForPage t
  return { s with worker }

def dispatchMiss (s : State) (t b c : Nat) : Option State := do
  let r ← s.worker.requests[t]?
  let image ← s.visible[r.page]?
  return { s with
    worker := (← s.worker.waitForPage t)
    cache := (← s.cache.reserve b r.page image.size)
    io := (← s.io.enqueue c b .read) }

def applyWrite (s : State) (t b : Nat) : Option State := do
  let r ← s.worker.requests[t]?
  let p ← s.cache.get b
  let image ← s.visible[p.id]?
  guard (p.id = r.page ∧ !s.io.frozen b)
  return { s with
    worker := (← s.worker.buffered t)
    cache := (← s.cache.patch b r.cell r.value)
    visible := s.visible.set! p.id (image.set! r.cell r.value) }

def prepareWrite (s : State) (b c : Nat) : Option State := do
  let p ← s.cache.get b
  guard (p.ready ∧ p.dirty)
  return { s with io := (← s.io.enqueue c b .write) }

def submit (s : State) (c : Nat) : Option State := do
  let ctx ← s.io.get c
  let p ← s.cache.get ctx.buffer
  return { s with
    io := (← s.io.submit c p.generation p.data)
    worker := if ctx.kind == .write then s.worker.submitWrites p.id else s.worker }

def kernelWriteCell (s : State) (c cell : Nat) : Option State := do
  let ctx ← s.io.get c
  let p ← s.cache.get ctx.buffer
  guard (ctx.stage = .submitted ∧ ctx.kind = .write)
  return { s with kernel := (← s.kernel.writeCell p.id cell ctx.image) }

def kernelComplete (s : State) (c : Nat) (success : Bool) : Option State := do
  let ctx ← s.io.get c
  let p ← s.cache.get ctx.buffer
  let image ← s.kernel.read p.id
  let io ← s.io.complete c success image
  if success && ctx.kind == .write then
    return { s with
      io := io
      kernel := (← s.kernel.write p.id ctx.image)
      worker := s.worker.transferWrites p.id }
  return { s with io }

/-- Internal successful read handling; readers see the old page before patches.
The inaccessible DMA result lives in the context until the worker validates it. -/
private def finishRead (s : State) (c : Nat) : Option State := do
  let ctx ← s.io.get c
  let p ← s.cache.get ctx.buffer
  guard (ctx.kind = .read ∧ ctx.success)
  return { s with
    io := (← s.io.reap c)
    cache := (← s.cache.load ctx.buffer ctx.image)
    worker := s.worker.finishReaders p.id ctx.image (← s.visible[p.id]?) }

def reapRead (s : State) (c : Nat) : Option State := do
  let ctx ← s.io.get c
  let p ← s.cache.get ctx.buffer
  guard (!s.worker.hasWrites p.id)
  finishRead s c

/-- Read CQE → patch → WriteFixed, atomically reusing the same context slot.
This is a worker continuation, not IOSQE_IO_LINK. No write reply is sent. -/
def chainReadToWrite (s : State) (c t : Nat) : Option State := do
  let ctx ← s.io.get c
  let s ← finishRead s c
  let s ← applyWrite s t ctx.buffer
  prepareWrite s ctx.buffer c

def reapReadError (s : State) (c : Nat) : Option State := do
  let ctx ← s.io.get c
  let p ← s.cache.get ctx.buffer
  guard (ctx.kind = .read ∧ !ctx.success)
  return { s with
    io := (← s.io.reap c)
    cache := (← s.cache.discardLoad ctx.buffer)
    worker := s.worker.failRead p.id }

def reapWrite (s : State) (c : Nat) : Option State := do
  let ctx ← s.io.get c
  let p ← s.cache.get ctx.buffer
  guard (ctx.kind = .write)
  let io ← s.io.reap c
  let cache ← if ctx.success then s.cache.markClean ctx.buffer ctx.generation else some s.cache
  return { s with io, cache, worker := s.worker.reapWrite p.id ctx.success }

def evict (s : State) (b : Nat) : Option State := do
  let p ← s.cache.get b
  guard (!s.io.pinned b ∧ !s.worker.usesPage p.id)
  return { s with cache := (← s.cache.evict b) }

end KvellModel.Protocol
