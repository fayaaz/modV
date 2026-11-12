//! WGSL ports of the four default shader flavours declared in
//! `src/renderer/src/application/renderers/shader/default-shader.js`.
//!
//! | GLSL flavour | Rust port                       | Notes                              |
//! |--------------|---------------------------------|------------------------------------|
//! | `v`          | [`DEFAULT_VERTEX`]              | one vertex path, no `v300` split   |
//! | `f`          | [`default_fragment`]            | gradient fallback, alpha = 1.0     |
//! | `v300`       | — (not ported)                  | see [`DEFAULT_VERTEX`] docs        |
//! | `fWrap`      | [`DEFAULT_MAINIMAGE_WRAP`]      | `%MAIN_IMAGE_INJECT%` template     |
//!
//! # Wrapping semantics (ported from `index.js:140-144`)
//!
//! The legacy pipeline inspects the fragment source: if it does **not**
//! contain `gl_FragColor`, it is wrapped in the `#version 300 es` `fWrap`
//! shim which declares `mainImage(out vec4, in vec2)` and a `main()` that
//! calls it, then swaps in the matching `v300` vertex shader. The Rust port
//! keeps exactly two flavours:
//!
//! 1. **Direct flavour** — the module's WGSL declares its own `fs_main`
//!    returning `@location(0) vec4<f32>`. Used by the five modules whose
//!    GLSL writes `gl_FragColor` (Bands, Fisheye, MattiasCRT, Plasma,
//!    Wobble).
//! 2. **mainImage flavour** — the module's WGSL declares a
//!    `mainImage(fragCoord: vec2<f32>) -> vec4<f32>` and
//!    [`DEFAULT_MAINIMAGE_WRAP`]'s `fs_main` calls it and **forces
//!    `image.a = 1.0`**, exactly as the GLSL wrapper does
//!    (`fWrap`: `image.a = 1.;`). Used by every module without
//!    `gl_FragColor`: all 31 Point10 effects and Chromatic Abberation.
//!
//! # Why strings are built by functions, not pure consts
//!
//! [`DEFAULT_VERTEX`] is a plain `&str` const. The fragment sources are
//! functions because they embed [`SHARED_UNIFORM_WGSL`](crate::shader::uniforms::SHARED_UNIFORM_WGSL),
//! and `concat!` cannot splice one const into another; `format!` at
//! registration time is the cheap, allocation-once equivalent. The generated
//! strings are byte-identical for every module of the same flavour.
//!
//! # Not ported: `v300`
//!
//! `default-shader.js` ships a second vertex shader, `v300`
//! (`#version 300 es`), used only by the GLSL ES3 code path that the
//! mainImage wrapper selects. WGSL has a single vertex stage for both
//! flavours, so `v300` disappears into [`DEFAULT_VERTEX`].
//!
//! # fragCoord origin
//!
//! GL's `gl_FragCoord` origin is bottom-left; wgpu's `@builtin(position)`
//! is top-left. Modules that consume `fragCoord` pixel-grid-wise
//! (MattiasCRT, Crosshatching, Crt's `crtMask`) observe flipped rows
//! relative to the GL reference. Only parity-sensitive patterns are
//! affected and none of the ported modules depend on the parity.

#![forbid(unsafe_code)]

pub(crate) use crate::shader::uniforms::SHARED_UNIFORM_WGSL;

/// Full-screen vertex shader shared by both fragment flavours.
///
/// Positions are the same big triangle the GLSL attribute path covers
/// (`(-1,-1), (3,-1), (-1,3)`), emitted from `@builtin(vertex_index)` so no
/// vertex buffer is bound. vUv is **not** declared here: GL's `vUv` varying
/// is a compatibility artefact of the pre-300-es path and every ported
/// module computes its own uv inline with `modv_uv` (see
/// `crate::shader::uniforms`).
///
/// Port of the shared structure of `v` / `v300` in `default-shader.js`.
pub const DEFAULT_VERTEX: &str = r#"// Full-screen triangle, the same three clip-space points modV's GLSL
// pipelines scan out of their aliased attribute buffer (x/y in {-1, -1, 3}).
@vertex fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
  var pos = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0),
  );
  return vec4<f32>(pos[vertex_index], 0.0, 1.0);
}
"#;

/// Body of the `f` gradient fallback (`default-shader.js`), without the
/// shared prelude — see [`default_fragment`].
const DEFAULT_FRAGMENT_BODY: &str = r#"@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let vUv = modv_uv(position.xy);
  // Port of: gl_FragColor = vec4(vUv.x, 0.5+0.5*cos(iGlobalTime/1.1),
  //                                       0.5+0.5*sin(iGlobalTime), 1.0);
  return vec4<f32>(
    vUv.x,
    0.5 + 0.5 * cos(u.iGlobalTime / 1.1),
    0.5 + 0.5 * sin(u.iGlobalTime),
    1.0,
  );
}
"#;

/// The `f` gradient fallback, complete as a compilable shader string.
///
/// Paints the horizontal gradient + time-animated green/blue the legacy
/// renderer uses when a module declares no fragment shader. No in-tree
/// module relies on it (all 37 ship their own), but the renderer must keep
/// it for parity with `defaults` in `index.js:127-133` and for
/// `makeProgram`'s `if (!frag) frag = defaultShader.f` path.
pub fn default_fragment() -> String {
    format!("{SHARED_UNIFORM_WGSL}\n{DEFAULT_FRAGMENT_BODY}")
}

/// Template for the `fWrap` shim (`default-shader.js`): declares the
/// `%MAIN_IMAGE_INJECT%` marker and the `fs_main` wrapper. The shared
/// uniform prelude is prepended by [`main_image_fragment`], so this const is
/// deliberately prelude-free (consts cannot splice other consts).
///
/// The wrapper replicates the GLSL: `main()` calls `mainImage(image,
/// gl_FragCoord.xy)` and **forces `image.a = 1.0`** before writing the
/// framebuffer — every mainImage-flavour module is therefore opaque even if
/// it writes a partial alpha.
pub const DEFAULT_MAINIMAGE_WRAP: &str = r#"%MAIN_IMAGE_INJECT%

@fragment fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let image = mainImage(position.xy);
  // GLSL fWrap: `image.a = 1.;` — alpha is forced opaque, no exceptions.
  return vec4<f32>(image.rgb, 1.0);
}
"#;

/// Build the mainImage-flavour shader: shared prelude, the module's
/// `mainImage` source, then the [`DEFAULT_MAINIMAGE_WRAP`] wrapper.
///
/// Mirrors `index.js:141-144`: `frag = fWrap.replace(/(%MAIN_IMAGE_INJECT%)/, frag)`.
pub fn main_image_fragment(module_body: &str) -> String {
    let wrapped = DEFAULT_MAINIMAGE_WRAP.replace("%MAIN_IMAGE_INJECT%", module_body);
    format!("{SHARED_UNIFORM_WGSL}\n{wrapped}")
}

/// Build the direct flavour: shared prelude followed by the module's own
/// `fs_main` source. Used by the five modules whose GLSL writes
/// `gl_FragColor`.
pub(crate) fn direct_fragment(module_body: &str) -> String {
    format!("{SHARED_UNIFORM_WGSL}\n{module_body}")
}

/// Validate a complete WGSL source with naga (front-end + full validator,
/// including the uniform-control-flow pass that `textureSample` requires).
/// Test helper shared by the shader-renderer test suite.
#[cfg(test)]
pub(crate) fn validate_wgsl(source: &str) -> Result<(), String> {
    let module =
        naga::front::wgsl::parse_str(source).map_err(|error| error.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_fragment_parses_and_validates() {
        validate_wgsl(&default_fragment()).expect("default fragment must validate");
    }

    #[test]
    fn default_vertex_and_fragment_validate_together() {
        // This is the exact pairing wgpu sees for the gradient fallback.
        let source = format!("{DEFAULT_VERTEX}\n{}", default_fragment());
        validate_wgsl(&source).expect("vertex + fragment must validate");
    }

    #[test]
    fn mainimage_wrap_expands_and_validates() {
        let body = r#"
fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> {
  let vUv = modv_uv(fragCoord);
  return textureSample(u_modVCanvas, modvGroupSampler, vUv);
}
"#;
        let expanded = main_image_fragment(body);
        validate_wgsl(&expanded).expect("expanded mainImage wrap must validate");
        assert!(
            expanded.contains("let image = mainImage(position.xy)"),
            "wrapper must call mainImage"
        );
        assert!(
            expanded.contains("vec4<f32>(image.rgb, 1.0)"),
            "wrapper must force alpha to 1.0"
        );
    }

    #[test]
    fn wrap_template_forces_opaque_alpha() {
        // The `image.a = 1.;` line of the GLSL fWrap, kept verbatim: modules
        // that write a partial alpha still reach the framebuffer opaque.
        assert!(DEFAULT_MAINIMAGE_WRAP.contains("vec4<f32>(image.rgb, 1.0)"));
    }

    #[test]
    fn wrap_replaces_exactly_the_marker() {
        let body = "fn mainImage(fragCoord: vec2<f32>) -> vec4<f32> { return vec4<f32>(fragCoord, 0.0, 1.0); }";
        let expanded = main_image_fragment(body);
        assert!(!expanded.contains("%MAIN_IMAGE_INJECT%"));
        assert!(expanded.matches("fn mainImage").count() == 1);
    }
}
