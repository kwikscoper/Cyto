#!/usr/bin/env bash
#
# Build and test everything (macOS).
#
# Sets up the MoltenVK / Lean / shaderc environment, then runs:
#   1. Lean rule-engine build (with the v4.16.0 relink workaround)
#   2. A Lean engine smoke check
#   3. The full Rust workspace build
#   4. kinetics unit tests (CPU)
#   5. fluidsim GPU + Lean integration tests
#
# Pass --with-demo to also run the windowed demo smoke test (opens a window).
#
# Usage:
#   ./scripts/run-all.sh
#   ./scripts/run-all.sh --with-demo

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v brew >/dev/null 2>&1; then
  echo "error: Homebrew not found; this script targets macOS + Homebrew Vulkan/MoltenVK." >&2
  exit 1
fi
BREW_PREFIX="$(brew --prefix)"

# --- environment (build + runtime) ---
export CMAKE_POLICY_VERSION_MINIMUM=3.5                                   # shaderc-sys / CMake 4.x shim
export DYLD_FALLBACK_LIBRARY_PATH="$BREW_PREFIX/lib"                      # Vulkan loader
export VK_ICD_FILENAMES="$BREW_PREFIX/etc/vulkan/icd.d/MoltenVK_icd.json" # MoltenVK driver
export CYTO_LEAN_BINARY="$ROOT/lean/.lake/build/bin/cyto-rules"           # Lean rule engine

fail=0
step() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
ok()   { printf '\033[1;32m    OK: %s\033[0m\n' "$*"; }
bad()  { printf '\033[1;31m    FAIL: %s\033[0m\n' "$*"; fail=1; }

step "1/5  Build Lean rule engine (cyto-rules)"
if ( cd lean \
      && LEAN_PREFIX="$(lean --print-prefix)" \
         LEAN_CC=clang \
         LIBRARY_PATH="$(lean --print-prefix)/lib:$(lean --print-prefix)/lib/lean" \
         lake build ); then
  ok "lean build"
else
  bad "lean build"
fi

step "2/5  Lean engine smoke check"
if echo '{"sim_time":0.0,"species_names":[],"tiles":[]}' | "$CYTO_LEAN_BINARY"; then
  ok "cyto-rules runs"
else
  bad "cyto-rules failed to run"
fi

step "3/5  Build Rust workspace"
if cargo build --workspace; then ok "cargo build"; else bad "cargo build"; fi

step "4/5  kinetics unit tests (CPU)"
if cargo test -p kinetics; then ok "kinetics tests"; else bad "kinetics tests"; fi

step "5/5  fluidsim GPU + Lean integration tests"
if cargo test -p fluidsim; then ok "fluidsim tests"; else bad "fluidsim tests"; fi

if [[ "${1:-}" == "--with-demo" ]]; then
  step "extra  demo smoke test (opens a window, renders a few frames, exits)"
  if cargo run -p demo -- --smoke-test basic; then ok "demo smoke test"; else bad "demo smoke test"; fi
fi

printf '\n'
if [[ $fail -eq 0 ]]; then
  printf '\033[1;32m========== ALL CHECKS PASSED ==========\033[0m\n'
else
  printf '\033[1;31m========== SOME CHECKS FAILED =========\033[0m\n'
fi
exit $fail
