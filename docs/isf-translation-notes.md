# ISF → WGSL translation — findings (input to M2d)

Gathered by running the actual vendored runtime and naga inside the toolchain
container (`docker run -v modv-node-modules:/app/node_modules modv-toolchain:local ...`).
This is the reference contract the Rust ISF preprocessor must reproduce.

## naga feature matrix (naga 30.0.1)

- `glsl-in` does **not** enable `interpolator` (needed by the GLSL frontend);
  with `default-features = false` + only `glsl-in`, naga fails to compile with
  `no method named apply_default_interpolation found for enum ir::Binding`
  (front/glsl/variables.rs:467 vs front/interpolator.rs:39). Fixed upstream
  practice: always enable `spv-in` or `wgsl-in` together with `glsl-in`.
- Naga's GLSL frontend accepts **only** `#version 440`, `#version 450`,
  `#version 460` (plain core). It does **not** parse GLSL ES at all — ISF's
  `#version 300 es` is rejected as `InvalidVersion(300)`/`InvalidProfile("es")`.
- naga's default cargo features are **empty**; `wgsl-in` is not default.

Correct feature set for the translator:

```toml
naga = { version = "30", features = ["glsl-in", "spv-in", "spv-out", "wgsl-in", "wgsl-out"] }
```

Pipeline is `.fs → ISF preprocessor (ES→desktop GLSL 450) → naga glsl-in →
Module → naga::back::wgsl` (SPIR-V detour optional).

## ES → desktop GLSL rewrite (the preprocessor)

From a probe on Bloom.fs, Life.fs and badtv.fs, this gets each past the lexer:

1. Strip the `/*{ ... }*/` JSON header.
2. `#version 300 es` / `100` / absent → `#version 450`.
3. Remove every `precision ...` line (naga infers precision).
4. `texture2D(` → `texture(`; `texture2DLod` → `textureLod`;
   `texture2DGrad` → `textureGrad`; `textureCube(` → `texture(`.
5. `varying` → `in` (fragment stage) / `out` (vertex stage);
   `attribute` → `in` (vertex stage).
6. `gl_FragColor` → a declared `out vec4 fragColor` (or use the mainImage
   wrapper instead).
7. Declare the ISF input uniforms, the runtime macros and the per-buffer
   uniforms (below).

Remaining errors after this rewrite for the three probed shaders are only
`UnknownVariable` for ISF inputs/macros — i.e. the rewrite itself works.

## The runtime's macro set (from `ISFParser.fragmentShaderSkeleton` etc.)

```glsl
uniform int PASSINDEX;
uniform vec2 RENDERSIZE;
varying vec2 isf_FragNormCoord;   // 0..1
varying vec2 isf_FragCoord;       // pixel space
uniform float TIME;               // seconds since engine start
uniform float TIMEDELTA;
uniform int FRAMEINDEX;
uniform vec4 DATE;                // [year, month 1-based, day, secs since midnight]
```

Vertex skeleton injects `void isf_vertShaderInit()` + per-buffer uniforms.
`isf_vertShaderInit` sets `isf_position` (attribute, -1..1), `gl_Position`,
`isf_FragNormCoord`, `isf_FragCoord = floor(norm * RENDERSIZE)`.

TIME/TIMEDELTA/FRAMEINDEX/DATE are also set every frame by
`ISFRenderer.setDateUniforms` (`TIME = (now - startTime)/1000`, startTime =
launch; `DATE.y` is **1-based** month; `DATE.w` = secs since midnight).

Per `PASSES` buffer `<name>`:
`uniform sampler2D _<name>_img;` plus
`uniform vec2 _<name>_imgSize;`,
`uniform vec4 _<name>_imgRect;` (`[0,0,1,1]`),
`uniform bool _<name>_flip;` (`false`).

`IMG_PIXEL/IMG_NORM_PIXEL/IMG_SIZE/IMG_THIS_PIXEL/IMG_THIS_NORM_PIXEL` macros
expand (ISFParser.js ~lines 144-260) to texture2D with the corresponding
`_<name>_imgSize/rect/flip` uniforms; `IMG_SIZE(name)` is a vec2 size. The
final fragment pass runs against the destination framebuffer (not a texture).

## FLOAT / PERSISTENT buffers

`ISFTexture.setSize` allocates unsized `RGBA` (line ~45) — rejected by WebGL2;
modV monkey-patches it to `RGBA32F` + NEAREST (float-buffer-support.js).
The Rust runtime allocates `Rgba32Float` with `FilterMode::Nearest` for
`FLOAT: true` + `PERSISTENT: true` buffers natively.

## Pass evaluation

`PASSES` entries carry `TARGET` (name), `WIDTH`/`HEIGHT` as string formulas
(`"max(floor($WIDTH*0.25),1.0)"`), `PERSISTENT`, `FLOAT`, `CLEAR`.
`ISFRenderer.evaluateSize` expands `$WIDTH`/`$HEIGHT` and the current pass
values (`$PASSINDEX`-like `$<input>` for uniform values) via mathjs eval.
Empty `{}` entry = implicit final pass to the destination.

## ISF inputs → modV props (from renderers/isf/index.js:181-281)

| ISF TYPE | prop      | synthesis |
| -------- | --------- | --------- |
| float    | float     | default 0.0, min/max/step 0.01 |
| bool     | bool      | Boolean(DEFAULT) |
| long     | enum      | VALUES + LABELS, default via `selected` |
| color    | vec4      | DEFAULT object |
| point2D  | vec2      | default [0,0] |
| image    | texture   | + `meta.previewWithOutput = true` |
| event    | event     | — |

`CREDIT`/`DESCRIPTION`/`VSN` → `meta.author`/`description`/`version`;
`MODV_CATEGORY` normalized via CATEGORY_ALIASES → Gallery category.

## UnknownVariable (`TIME`) in the corpus

65 shaders use `TIME` with no declaration — it is **always** runtime-injected
(see skeleton). Similarly `restartNow`-style event inputs, ISF input uniforms,
and `PASSINDEX`/`FRAMEINDEX`/`DATE`/`TIMEDELTA`.

## Translator verdict (probe)

After the ES→450 rewrite, representative shaders (Bloom, Life, badtv) fail
only on `UnknownVariable` — i.e. the pipeline works end to end once the
preprocessor emits the full macro set + input uniforms. Expect the corpus to
translate at a high rate; the multi-pass + PERSISTENT paths are separate
(63 `PASSES` multi-pass shaders; 43 `PERSISTENT`; 16 `FLOAT`).
The one known-gap area: shaders declaring `uniform` for inputs the JSON
header also declares with `DEFAULT` values (overlapping declarations are
harmless).
