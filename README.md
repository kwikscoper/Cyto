# Cyto

Cyto is a GPU-accelerated 2D chemical transport sandbox written in Rust on top of Vulkan, with a Lean 4 rule engine driving its chemistry. It is "pre-production R&D" for a video game: a fine-grid simulation runs on the GPU while a slower semantic layer, expressed in Lean, decides which reactions are active.

The current build includes a Lean-backed reaction layer, coarse semantic snapshots, thermal transport, membrane leak channels, and moving enzyme entities.

## Architecture at a Glance

The workspace is split into a fast fine-grid transport loop on the GPU and a slower semantic reasoning loop in Lean:

| Crate      | Responsibility                                                                 |
|------------|--------------------------------------------------------------------------------|
| `fluidsim` | Core GPU simulation: transport, reaction, leak, enzyme, and thermal passes.    |
| `kinetics` | Low-frequency semantic layer; builds snapshots and bridges to Lean.            |
| `lean`     | The `cyto-rules` executable that decides which reactions are active.           |
| `renderer` | Vulkan visualization of concentration/temperature plus an egui overlay.        |
| `demo`     | Interactive desktop application and scenario runner.                           |

`fluidsim` owns the high-frequency simulation. `kinetics` is invoked roughly once per simulated second: it serializes a coarse snapshot to JSON, hands it to the Lean `cyto-rules` binary, and applies the compact reaction directives that come back. Lean is the single source of truth for active semantic rules.

## Feature Set

### Fine-grid GPU simulation

- Multi-species transport on a dense `[species][cell]` buffer layout.
- Explicit diffusion on Vulkan compute shaders with ping-pong buffers.
- Solid geometry and material masks for impermeable walls and embedded structures.
- A per-cell temperature field with a separate thermal diffusion pass.
- Optional charge-correction / electrochemical transport heuristics for ionic systems.
- A shared render/simulation Vulkan context, so rendering can bind live simulation buffers directly.

### Lean-backed kinetics and semantics

- `fluidsim` builds a coarse semantic snapshot once per simulated second.
- `kinetics` serializes that snapshot to JSON and sends it to Lean (`cyto-rules`).
- Lean returns compact reaction directives rather than replacing the simulation state.
- Directives can carry:
  - Mass-action or Michaelis–Menten kinetics.
  - Tile-local applicability.
  - Thermodynamic metadata: $\Delta H$, $\Delta G$, $\Delta S$, and activation energy.
- A GPU reaction pass consumes those directives and updates concentration and temperature fields in place.

### Chemistry implemented today

- Strong acid/base neutralization: $\mathrm{H^+ + OH^- \rightarrow H_2O}$.
- Weak-acid buffer behavior for the acetic acid / acetate system.
- Direct neutralization of acetic acid by hydroxide.
- A catalyst-gated phosphorylation rule for hexokinase.
- Michaelis–Menten kinetics for catalyst-driven reactions.

### Membranes, leaks, and enzymes

- Leak channels embedded in solid boundaries for directional transport experiments.
- Electrochemical leak heuristics that preserve directional flow while damping unstable local charge separation.
- Moving enzyme entities with drift, rotation, and thermal/circulation heuristics.
- A dedicated enzyme GPU pass for entity-mediated catalysis, separate from dissolved-catalyst rules.

### Inspection and debugging

- Async coarse inspection readback for hover tooltips.
- Detail mode with pinned probe callouts around the simulation viewport.
- A thermal visualization overlay.
- A performance overlay for frame-time monitoring.
- A smoke-test mode that renders a few frames and exits.

## Simulation Flow

Each frame, the demo advances the fine-grid simulation on the GPU and renders the current concentration or temperature field. On a slower cadence it also runs a semantic pass:

1. Build a coarse snapshot from the current grid.
2. Send that snapshot to Lean through the `kinetics` crate.
3. Receive reaction directives for the tiles where rules are active.
4. Upload those directives back to the GPU.
5. Continue the fine-grid simulation with the updated kinetics parameters.

This split keeps high-frequency transport on the GPU while moving rule selection and reaction semantics into Lean.

## Scenarios

The demo ships with six scenarios:

| Scenario    | Description                                                                       |
|-------------|-----------------------------------------------------------------------------------|
| `basic`     | The original Na/K/Cl transport demo inside a hollow titanium box with a temperature split. |
| `acid-base` | Strong acid / strong base mixing with exothermic neutralization.                  |
| `buffers`   | A weak-acid buffer against NaOH, including acetate/acetic-acid equilibrium.        |
| `catalyst`  | Dissolved hexokinase driving glucose phosphorylation.                             |
| `enzyme`    | Moving enzyme entities performing the same phosphorylation chemistry as localized actors. |
| `leak`      | A buffered ionic system with membrane leak channels for K⁺ and Na⁺ transport.     |

## Building

### Requirements

- A Rust 2024 edition toolchain.
- A Vulkan 1.2-capable GPU and working Vulkan driver.
  - On **Linux**, a native Vulkan driver (Mesa, proprietary, etc.).
  - On **macOS**, the Vulkan SDK with **MoltenVK** (Vulkan-over-Metal). This is the primary cross-platform target alongside Linux.
- Lean 4 and Lake (see `lean/lean-toolchain` for the pinned version).

> Other platforms may work but are untested. Presentation behavior in particular depends on the windowing system.

### macOS setup (Homebrew + workarounds)

macOS has no native Vulkan driver, and a couple of current toolchain versions need small workarounds. The steps below are verified on Apple Silicon with Homebrew; on Intel Macs `brew --prefix` resolves to `/usr/local` instead of `/opt/homebrew`.

Install the Vulkan runtime (loader + MoltenVK) and tools:

```bash
brew install molten-vk vulkan-loader vulkan-headers vulkan-tools
```

The loader and the MoltenVK ICD live under the Homebrew prefix and must be visible at runtime (for the demo and for the GPU tests):

```bash
export DYLD_FALLBACK_LIBRARY_PATH="$(brew --prefix)/lib"
export VK_ICD_FILENAMES="$(brew --prefix)/etc/vulkan/icd.d/MoltenVK_icd.json"
```

> Cyto already opts into the Vulkan portability extensions (`VK_KHR_portability_enumeration` / `VK_KHR_portability_subset`) when present, so MoltenVK is discovered automatically once the loader and ICD above are on the path.

### Build the Lean rule engine

The simulation initializes the kinetics layer by default, so build the Lean executable before running the demo:

```bash
cd lean
lake build
cd ..
```

On **macOS**, the pinned Lean toolchain (v4.16.0) produces a binary that fails to load on recent macOS with a `__DATA_CONST segment missing SG_READ_ONLY flag` dyld error. Relink against the toolchain's bundled libraries by building with:

```bash
cd lean
LEAN_PREFIX="$(lean --print-prefix)"
LEAN_CC=clang LIBRARY_PATH="$LEAN_PREFIX/lib:$LEAN_PREFIX/lib/lean" lake build
cd ..
```

The Rust side looks for the binary in the following order:

1. `CYTO_LEAN_BINARY`
2. `lean/.lake/build/bin/cyto-rules` (from the workspace root)
3. `../lean/.lake/build/bin/cyto-rules` (from inside a sub-crate)
4. `cyto-rules` on `PATH`

If you build the Lean binary elsewhere, point the env var at it:

```bash
export CYTO_LEAN_BINARY=/absolute/path/to/cyto-rules
```

### Build the Rust workspace

```bash
cargo build --release
```

`fluidsim` compiles its shaders through `shaderc-sys`, which builds the bundled `shaderc` C++ library via CMake. With recent CMake (4.x), that build aborts with `Compatibility with CMake < 3.5 has been removed`. Until the dependency is updated, set the compatibility shim for the build (and for `cargo test`):

```bash
export CMAKE_POLICY_VERSION_MINIMUM=3.5
```

## Running the Demo

Show CLI help:

```bash
cargo run -p demo -- --help
```

Run the default scenario:

```bash
cargo run --release -p demo
```

Run a specific scenario:

```bash
cargo run --release -p demo -- acid-base
cargo run --release -p demo -- buffers
cargo run --release -p demo -- catalyst
cargo run --release -p demo -- enzyme
cargo run --release -p demo -- leak
```

Useful flags:

- `--detail` — render the sim as an inset with pinned inspection probes.
- `--smoke-test` — render 5 frames and exit.
- `--present-mode auto|fifo|mailbox` — choose the Vulkan present mode. `auto` picks a capture-friendly default for the current windowing system.

Examples:

```bash
cargo run --release -p demo -- --detail leak
cargo run --release -p demo -- --smoke-test basic
cargo run --release -p demo -- --present-mode fifo enzyme
```

## Controls

General:

- **Mouse hover** — inspect the coarse cell under the cursor.
- **Space** — pause or resume the simulation.
- **`+` / `-`** — increase or decrease the inspection mip factor.
- **Hold `T`** — show the thermal view.
- **Tab** — toggle the performance overlay.
- **Shift+R** — reset the current scenario.
- **Escape** — quit.

Leak editor:

- Use the egui `CREATE` panel to add leak channels.
- **Left click** — select a leak channel, or confirm placement/transform.
- **`R`** — rotate a leak channel by 45° while placing or transforming it.
- **`T`** (with a leak channel selected) — enter transform mode.
- **Delete** — remove the selected leak channel.

## Tests and Probes

The `fluidsim` crate includes probe binaries and regression tests for the newer chemistry and transport paths:

- `acid_base_probe` — checks center-window neutralization and exothermic heating.
- `buffer_probe` — checks weak-acid / hydroxide consumption and acetate formation.
- `leak_probe` — checks K⁺ inward flow, Na⁺ outward flow, mass conservation, and bounded local charge error.

Run them with:

```bash
cargo test -p fluidsim
```

These tests dispatch real GPU work and invoke the Lean engine, so they need a working Vulkan device and the `cyto-rules` binary. On macOS, run them with the environment from the [macOS setup](#macos-setup-homebrew--workarounds) section plus the CMake shim:

```bash
CMAKE_POLICY_VERSION_MINIMUM=3.5 \
DYLD_FALLBACK_LIBRARY_PATH="$(brew --prefix)/lib" \
VK_ICD_FILENAMES="$(brew --prefix)/etc/vulkan/icd.d/MoltenVK_icd.json" \
CYTO_LEAN_BINARY="$PWD/lean/.lake/build/bin/cyto-rules" \
cargo test -p fluidsim
```

## Notes

- Lean is the source of truth for active semantic rules. New rule families are intended to be added in Lean first, with Rust remaining mostly rule-agnostic.
- The simulation is intentionally split into a fast fine-grid transport loop and a slower semantic reasoning loop.
- The project targets Linux and macOS (via MoltenVK); both assume a working Vulkan presentation path.
- Screen capture via tools like OBS may behave inconsistently because of the low-level Vulkan presentation path.
- Chemical species are currently represented by molecular formulae. Representing them as full structural formulae is planned.
