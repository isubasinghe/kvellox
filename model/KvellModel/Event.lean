import KvellModel.Safety

namespace KvellModel

/-! Finite event alphabet: bounds constrain event choices, not execution length.
Disabled events yield `none`; Veil explores enabled events in every order. -/
inductive Event (pages buffers slots requests cells : Nat) where
  | idle
  | accept (t : Fin requests) (p : Fin pages) (kind : Kind) (c : Fin cells) (v : Bool)
  | dispatchCached (t : Fin requests) (b : Fin buffers)
  | dispatchMiss (t : Fin requests) (b : Fin buffers) (s : Fin slots)
  | applyWrite (t : Fin requests) (b : Fin buffers)
  | prepareWrite (b : Fin buffers) (s : Fin slots)
  | submit (s : Fin slots)
  | kernelWriteCell (s : Fin slots) (c : Fin cells)
  | kernelComplete (s : Fin slots) (success : Bool)
  | reapRead (s : Fin slots)
  | chainReadToWrite (s : Fin slots) (t : Fin requests)
  | reapReadError (s : Fin slots)
  | reapWrite (s : Fin slots)
  | evict (b : Fin buffers)
  deriving Repr, DecidableEq, Lean.ToJson, Hashable, Veil.FinEncodable, Veil.Enumeration

instance : Inhabited (Event p b s t c) := ⟨.idle⟩

def Event.run (state : Protocol.State) : Event p b s t c → Option Protocol.State
  -- Idling is enabled only without outstanding work, so it cannot mask a stuck request.
  | .idle => if Safety.quiescent state then some state else none
  | .accept t p k c v => Protocol.accept state t.val p.val k c.val v
  | .dispatchCached t b => Protocol.dispatchCached state t.val b.val
  | .dispatchMiss t b s => Protocol.dispatchMiss state t.val b.val s.val
  | .applyWrite t b => Protocol.applyWrite state t.val b.val
  | .prepareWrite b s => Protocol.prepareWrite state b.val s.val
  | .submit s => Protocol.submit state s.val
  | .kernelWriteCell s c => Protocol.kernelWriteCell state s.val c.val
  | .kernelComplete s ok => Protocol.kernelComplete state s.val ok
  | .reapRead s => Protocol.reapRead state s.val
  | .chainReadToWrite s t => Protocol.chainReadToWrite state s.val t.val
  | .reapReadError s => Protocol.reapReadError state s.val
  | .reapWrite s => Protocol.reapWrite state s.val
  | .evict b => Protocol.evict state b.val

end KvellModel
