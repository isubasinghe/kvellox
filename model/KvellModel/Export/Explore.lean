import KvellModel.Event

namespace KvellModel.Export
open Lean

/-! Exact breadth-first exploration of the same event alphabet and transition
function used by SlabIO. Hashes index full states; equality resolves collisions.
JSONL streams nodes/edges so serialization does not duplicate the graph in RAM.
-/

def emit (out : IO.FS.Handle) (tag : String) (fields : List (String × Json)) : IO Unit :=
  out.putStrLn (Json.mkObj (("type", toJson tag) :: fields)).compress

def violations (s : Protocol.State) : List String :=
  [("cache_local", s.cache.wellFormed), ("transport_local", s.io.wellFormed),
   ("request_local", s.worker.wellFormed), ("buffer_ownership", Safety.ownership s),
   ("page_contents", Safety.dataCoherent s), ("dma_source_frozen", Safety.frozenImages s),
   ("waiter_ownership", Safety.waitersOwned s)].filterMap fun (name, ok) =>
     if ok then none else some name

def explore (out : IO.FS.Handle) (n : Bounds) (maxStates maxDepth : Nat) : IO Unit := do
  let events := (Veil.Enumeration.allValues :
    List (Event n.pages n.buffers n.slots n.requests n.cells)).toArray
  emit out "header" [
    ("schemaVersion", toJson (1 : Nat)),
    ("bounds", Json.mkObj [("pages", toJson n.pages), ("buffers", toJson n.buffers),
      ("slots", toJson n.slots), ("requests", toJson n.requests), ("cells", toJson n.cells)]),
    ("limits", Json.mkObj [("maxStates", toJson maxStates), ("maxDepth", toJson maxDepth)]),
    ("events", toJson (events.map fun e => Json.mkObj [
      ("label", toJson (reprStr e)), ("value", toJson e)]))]
  let initial := Protocol.init n
  let mut queue := #[(initial, 0)]
  let mut seen : Std.HashMap Protocol.State Nat := {}
  seen := seen.insert initial 0
  let mut cursor := 0
  let mut edgeCount := 0
  let mut omittedEdges := 0
  let mut depthLimited := false
  let mut violationCount := 0
  let mut deadlockCount := 0
  while cursor < queue.size do
    let (state, depth) := queue[cursor]!
    let expanded := maxDepth == 0 || depth < maxDepth
    let mut outgoing := 0
    let mut omittedHere := 0
    if expanded then
      for eventId in [:events.size] do
        if let some next := events[eventId]!.run state then
          outgoing := outgoing + 1
          let mut target : Option Nat := seen[next]?
          if target.isNone then
            if maxStates == 0 || queue.size < maxStates then
              target := some queue.size
              seen := seen.insert next queue.size
              queue := queue.push (next, depth + 1)
            else
              omittedHere := omittedHere + 1
          emit out "edge" [("source", toJson cursor), ("target", toJson target),
            ("event", toJson eventId)]
    else
      depthLimited := true
    let failed := violations state
    let quiescent := Safety.quiescent state
    let deadlock := expanded && outgoing == 0 && !quiescent
    if !failed.isEmpty then violationCount := violationCount + 1
    if deadlock then deadlockCount := deadlockCount + 1
    edgeCount := edgeCount + outgoing
    omittedEdges := omittedEdges + omittedHere
    emit out "node" [("id", toJson cursor), ("depth", toJson depth),
      ("expanded", toJson expanded), ("omittedEdges", toJson omittedHere),
      ("quiescent", toJson quiescent), ("deadlock", toJson deadlock),
      ("violations", toJson failed), ("state", toJson state)]
    cursor := cursor + 1
    if cursor % 1000 == 0 then
      (← IO.getStderr).putStrLn s!"Explored {cursor} states; discovered {queue.size}."
  emit out "summary" [("states", toJson queue.size), ("edges", toJson edgeCount),
    ("complete", toJson (!depthLimited && omittedEdges == 0)),
    ("omittedEdges", toJson omittedEdges), ("depthLimited", toJson depthLimited),
    ("violationStates", toJson violationCount), ("deadlockStates", toJson deadlockCount)]

end KvellModel.Export
