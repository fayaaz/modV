//! Compatibility path for the pre-batch-1 Chromatic Line port.
//!
//! The implementation moved into [`crate::point10::chromatic::line`] when the
//! ring-buffer simulation was factored out for ChromaticWave, but
//! `modv_renderers::point10::chromatic_line::{ChromaticLine, SyntheticSine}`
//! stays valid (the app's headless renderer uses it — `modv-app/src/headless.rs`).
//!
//! New code should import from [`crate::point10::chromatic`] directly.

pub use crate::point10::chromatic::SyntheticSine;
pub use crate::point10::chromatic::line::*;

#[cfg(test)]
mod tests {
    #[test]
    fn the_historical_path_still_exports_the_scene() {
        // Type-check only: constructing needs a device, but the public types
        // must resolve from this path.
        fn _accepts(_: &crate::point10::chromatic_line::ChromaticLine) {}
        let _ = std::mem::size_of::<crate::point10::chromatic_line::SyntheticSine>();
    }
}
