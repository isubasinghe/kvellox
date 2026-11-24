import KvellModel.Export.Explore

/-! Machine-facing entry point. The friendly CLI is scripts/export-model.py. -/
def main (args : List String) : IO UInt32 := do
  let [output, pages, buffers, slots, requests, cells, maxStates, maxDepth] := args
    | do
      (← IO.getStderr).putStrLn "usage: Main.lean OUTPUT PAGES BUFFERS SLOTS REQUESTS CELLS MAX_STATES MAX_DEPTH"
      return 64
  let some ns := [pages, buffers, slots, requests, cells, maxStates, maxDepth].mapM String.toNat?
    | do
      (← IO.getStderr).putStrLn "bounds and limits must be nonnegative integers"
      return 64
  let [p, b, s, r, c, limit, depth] := ns | return 64
  IO.FS.withFile output .write fun out =>
    KvellModel.Export.explore out ⟨p, b, s, r, c⟩ limit depth
  return 0
