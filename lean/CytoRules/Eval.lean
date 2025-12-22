/-
  Eval.lean — Top-level rule evaluator.

  Runs every registered rule against the snapshot and collects results.
  To add a new rule, import it here and add it to `evaluateAll`.
-/

import CytoRules.Types
import CytoRules.AcidBase
import CytoRules.Catalyst

open CytoRules

namespace CytoRules.Eval

/-- Run all registered rules against the snapshot.
    New rules are added here — no Rust changes required: append another
    `RuleFamily.evaluate snap` to the concatenation below. -/
def evaluateAll (snap : Snapshot) : EvalResult :=
  let rules := AcidBase.evaluate snap ++ Catalyst.evaluate snap
  let diagnostics :=
    if rules.isEmpty then
      #["kinetics evaluator: no active rules"]
    else
      rules.map fun rule =>
        s!"{rule.reactionName} active in {rule.applicableTileIds.size} tiles"
  { rules, diagnostics }

end CytoRules.Eval
