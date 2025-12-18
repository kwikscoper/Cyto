/-
  Main.lean — CLI entry point for the Lean rule engine.

  Protocol:
    1. Read JSON snapshot from stdin  (one object, no newline framing)
    2. Parse into `Snapshot`
    3. Run all rules via `Eval.evaluateAll`
    4. Write JSON `EvalResult` to stdout
    5. Exit 0 on success, non-zero on parse/eval error

  The Rust `lean_bridge` module spawns this binary and communicates
  via stdin/stdout pipes.
-/

import CytoRules
import Lean.Data.Json

open Lean (Json FromJson ToJson)
open CytoRules

def main : IO Unit := do
  -- Slurp the whole snapshot from stdin. `/dev/stdin` works on both Linux and
  -- macOS, which keeps this entry point portable across the dev targets.
  let input ← IO.FS.readFile "/dev/stdin"

  let json ← match Json.parse input with
    | .ok j    => pure j
    | .error e =>
      IO.eprintln s!"JSON parse error: {e}"
      IO.Process.exit 1

  let snap ← match @FromJson.fromJson? Snapshot _ json with
    | .ok s    => pure s
    | .error e =>
      IO.eprintln s!"Snapshot deserialisation error: {e}"
      IO.Process.exit 1

  -- Evaluate every registered rule and emit the result as compact JSON.
  let result := Eval.evaluateAll snap
  IO.println (ToJson.toJson result).compress
