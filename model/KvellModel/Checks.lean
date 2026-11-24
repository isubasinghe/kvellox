import KvellModel.Safety

namespace KvellModel.Checks
open Protocol

/-! Named, non-vacuous executions of the composed machines. Every step checks
all local and cross-machine invariants. A disabled step or failed expectation
fails `lake build KvellModel.Checks`. Veil's exhaustive checks are in SlabIO. -/

abbrev Step := String × (Protocol.State → Option Protocol.State)

def expect (name : String) (p : Protocol.State → Bool) : Step :=
  (name, fun s => if p s then some s else none)

def blocked (name : String) (f : Protocol.State → Option Protocol.State) : Step :=
  expect name (fun s => (f s).isNone)

def run (name : String) (bounds : Bounds) (steps : List Step) : IO Unit := do
  let mut s := Protocol.init bounds
  for (label, f) in steps do
    let some next := f s | throw (IO.userError s!"{name}: disabled/failed step: {label}")
    unless Safety.all next do
      throw (IO.userError s!"{name}: invariant violation after {label}: {repr next}")
    s := next
  IO.println s!"PASS {name} ({steps.length} steps/assertions)"

def onePage : Bounds := ⟨1, 1, 1, 4, 2⟩

#eval run "cold write, CQE acknowledgment, eviction and reread" onePage [
  ("write", fun s => accept s 0 0 .write 0 true),
  ("miss", fun s => dispatchMiss s 0 0 0),
  ("submit read", fun s => submit s 0),
  ("read completion", fun s => kernelComplete s 0 true),
  expect "DMA read has not acknowledged write" (fun s => s.worker.requests[0]!.replies == 0),
  ("chain read to write", fun s => chainReadToWrite s 0 0),
  expect "patch preserves other cell" (fun s => (s.cache.get 0).any (·.data == #[true, true])),
  expect "chaining is not a write acknowledgment" (fun s => s.worker.requests[0]!.replies == 0),
  blocked "pinned/dirty buffer cannot be evicted" (fun s => evict s 0),
  ("submit write", fun s => submit s 0),
  ("kernel write completion", fun s => kernelComplete s 0 true),
  expect "write CQE must be consumed" (fun s => s.worker.requests[0]!.replies == 0),
  ("handle write CQE", fun s => reapWrite s 0),
  expect "one success response" (fun s => s.worker.requests[0]!.phase == .done),
  blocked "cannot handle the same CQE twice" (fun s => reapWrite s 0),
  ("evict", fun s => evict s 0),
  ("read back", fun s => accept s 1 0 .read 0 false),
  ("reuse buffer and context", fun s => dispatchMiss s 1 0 0),
  ("submit reread", fun s => submit s 0),
  ("complete reread", fun s => kernelComplete s 0 true),
  ("handle reread", fun s => reapRead s 0),
  expect "read back persisted page image" (fun s => s.worker.requests[1]!.answer == #[true, true])
]

#eval run "read deduplication and multiple deferred writes" onePage [
  ("reader one", fun s => accept s 0 0 .read 0 false),
  ("first read", fun s => dispatchMiss s 0 0 0),
  ("writer one", fun s => accept s 1 0 .write 0 true),
  ("defer writer one", fun s => dispatchCached s 1 0),
  ("reader two", fun s => accept s 2 0 .read 0 false),
  ("join read", fun s => dispatchCached s 2 0),
  ("writer two", fun s => accept s 3 0 .write 1 false),
  ("defer writer two", fun s => dispatchCached s 3 0),
  expect "only one read SQE" (fun s => s.io.sq == [0]),
  ("submit read", fun s => submit s 0),
  ("complete read", fun s => kernelComplete s 0 true),
  blocked "cannot reorder deferred writes" (fun s => chainReadToWrite s 0 3),
  ("chain first patch", fun s => chainReadToWrite s 0 1),
  expect "readers see loaded page" (fun s =>
    s.worker.requests[0]!.answer == #[false, true] && s.worker.requests[2]!.answer == #[false, true]),
  ("coalesce second patch before submit", fun s => applyWrite s 3 0),
  expect "still one write SQE" (fun s => s.io.sq == [0]),
  ("submit merged image", fun s => submit s 0),
  ("complete merged write", fun s => kernelComplete s 0 true),
  ("acknowledge both writers", fun s => reapWrite s 0),
  expect "both patches and all four responses" (fun s =>
    s.kernel.disk[0]! == #[true, false] && s.worker.requests.all (·.replies == 1))
]

#eval run "write arriving while buffer belongs to kernel" onePage [
  ("first writer", fun s => accept s 0 0 .write 0 true),
  ("read miss", fun s => dispatchMiss s 0 0 0),
  ("submit read", fun s => submit s 0),
  ("read done", fun s => kernelComplete s 0 true),
  ("chain", fun s => chainReadToWrite s 0 0),
  ("submit write", fun s => submit s 0),
  ("second writer", fun s => accept s 1 0 .write 1 false),
  ("defer second writer", fun s => dispatchCached s 1 0),
  blocked "no mutation during DMA" (fun s => applyWrite s 1 0),
  ("kernel completed", fun s => kernelComplete s 0 true),
  blocked "pin lasts until CQE handling" (fun s => applyWrite s 1 0),
  ("first CQE", fun s => reapWrite s 0),
  expect "old CQE does not reply to new writer" (fun s => s.worker.requests[1]!.replies == 0),
  ("apply new generation", fun s => applyWrite s 1 0),
  ("prepare new write", fun s => prepareWrite s 0 0),
  ("submit new write", fun s => submit s 0),
  ("complete new write", fun s => kernelComplete s 0 true),
  ("new CQE", fun s => reapWrite s 0),
  expect "new generation survives old completion" (fun s => s.kernel.disk[0]! == #[true, false])
]

#eval run "failed prerequisite read fails every attached waiter" onePage [
  ("reader", fun s => accept s 0 0 .read 0 false),
  ("miss", fun s => dispatchMiss s 0 0 0),
  ("writer", fun s => accept s 1 0 .write 0 true),
  ("defer writer", fun s => dispatchCached s 1 0),
  ("submit", fun s => submit s 0),
  ("short or failed read", fun s => kernelComplete s 0 false),
  blocked "cannot chain after failure" (fun s => chainReadToWrite s 0 1),
  ("fail waiters", fun s => reapReadError s 0),
  expect "no write queued and buffer released" (fun s =>
    s.io.sq.isEmpty && (s.cache.get 0).isNone &&
    s.worker.requests[0]!.phase == .failed && s.worker.requests[1]!.phase == .failed)
]

#eval run "partial write failure and internal retry" onePage [
  ("writer", fun s => accept s 0 0 .write 0 true),
  ("miss", fun s => dispatchMiss s 0 0 0),
  ("submit read", fun s => submit s 0),
  ("read done", fun s => kernelComplete s 0 true),
  ("chain", fun s => chainReadToWrite s 0 0),
  ("submit write", fun s => submit s 0),
  ("partial transfer", fun s => kernelWriteCell s 0 0),
  ("failed CQE", fun s => kernelComplete s 0 false),
  ("handle failure", fun s => reapWrite s 0),
  expect "dirty data retained and caller failed" (fun s =>
    (s.cache.get 0).any (·.dirty) && s.worker.requests[0]!.phase == .failed),
  blocked "dirty data cannot be evicted" (fun s => evict s 0),
  ("retry", fun s => prepareWrite s 0 0),
  ("submit retry", fun s => submit s 0),
  ("retry succeeded", fun s => kernelComplete s 0 true),
  ("handle retry CQE", fun s => reapWrite s 0),
  expect "retry cleans page without a second response" (fun s =>
    (s.cache.get 0).any (! ·.dirty) && s.worker.requests[0]!.replies == 1 &&
    s.worker.requests[0]!.phase == .failed)
]

#eval run "out-of-order kernel completion, FIFO SQ/CQ" ⟨2, 2, 2, 2, 2⟩ [
  ("page zero read", fun s => accept s 0 0 .read 0 false),
  ("page zero miss", fun s => dispatchMiss s 0 0 0),
  ("page one write", fun s => accept s 1 1 .write 0 true),
  ("page one miss", fun s => dispatchMiss s 1 1 1),
  blocked "SQ order is preserved" (fun s => submit s 1),
  ("submit zero", fun s => submit s 0),
  ("submit one", fun s => submit s 1),
  ("one finishes first", fun s => kernelComplete s 1 true),
  ("zero finishes second", fun s => kernelComplete s 0 true),
  blocked "CQ order follows completion order" (fun s => reapRead s 0),
  ("chain one", fun s => chainReadToWrite s 1 1),
  ("reap zero", fun s => reapRead s 0),
  ("submit page one write", fun s => submit s 1),
  ("finish page one write", fun s => kernelComplete s 1 true),
  ("reap page one write", fun s => reapWrite s 1),
  expect "both requests completed exactly once" (fun s => s.worker.requests.all (·.replies == 1))
]

end KvellModel.Checks
