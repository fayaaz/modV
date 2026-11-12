# ISF corpus translation report

The corpus gate is the checked-in baseline `crates/modv-renderers/data/isf-corpus-baseline.json` (asserted by `cargo test -p modv-renderers`; the test no longer rewrites this file). The corpus is the vendored `src/renderer/src/application/renderers/isf/modules/isf`.

- Corpus directory: `src/renderer/src/application/renderers/isf/modules/isf`
- Fragment shaders (`.fs`): **351**
- Translated without an override: **351** (100.0 %)
- Translated via an override: **0**
- Failed: **0**
- Total passes across the corpus: 507
- Shaders with a `PERSISTENT` buffer: 46; with a `FLOAT` buffer: 15
- Vertex sidecars (`.vs`): 36 — translated 36, failed 0

## Overrides

None.

## Failures

None.

## Notes

- A shader counts as translated only when **every** pass translates.
- Coordinate systems differ from GL: `isf_FragNormCoord` is top-left origin in wgpu, `gl_FragCoord` is top-left instead of bottom-left. Texture sampling stays visually identical; shaders that treat `isf_FragNormCoord` as a Cartesian coordinate are mirrored.
- `bool`/`event` inputs travel as `int` uniforms copied into private `bool` globals at the top of the entry point (a `bool(...)` macro would rewrite `.r` swizzles for inputs named `r`), and `_<name>_flip` is an `int` because WGSL forbids `bool` in a uniform block.
- The last seven shaders now translate via four general source rewrites: hoisted non-constant global initialisers, per-sampler cloning of functions with `sampler2D` parameters, default returns for non-void fall-through paths, and split `matN` varyings. No override is needed for the corpus; the override mechanism remains for genuinely ambiguous third-party shaders.
- Fragment-declared `in`/`varying` lines become zero-initialised private globals: the runtime always draws the built-in full-screen triangle, so unbound sidecar varyings (the corpus' `left_coord`/`texOffsets` template leftovers) read zero instead of failing to link.
