import KvellModel.Types

namespace KvellModel.Worker

/-! Request machine. Read waiters share a page read. Deferred writes are ordered
by arrival ticket, can merge into a prepared write, and acknowledge only after
the worker handles the write CQE. Request payloads are owned abstract values. -/

structure Request where
  phase : RequestPhase := .fresh
  kind : Kind := .read
  page : Nat := 0
  cell : Nat := 0
  value : Bool := false
  ticket : Nat := 0
  replies : Nat := 0
  transferred : Bool := false
  answer : Image := #[]
  expected : Image := #[]
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable

structure State where
  requests : Array Request
  nextTicket : Nat := 0
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable

namespace State

def init (n : Nat) : State := ⟨Array.replicate n {}, 0⟩
def attached (r : Request) : Bool :=
  [.readWait, .writeWait, .buffered, .writing].contains r.phase
def usesPage (s : State) (p : Nat) : Bool := s.requests.any fun r => r.page == p && attached r
def hasWrites (s : State) (p : Nat) : Bool :=
  s.requests.any fun r => r.page == p && r.phase == .writeWait

def dispatchable (s : State) (t : Nat) : Bool := (s.requests[t]?).any fun r =>
  r.phase == .queued && s.requests.all (fun q => q.phase != .queued || r.ticket ≤ q.ticket)
def nextWrite (s : State) (t : Nat) : Bool := (s.requests[t]?).any fun r =>
  r.phase == .writeWait && s.requests.all fun q =>
    q.phase != .writeWait || q.page != r.page || r.ticket ≤ q.ticket

def accept (s : State) (t p : Nat) (k : Kind) (c : Nat) (v : Bool) : Option State := do
  let r ← s.requests[t]?
  guard (r.phase = .fresh)
  let r : Request := {
    phase := .queued, kind := k, page := p, ticket := s.nextTicket
    cell := if k == .write then c else 0
    value := k == .write && v }
  return { requests := s.requests.set! t r, nextTicket := s.nextTicket + 1 }

def waitForPage (s : State) (t : Nat) : Option State := do
  let r ← s.requests[t]?
  guard (s.dispatchable t)
  let r := { r with phase := if r.kind == .read then .readWait else .writeWait }
  return { s with requests := s.requests.set! t r }

def buffered (s : State) (t : Nat) : Option State := do
  let r ← s.requests[t]?
  guard (s.nextWrite t)
  return { s with requests := s.requests.set! t { r with phase := .buffered } }

def readReply (s : State) (t : Nat) (answer expected : Image) : Option State := do
  let r ← s.requests[t]?
  guard (r.kind = .read ∧ (r.phase = .queued ∨ r.phase = .readWait))
  let r := { r with phase := .done, replies := r.replies + 1, answer, expected }
  return { s with requests := s.requests.set! t r }

def finishReaders (s : State) (p : Nat) (answer expected : Image) : State :=
  { s with requests := s.requests.map fun r =>
    if r.page == p && r.phase == .readWait then
      { r with phase := .done, replies := r.replies + 1, answer, expected }
    else r }

def submitWrites (s : State) (p : Nat) : State :=
  { s with requests := s.requests.map fun r =>
    if r.page == p && r.phase == .buffered then { r with phase := .writing } else r }

def transferWrites (s : State) (p : Nat) : State :=
  { s with requests := s.requests.map fun r =>
    if r.page == p && r.phase == .writing then { r with transferred := true } else r }

def reapWrite (s : State) (p : Nat) (success : Bool) : State :=
  { s with requests := s.requests.map fun r =>
    if r.page == p && r.phase == .writing then
      { r with phase := if success then .done else .failed, replies := r.replies + 1 }
    else r }

def failRead (s : State) (p : Nat) : State :=
  { s with requests := s.requests.map fun r =>
    if r.page == p && (r.phase == .readWait || r.phase == .writeWait) then
      { r with phase := .failed, replies := r.replies + 1 }
    else r }

def wellFormed (s : State) : Bool := s.requests.all fun r =>
  r.replies ≤ 1 &&
  ((r.replies == 1) == (r.phase == .done || r.phase == .failed)) &&
  (r.phase != .done || if r.kind == .write then r.transferred else r.answer == r.expected)

end State

end KvellModel.Worker
