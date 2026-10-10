//! G-code generation for GRBL lasers and CNC routers.
//!
//! Built so far: SVG loading, laser cut, laser fill and CNC profile operations.
//! Planned: raster image engraving, and machine and material presets.

pub mod generate;
pub mod geometry;
mod image_engrave;
pub mod job;
pub mod relief;

pub use generate::{generate, generate_with, Bounds, Program};
pub use geometry::{Artwork, Polyline, Shape};
pub use job::{Job, Machine, MachineKind, Operation, OperationKind, Origin};
pub use relief::Heightmap;
