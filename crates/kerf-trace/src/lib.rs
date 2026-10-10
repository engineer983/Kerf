//! Raster image to SVG path tracing.
//!
//! Built so far: pre-processing into a black-and-white mask, and outline and
//! colour tracing via `vtracer`. Planned: centerline tracing.

pub mod preprocess;
pub mod trace;

pub use image;
pub use preprocess::{Preprocess, Threshold};
pub use trace::{trace, Mode, TraceSettings, Traced};
