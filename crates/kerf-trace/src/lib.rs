//! Raster image to SVG path tracing.
//!
//! Built so far: pre-processing into a black-and-white mask.
//! Planned: outline and colour-layer tracing via `vtracer`, and centerline tracing.

pub mod preprocess;

pub use preprocess::{Preprocess, Threshold};
