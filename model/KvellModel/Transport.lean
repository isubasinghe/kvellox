import KvellModel.Types

namespace KvellModel.Transport

/-! I/O transport machine: free → SQ → kernel → CQ → free.
Slots are io_uring user_data, not caller tokens. A context pins its buffer at
every stage. The kernel may complete slots out of submission order. -/

structure Context where
  kind : Kind
  buffer : Nat
  stage : IOStage := .prepared
  generation : Nat := 0
  image : Image := #[]
  success : Bool := false
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable

structure State where
  slots : Array (Option Context)
  sq : List Nat := []
  cq : List Nat := []
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable

namespace State

def init (n : Nat) : State := ⟨Array.replicate n none, [], []⟩
def get (s : State) (c : Nat) : Option Context := (s.slots[c]?).bind id
def pinned (s : State) (b : Nat) : Bool := s.slots.any fun c => c.any (·.buffer == b)
def loading (s : State) (b : Nat) : Bool :=
  s.slots.any fun c => c.any fun x => x.buffer == b && x.kind == .read
def frozen (s : State) (b : Nat) : Bool :=
  s.slots.any fun c => c.any fun x =>
    x.buffer == b && x.kind == .write && x.stage != .prepared

def enqueue (s : State) (c b : Nat) (kind : Kind) : Option State := do
  guard (c < s.slots.size ∧ (s.get c).isNone ∧ !s.pinned b)
  return { s with slots := s.slots.set! c (some { kind, buffer := b }), sq := s.sq ++ [c] }

/-- Submission snapshots the source after all worker-side coalescing. -/
def submit (s : State) (c generation : Nat) (image : Image) : Option State := do
  let ctx ← s.get c
  guard (s.sq.head? = some c ∧ ctx.stage = .prepared)
  let ctx := { ctx with
    stage := .submitted
    generation := if ctx.kind == .write then generation else 0
    image := if ctx.kind == .write then image else #[] }
  return { s with slots := s.slots.set! c (some ctx), sq := s.sq.tail }

def complete (s : State) (c : Nat) (success : Bool) (readImage : Image) : Option State := do
  let ctx ← s.get c
  guard (ctx.stage = .submitted)
  let ctx := { ctx with
    stage := .completed
    success := success
    image := if ctx.kind == .read && success then readImage else ctx.image }
  return { s with
    slots := s.slots.set! c (some ctx)
    cq := s.cq ++ [c] }

def reap (s : State) (c : Nat) : Option State := do
  let ctx ← s.get c
  guard (s.cq.head? = some c ∧ ctx.stage = .completed)
  return { s with slots := s.slots.set! c none, cq := s.cq.tail }

def wellFormed (s : State) : Bool := Id.run do
  let ids := s.slots.toList.filterMap id |>.map (·.buffer)
  if ids.eraseDups.length != ids.length then return false
  if s.sq.eraseDups != s.sq || s.cq.eraseDups != s.cq then return false
  if !(s.sq ++ s.cq).all (· < s.slots.size) then return false
  return (List.range s.slots.size).all fun c =>
    (s.sq.contains c == ((s.get c).any (·.stage == .prepared))) &&
    (s.cq.contains c == ((s.get c).any (·.stage == .completed)))

end State

end KvellModel.Transport
