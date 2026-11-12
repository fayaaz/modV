//! Registry entry for the Chromatic Wave scene.
//!
//! The scene lives in [`crate::point10::chromatic::wave`] next to the shared
//! ring-buffer simulation it inherits from ChromaticLine (the JS class extends
//! `ChromaticLine`, so keeping `ChromaticLineSim` and the wave in one module
//! tree mirrors that relationship). This module exists so every scene the
//! [`crate::point10::all_scenes`] registry builds has a `scenes::*` path, like
//! the JS wrappers in `renderers/2d/modules/Point10*.js`.

pub use crate::point10::chromatic::wave::ChromaticWave;
