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
`translate_named_with_overrides` / `IsfRuntime::compile`, keyed by the name the
caller passes.

## Shipping: the directory is embedded in the binary

`build.rs` embeds every `*.wgsl` file in this directory with `include_str!`
(`isf::translate::EMBEDDED_OVERRIDES`), so a packaged engine carries the
overrides — there is no data directory next to the executable and
`electron-builder.yml` needs no extra resource. Adding, editing or deleting an
override only needs a rebuild; `cargo:rerun-if-changed` tracks the directory.

Lookup order:

1. `MODV_ISF_OVERRIDES_DIR` (or the committed directory in a development
   checkout), so tests and local experiments can shadow the embedded copy;
2. the embedded table.

`src/isf/overrides` remains the source of truth; nothing is generated into the
repository.

## Name collisions: the fingerprint guard

The file stem alone cannot prove that a shader is the corpus shader it is named
after. An override therefore applies only when

* the shader is known to come from the bundled corpus
  (`isf::translate::ShaderOrigin::Corpus`, used by the corpus report), or
* the parsed `IsfDocument::fragment_source` matches the fingerprint recorded for
  that name in `crates/modv-renderers/data/isf-overrides.json`
  (`isf::translate::document_fingerprint`, `fnv64:<hex>`), which is how
  `IsfRuntime::compile` treats every shader.

A user media shader called `Soft Blur.fs` with different source therefore
translates (or fails) on its own merits instead of silently inheriting corpus
behaviour. When you add or update an override, add the corpus file's
fingerprint:

```json
{
  "overrides": {
    "Soft Blur": "fnv64:0123456789abcdef"
  }
}
```

## Validation

An override is parsed and validated with naga, then checked against the exact
binding contract the runtime generates for that shader: a `@fragment fn main`,
a `@location(0) vec4<f32>` output, the `IsfUniforms` members in the generated
order and types, and every texture/sampler binding pair. Anything else is
reported as `TranslateError::InvalidOverride` when the shader is compiled,
instead of a wgpu validation panic at pipeline creation. `cargo test -p
modv-renderers` also asserts that every file here is present in
`EMBEDDED_OVERRIDES`.

## Rect samplers

`texture2DRect`/`sampler2DRect` are rejected with
`TranslateError::Unsupported` rather than rewritten to normalised `texture()`.
They sample in pixel coordinates, so the rewrite produced silently wrong
output; those shaders need a real `textureLoad` override.

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

* **`FRAMEINDEX`** is 1-based, as in `ISFRenderer.setDateUniforms`: the
  renderer increments its counter before the first draw, so the first
  presented frame sees `1` (not `0`). `IsfFrameUniforms::default()` matches
  that first frame.

The validator checks the entry point, the `IsfUniforms` member list/order/
types and every texture/sampler binding pair, so a mismatch is a
`TranslateError::InvalidOverride` naming the offending member or binding.

Pass semantics are unchanged from ISF: the same module is used for every
pass, `PASSINDEX` selects the branch, and the final pass decides the output.
A final pass **with** a `TARGET` renders into that buffer and the runtime then
blits it to the destination; an **implicit** final pass (no `TARGET`) renders
directly to the destination and nothing else is blitted — the same contract as
`ISFRenderer.draw`, which resets `lastTarget` on every implicit pass.

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
