//! Vulkan renderer for the fluid simulation.
//!
//! Visualizes simulation state with Vulkan (via `ash`):
//! - Fluid concentration fields as a 2D image
//! - Solid geometry with distinct coloring
//! - egui overlays for inspection tooltips
//!
//! ## Architecture
//!
//! The renderer consumes [`RenderState`](fluidsim::RenderState) and produces
//! frames by drawing a single fullscreen triangle whose fragment shader samples
//! the concentration data and maps it to colors. It runs on any Vulkan 1.2
//! target supported by `ash`, including Linux (X11/Wayland) and macOS via
//! MoltenVK.

pub mod context;
pub mod egui_integration;
pub mod pipeline;

pub use context::{PresentModePreference, RenderContext};
pub use egui_integration::EguiRenderer;
pub use pipeline::{RenderPipeline, RenderViewport};
