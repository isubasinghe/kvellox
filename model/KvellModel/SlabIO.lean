import KvellModel.Event
import KvellModel.Safety

/-! Veil composition of the worker, cache, transport, and kernel machines.
Read the individual machines for their local transitions, Protocol for the
handshakes, and Safety for cross-machine properties. See docs/slab_io_model.md.
-/

veil module KvellModel.SlabIO

param pages : Nat
param buffers : Nat
param slots : Nat
param requests : Nat
param cells : Nat

individual system : Protocol.State
set_option veil.deferVCGeneration true
#gen_state

after_init {
  system := Protocol.init { pages, buffers, slots, requests, cells }
}

-- A product-machine step: each event atomically performs its component
-- transitions. Failed guards disable the event without changing any state.
action step (e : Event pages buffers slots requests cells) {
  let next := e.run system
  require next.isSome
  system := next.getD system
}

safety [cache_local] system.cache.wellFormed
safety [transport_local] system.io.wellFormed
safety [request_local] system.worker.wellFormed
safety [buffer_ownership] Safety.ownership system
safety [page_contents] Safety.dataCoherent system
safety [dma_source_frozen] Safety.frozenImages system
safety [waiter_ownership] Safety.waitersOwned system
termination [quiescent] Safety.quiescent system

#gen_spec

-- Full reachable-state search, not a depth-bounded search. One context is
-- deliberately enough for a cold write's read → write continuation.
#model_check interpreted
  { pages := 1, buffers := 1, slots := 1, requests := 1, cells := 2 } {}
  (sequential := true)

-- Two callers exercise read deduplication, deferred-write coalescing, and a
-- second write arriving while the first image belongs to the kernel.
#model_check interpreted
  { pages := 1, buffers := 1, slots := 1, requests := 2, cells := 2 } {}
  (sequential := true)

-- Different pages compete for one cache buffer and reuse its context slot.
#model_check interpreted
  { pages := 2, buffers := 1, slots := 1, requests := 2, cells := 1 } {}
  (sequential := true)

-- Two contexts allow kernel completions in the opposite order to submissions.
#model_check interpreted
  { pages := 2, buffers := 2, slots := 2, requests := 2, cells := 1 } {}
  (sequential := true)

end KvellModel.SlabIO
