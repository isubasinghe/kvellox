import KvellModel.Types

namespace KvellModel.Kernel

/-! File image seen by I/O, not crash-durable storage. Reads capture a whole
image; writes may transfer individual cells before a failed/short CQE. -/

structure State where
  disk : Array Image
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable

namespace State

def init (pages cells : Nat) : State :=
  ⟨Array.replicate pages ((List.range cells).toArray.map fun c => c % 2 == 1)⟩

def read (s : State) (p : Nat) : Option Image := s.disk[p]?

def write (s : State) (p : Nat) (image : Image) : Option State := do
  let old ← s.disk[p]?
  guard (old.size = image.size)
  return ⟨s.disk.set! p image⟩

def writeCell (s : State) (p c : Nat) (image : Image) : Option State := do
  let old ← s.disk[p]?
  let value ← image[c]?
  guard (c < old.size)
  return ⟨s.disk.set! p (old.set! c value)⟩

end State

end KvellModel.Kernel
