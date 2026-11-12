# WGSL overrides

Every shader in the ISF corpus is translated from GLSL with naga
(`super::translate`). When naga's GLSL front-end (or the validator) rejects a
shader, the runtime looks for a **complete WGSL replacement** here:

```
overrides/<shader-name>.wgsl
```

`<shader-name>` is the `.fs` file stem exactly as it appears in
`src/renderer/src/application/renderers/isf/modules/isf`, spaces and case
included — e.g. `Soft Blur.fs` → `Soft Blur.wgsl`. The lookup is performed by
`translate_named` / `IsfRuntime::compile`, keyed by the name the caller
passes, so a shader loaded from a user media folder must use the same name.
`MODV_ISF_OVERRIDES_DIR` redirects the directory (used by tests).

Overrides are validated with naga's WGSL front-end before use; an invalid
override is reported as `TranslateError::InvalidOverride` instead of silently
falling back.

## The binding contract

An override replaces the *whole* translated fragment module. It must match
exactly the contract the runtime generates for the shader it replaces:

* **Entry point** `main`, a `@fragment` function returning
  `@location(0) vec4<f32>` (directly or through a struct with a
  `@location(0)` member).
* **`@group(0) @binding(0)`** — `var<uniform> global: IsfUniforms`, bound with
  a **dynamic offset** (one 256-byte slice per pass). The `IsfUniforms`
  struct must contain the members generated for that shader, in the same
  order and with the same types: `super::source::uniform_members(&doc)`
  produces the list, `super::source::uniform_layout` the byte offsets.
  The fixed prefix is:

  ```wgsl
  struct IsfUniforms {
      DATE: vec4<f32>,
      RENDERSIZE: vec2<f32>,
      PASSINDEX: i32,
      FRAMEINDEX: i32,
      TIME: f32,
      TIMEDELTA: f32,
      // one member per scalar/vector ISF input, in header order:
      //   float  -> f32, long -> i32, bool/event -> i32 (see below),
      //   color -> vec4<f32>, point2D -> vec2<f32>
      // then three members per texture in texture_inputs() order:
      //   _<name>_imgSize: vec2<f32>,
      //   _<name>_imgRect: vec4<f32>,   // always (0, 0, 1, 1)
      //   _<name>_flip:    i32,         // always 0
  }
  ```

  `bool`/`event` inputs are declared as `isf_bool_<NAME>: i32` (WGSL forbids
  `bool` in a uniform block); read them as `bool(global.isf_bool_NAME)`.

* **Texture bindings** — for each entry of
  `super::source::texture_inputs(&doc)` (all `image`/`audio`/`audioFFT`
  inputs in declaration order, then every pass `TARGET`), a pair in order:

  ```wgsl
  @group(0) @binding(2*i + 1) var isf_tex_<name>: texture_2d<f32>;
  @group(0) @binding(2*i + 2) var isf_smp_<name>: sampler;
  ```

  The runtime binds a `Filtering` sampler for normal textures and a
  `NonFiltering` sampler for `FLOAT` buffers (allocated as `Rgba32Float`,
  which is not filterable). Do not declare the sample type as filterable for
  a float target.

* **Varyings** — only `@location(0) isf_FragNormCoord: vec2<f32>` is
  provided by the built-in vertex stage (`runtime::DEFAULT_VERTEX_WGSL`).
  Do not add further entry-point inputs.

Pass semantics are unchanged from ISF: the same module is used for every
pass, `PASSINDEX` selects the branch, and the final pass with a `TARGET` is
blitted to the destination by the runtime.

## Example

A minimal override for a generator that ignores its inputs (inspect the
shader's own generated WGSL with `translate::translate_named` in a scratch
program; the corpus report lists exactly which shaders need one):

```wgsl
struct IsfUniforms {
    DATE: vec4<f32>,
    RENDERSIZE: vec2<f32>,
    PASSINDEX: i32,
    FRAMEINDEX: i32,
    TIME: f32,
    TIMEDELTA: f32,
}

@group(0) @binding(0) var<uniform> global: IsfUniforms;

@fragment
fn main() -> @location(0) vec4<f32> {
    return vec4<f32>(fract(global.TIME), 0.0, 0.0, 1.0);
}
```

## When *not* to add an override

Prefer fixing the preprocessor (`super::source`) when a whole class of
shaders fails — e.g. a missing macro or an ES spelling. Overrides are for
genuine one-offs (vendor-specific GLSL that naga cannot parse). The corpus
report (`docs/isf-corpus-report.md`) lists every failure precisely so the
decision is data-driven, and the WS-4 acceptance target is ≥ 95 % translated
without overrides.
