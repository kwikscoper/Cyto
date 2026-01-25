#!/usr/bin/env bash
#
# Launch the interactive Cyto demo on macOS.
#
# Sets up the MoltenVK / Lean / shaderc environment, then runs the demo with
# whatever arguments you pass through.
#
# Examples:
#   ./scripts/demo.sh                       # default "basic" scenario
#   ./scripts/demo.sh acid-base             # a specific scenario
#   ./scripts/demo.sh --detail leak         # inset view with pinned probes
#   ./scripts/demo.sh --help                # show the demo's own CLI help
#
# Scenarios: basic | acid-base | buffers | catalyst | enzyme | leak

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v brew >/dev/null 2>&1; then
  echo "error: Homebrew not found; this launcher targets macOS + Homebrew Vulkan/MoltenVK." >&2
  exit 1
fi
BREW_PREFIX="$(brew --prefix)"

export CMAKE_POLICY_VERSION_MINIMUM=3.5
export DYLD_FALLBACK_LIBRARY_PATH="$BREW_PREFIX/lib"
export VK_ICD_FILENAMES="$BREW_PREFIX/etc/vulkan/icd.d/MoltenVK_icd.json"
export CYTO_LEAN_BINARY="$ROOT/lean/.lake/build/bin/cyto-rules"

# Build the Lean engine if it is missing (with the macOS relink workaround).
if [[ ! -x "$CYTO_LEAN_BINARY" ]]; then
  echo "Lean engine not built yet; building cyto-rules..."
  ( cd lean \
      && LEAN_CC=clang \
         LIBRARY_PATH="$(lean --print-prefix)/lib:$(lean --print-prefix)/lib/lean" \
         lake build )
fi

exec cargo run --release -p demo -- "$@"
