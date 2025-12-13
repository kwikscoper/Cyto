//! Lean bridge — subprocess communication with the Lean rule engine.
//!
//! The Lean binary (`cyto-rules`) reads a JSON snapshot from stdin and
//! writes a JSON `EvalResult` to stdout.  This module handles:
//!
//! - Locating the Lean binary (env var → build output → PATH)
//! - Spawning the process and piping JSON
//! - Surfacing non-zero exits and empty/invalid output as errors

use crate::KineticsError;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Bridge to the Lean rule engine binary.
pub struct LeanBridge {
    /// Resolved path to the `cyto-rules` binary.
    binary_path: PathBuf,
}

impl LeanBridge {
    /// Try to locate and validate the Lean binary.
    ///
    /// Search order:
    /// 1. `CYTO_LEAN_BINARY` environment variable
    /// 2. `lean/.lake/build/bin/cyto-rules` relative to the workspace root
    /// 3. `../lean/.lake/build/bin/cyto-rules` (when the CWD is a sub-crate)
    /// 4. `cyto-rules` on `PATH`
    pub fn discover() -> Result<Self, KineticsError> {
        // 1. Env var
        if let Ok(p) = std::env::var("CYTO_LEAN_BINARY") {
            let path = PathBuf::from(&p);
            if path.is_file() {
                log::info!("Lean binary from CYTO_LEAN_BINARY: {}", path.display());
                return Ok(Self { binary_path: path });
            }
            log::warn!("CYTO_LEAN_BINARY={} does not exist", p);
        }

        // 2. Workspace build output (relative to the kinetics crate)
        let workspace_candidates = [
            // When run from workspace root
            PathBuf::from("lean/.lake/build/bin/cyto-rules"),
            // When CWD is inside a sub-crate
            PathBuf::from("../lean/.lake/build/bin/cyto-rules"),
        ];
        for candidate in &workspace_candidates {
            if candidate.is_file() {
                let abs = candidate
                    .canonicalize()
                    .unwrap_or_else(|_| candidate.clone());
                log::info!("Lean binary from workspace: {}", abs.display());
                return Ok(Self { binary_path: abs });
            }
        }

        // 3. PATH lookup
        if let Ok(output) = Command::new("which").arg("cyto-rules").output()
            && output.status.success()
        {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let path = PathBuf::from(&path_str);
            if path.is_file() {
                log::info!("Lean binary from PATH: {}", path.display());
                return Ok(Self { binary_path: path });
            }
        }

        Err(KineticsError::LeanError(
            "Lean binary 'cyto-rules' not found. \
             Build with `cd lean && lake build` or set CYTO_LEAN_BINARY."
                .to_string(),
        ))
    }

    /// Get the resolved binary path.
    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }

    /// Send a JSON snapshot to the Lean binary and receive the evaluation result.
    ///
    /// Returns the raw JSON string from stdout on success.
    pub fn evaluate_json(&self, input_json: &str) -> Result<String, KineticsError> {
        let mut child = Command::new(&self.binary_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                KineticsError::LeanError(format!(
                    "Failed to spawn {}: {}",
                    self.binary_path.display(),
                    e
                ))
            })?;

        // Write the snapshot, then drop the handle so the child sees EOF on stdin.
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| KineticsError::LeanError("Failed to open stdin pipe".to_string()))?;
            stdin.write_all(input_json.as_bytes()).map_err(|e| {
                KineticsError::LeanError(format!("Failed to write to stdin: {}", e))
            })?;
        }

        // Block until the child finishes and collect its full output.
        let output = child
            .wait_with_output()
            .map_err(|e| KineticsError::LeanError(format!("Lean process error: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(KineticsError::LeanError(format!(
                "Lean binary exited with {}: {}",
                output.status, stderr
            )));
        }

        let stdout = String::from_utf8(output.stdout)
            .map_err(|e| KineticsError::LeanError(format!("Invalid UTF-8 from Lean: {}", e)))?;

        if stdout.trim().is_empty() {
            return Err(KineticsError::LeanError(
                "Lean binary produced empty output".to_string(),
            ));
        }

        Ok(stdout)
    }
}

impl std::fmt::Debug for LeanBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeanBridge")
            .field("binary_path", &self.binary_path)
            .finish()
    }
}

// The bridge is intentionally subprocess + JSON based rather than FFI: it keeps
// the Lean and Rust build graphs fully decoupled and lets the rule engine be
// rebuilt, swapped, or run out-of-process without touching the Rust ABI. If a
// lower-latency path is ever needed, an in-process FFI evaluator could be added
// as an alternative `RuleEvaluator` implementation without changing this API.
