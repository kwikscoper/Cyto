-- Build config for the `cyto-rules` engine: a `CytoRules` library plus a
-- `cyto-rules` executable (rooted at `Main`) that the Rust `kinetics` crate
-- spawns as a subprocess.
import Lake
open Lake DSL

package «cyto-rules» where
  leanOptions := #[
    ⟨`autoImplicit, false⟩
  ]

lean_lib CytoRules where

@[default_target]
lean_exe «cyto-rules» where
  root := `Main
  supportInterpreter := true
