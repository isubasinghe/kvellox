import Veil

namespace KvellModel

abbrev Image := Array Bool

inductive Kind where
  | read | write
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable, Veil.FinEncodable, Veil.Enumeration

inductive RequestPhase where
  | fresh | queued | readWait | writeWait | buffered | writing | done | failed
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable, Veil.FinEncodable

inductive IOStage where
  | prepared | submitted | completed
  deriving Inhabited, Repr, BEq, DecidableEq, Lean.ToJson, Hashable, Veil.FinEncodable

/-- IDs in the component machines index finite arrays. The Veil boundary uses
`Fin` arguments, and every component transition checks its input index. -/
structure Bounds where
  pages : Nat
  buffers : Nat
  slots : Nat
  requests : Nat
  cells : Nat
  deriving Inhabited, Repr

end KvellModel
