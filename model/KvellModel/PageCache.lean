import KvellModel.Types

namespace KvellModel.PageCache

/-! Cache machine: absent → loading → clean ⇄ dirty → absent.
Pinning belongs to Transport; the composition guards mutation and eviction.
S3-FIFO ranking is abstracted to an arbitrary eligible eviction candidate. -/

structure Page where
  id : Nat
  ready : Bool := false
  dirty : Bool := false
  generation : Nat := 0
  data : Image
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable

structure State where
  buffers : Array (Option Page)
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable

namespace State

def init (n : Nat) : State := ⟨Array.replicate n none⟩
def get (s : State) (b : Nat) : Option Page := (s.buffers[b]?).bind id
def contains (s : State) (p : Nat) : Bool := s.buffers.any (fun x => x.any (·.id == p))

def reserve (s : State) (b p cells : Nat) : Option State := do
  guard (b < s.buffers.size ∧ (s.get b).isNone ∧ !s.contains p)
  return ⟨s.buffers.set! b (some { id := p, data := Array.replicate cells false })⟩

def load (s : State) (b : Nat) (image : Image) : Option State := do
  let p ← s.get b
  guard (!p.ready ∧ image.size = p.data.size)
  return ⟨s.buffers.set! b (some { p with ready := true, data := image })⟩

def patch (s : State) (b c : Nat) (value : Bool) : Option State := do
  let p ← s.get b
  guard (p.ready ∧ c < p.data.size)
  let p := { p with
    data := p.data.set! c value
    dirty := true
    generation := p.generation + 1 }
  return ⟨s.buffers.set! b (some p)⟩

def markClean (s : State) (b generation : Nat) : Option State := do
  let p ← s.get b
  guard p.ready
  return ⟨s.buffers.set! b (some { p with
    dirty := if p.generation = generation then false else p.dirty })⟩

def evict (s : State) (b : Nat) : Option State := do
  let p ← s.get b
  guard (p.ready ∧ !p.dirty)
  return ⟨s.buffers.set! b none⟩

/-- Failed prerequisite reads have never produced usable contents. -/
def discardLoad (s : State) (b : Nat) : Option State := do
  let p ← s.get b
  guard (!p.ready ∧ !p.dirty)
  return ⟨s.buffers.set! b none⟩

def wellFormed (s : State) : Bool :=
  (s.buffers.all fun p => p.all fun x => !x.dirty || x.ready) &&
  (s.buffers.toList.filterMap id |>.map (·.id)).eraseDups.length ==
    (s.buffers.toList.filterMap id).length

end State

end KvellModel.PageCache
