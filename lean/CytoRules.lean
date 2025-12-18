/-
  CytoRules — Lean 4 rule engine for the Cyto simulation.

  This library receives a coarse-grid simulation snapshot (JSON on stdin)
  and emits validated reaction rules (JSON on stdout).

  New rules are added here, not in Rust.  The Rust side is rule-agnostic:
  it serialises the snapshot, invokes this binary, and applies whatever
  rules come back.

  Module map:
  - `Types`    : shared snapshot/result data types + JSON codecs
  - `AcidBase` : neutralisation and acetate-buffer chemistry
  - `Catalyst` : catalyst-gated reactions (e.g. hexokinase)
  - `Eval`     : top-level evaluator that runs every rule family
-/

import CytoRules.Types
import CytoRules.AcidBase
import CytoRules.Catalyst
import CytoRules.Eval
