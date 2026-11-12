//! The 28 composite/blend tokens exposed by the app.
//!
//! Ported from `src/renderer/src/util/composite-operations.js`:
//!
//! - **15 "Blend Modes" tokens**: `normal`, `multiply`, `overlay`, `darken`,
//!   `lighten`, `color-dodge`, `color-burn`, `hard-light`, `soft-light`,
//!   `difference`, `exclusion`, `hue`, `saturation`, `color`, `luminosity`
//!   (returned by [`BlendMode::is_separable`], matching the JS section
//!   grouping — W3C would call the last four "non-separable").
//! - **13 Porter-Duff composite tokens**: `clear`, `copy`, `destination`,
//!   `source-over`, `destination-over`, `source-in`, `destination-in`,
//!   `source-out`, `destination-out`, `source-atop`, `destination-atop`,
//!   `xor`, `lighter`.
//!
//! ### Fixed-function vs shader routing
//!
//! [`BlendMode::is_fixed_function`] is `true` for the 13 Porter-Duff modes
//! plus `normal`: each reduces to a single `wgpu::BlendState`
//! `{src factor, dst factor, operation}` with constant factors
//! (`Zero/One/SrcColor/DstColor/SrcAlpha/DstAlpha/OneMinus*`, `Add`):
//!
//! - `normal` ~ source-over: premultiplied `(One, OneMinusSrcAlpha, Add)`.
//! - Porter-Duff: straight-alpha factors, e.g. `source-in` =
//!   `(DstAlpha, Zero, Add)`, `xor` = `(OneMinusDstAlpha, OneMinusSrcAlpha, Add)`,
//!   `clear`/`copy`/`destination` use `Zero`/`One` mixes; all are constant
//!   factors.
//!
//! The other 11 blend modes are **not** expressible as one fixed-function
//! multiply-add (`darken`/`lighten` want `Min`/`Max` operations on the color
//! channels but composited alpha stays source-over, and the rest want
//! per-channel conditional math). They route through `blend.wgsl` together
//! with the non-separable four.
//!
//! [`BlendMode::needs_shader_pass`] flags the four W3C *non-separable*
//! modes (`hue`, `saturation`, `color`, `luminosity`), which cannot be
//! expressed as separable per-channel curves at all and need the HSL/Lum
//! math in `blend.wgsl`. The remaining 10 separable modes also require a
//! shader pass in practice (separable blend curves); they return `false`
//! from `needs_shader_pass` only because that method tracks the
//! non-separable W3C routing specifically. Callers should treat
//! `!is_fixed_function()` as "needs a shader pass"; `needs_shader_pass()`
//! narrows to the non-separable four.

use serde::{Deserialize, Serialize};

/// One of the 28 blend/composite tokens from `composite-operations.js`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlendMode {
    // —— Blend Modes (the "Blend Modes" section in composite-operations.js) ——
    /// `normal` — source-over.
    Normal,
    /// `multiply`.
    Multiply,
    /// `overlay`.
    Overlay,
    /// `darken`.
    Darken,
    /// `lighten`.
    Lighten,
    /// `color-dodge`.
    ColorDodge,
    /// `color-burn`.
    ColorBurn,
    /// `hard-light`.
    HardLight,
    /// `soft-light`.
    SoftLight,
    /// `difference`.
    Difference,
    /// `exclusion`.
    Exclusion,
    /// `hue` — non-separable W3C blend mode; needs `blend.wgsl`.
    Hue,
    /// `saturation` — non-separable W3C blend mode; needs `blend.wgsl`.
    Saturation,
    /// `color` — non-separable W3C blend mode; needs `blend.wgsl`.
    Color,
    /// `luminosity` — non-separable W3C blend mode; needs `blend.wgsl`.
    Luminosity,
    // —— Porter-Duff composite modes ("Composite Modes" section) ——
    /// `clear` — src `Zero`, dst `Zero`.
    Clear,
    /// `copy` — src `One`, dst `Zero`.
    Copy,
    /// `destination` — src `Zero`, dst `One`.
    Destination,
    /// `source-over` — src `One`, dst `OneMinusSrcAlpha`.
    SourceOver,
    /// `destination-over` — src `OneMinusDstAlpha`, dst `One`.
    DestinationOver,
    /// `source-in` — src `DstAlpha`, dst `Zero`.
    SourceIn,
    /// `destination-in` — src `Zero`, dst `SrcAlpha`.
    DestinationIn,
    /// `source-out` — src `OneMinusDstAlpha`, dst `Zero`.
    SourceOut,
    /// `destination-out` — src `Zero`, dst `OneMinusSrcAlpha`.
    DestinationOut,
    /// `source-atop` — src `DstAlpha`, dst `OneMinusSrcAlpha`.
    SourceAtop,
    /// `destination-atop` — src `OneMinusDstAlpha`, dst `SrcAlpha`.
    DestinationAtop,
    /// `xor` — src `OneMinusDstAlpha`, dst `OneMinusSrcAlpha`.
    Xor,
    /// `lighter` — additive: src `One`, dst `One`.
    Lighter,
}

/// Every blend mode token, in the order `composite-operations.js` lists them.
pub const ALL_BLEND_MODES: &[BlendMode] = &[
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::HardLight,
    BlendMode::SoftLight,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::Clear,
    BlendMode::Copy,
    BlendMode::Destination,
    BlendMode::SourceOver,
    BlendMode::DestinationOver,
    BlendMode::SourceIn,
    BlendMode::DestinationIn,
    BlendMode::SourceOut,
    BlendMode::DestinationOut,
    BlendMode::SourceAtop,
    BlendMode::DestinationAtop,
    BlendMode::Xor,
    BlendMode::Lighter,
];

/// An unrecognised blend mode token.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown blend mode: {0:?}")]
pub struct UnknownBlendMode(pub String);

impl BlendMode {
    /// Parse a blend mode token from `composite-operations.js`.
    pub fn from_str(s: &str) -> Result<BlendMode, UnknownBlendMode> {
        match s {
            "normal" => Ok(BlendMode::Normal),
            "multiply" => Ok(BlendMode::Multiply),
            "overlay" => Ok(BlendMode::Overlay),
            "darken" => Ok(BlendMode::Darken),
            "lighten" => Ok(BlendMode::Lighten),
            "color-dodge" => Ok(BlendMode::ColorDodge),
            "color-burn" => Ok(BlendMode::ColorBurn),
            "hard-light" => Ok(BlendMode::HardLight),
            "soft-light" => Ok(BlendMode::SoftLight),
            "difference" => Ok(BlendMode::Difference),
            "exclusion" => Ok(BlendMode::Exclusion),
            "hue" => Ok(BlendMode::Hue),
            "saturation" => Ok(BlendMode::Saturation),
            "color" => Ok(BlendMode::Color),
            "luminosity" => Ok(BlendMode::Luminosity),
            "clear" => Ok(BlendMode::Clear),
            "copy" => Ok(BlendMode::Copy),
            "destination" => Ok(BlendMode::Destination),
            "source-over" => Ok(BlendMode::SourceOver),
            "destination-over" => Ok(BlendMode::DestinationOver),
            "source-in" => Ok(BlendMode::SourceIn),
            "destination-in" => Ok(BlendMode::DestinationIn),
            "source-out" => Ok(BlendMode::SourceOut),
            "destination-out" => Ok(BlendMode::DestinationOut),
            "source-atop" => Ok(BlendMode::SourceAtop),
            "destination-atop" => Ok(BlendMode::DestinationAtop),
            "xor" => Ok(BlendMode::Xor),
            "lighter" => Ok(BlendMode::Lighter),
            other => Err(UnknownBlendMode(other.to_string())),
        }
    }

    /// The canonical token, exactly as used in presets and
    /// `composite-operations.js`.
    pub fn as_str(&self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Multiply => "multiply",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::ColorDodge => "color-dodge",
            BlendMode::ColorBurn => "color-burn",
            BlendMode::HardLight => "hard-light",
            BlendMode::SoftLight => "soft-light",
            BlendMode::Difference => "difference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Color => "color",
            BlendMode::Luminosity => "luminosity",
            BlendMode::Clear => "clear",
            BlendMode::Copy => "copy",
            BlendMode::Destination => "destination",
            BlendMode::SourceOver => "source-over",
            BlendMode::DestinationOver => "destination-over",
            BlendMode::SourceIn => "source-in",
            BlendMode::DestinationIn => "destination-in",
            BlendMode::SourceOut => "source-out",
            BlendMode::DestinationOut => "destination-out",
            BlendMode::SourceAtop => "source-atop",
            BlendMode::DestinationAtop => "destination-atop",
            BlendMode::Xor => "xor",
            BlendMode::Lighter => "lighter",
        }
    }

    /// `true` for the 15 tokens in the JS "Blend Modes" section
    /// (the spec's "separable" set): all blend modes except the 13
    /// Porter-Duff composite modes.
    pub fn is_separable(&self) -> bool {
        matches!(
            self,
            BlendMode::Normal
                | BlendMode::Multiply
                | BlendMode::Overlay
                | BlendMode::Darken
                | BlendMode::Lighten
                | BlendMode::ColorDodge
                | BlendMode::ColorBurn
                | BlendMode::HardLight
                | BlendMode::SoftLight
                | BlendMode::Difference
                | BlendMode::Exclusion
                | BlendMode::Hue
                | BlendMode::Saturation
                | BlendMode::Color
                | BlendMode::Luminosity
        )
    }

    /// `true` when the mode reduces to one constant-factor `wgpu::BlendState`
    /// (`{ src, dst, Add }`): the 13 Porter-Duff modes plus `normal`.
    ///
    /// The other 11 blend modes are separable-but-conditional and route
    /// through `blend.wgsl`; see module docs.
    pub fn is_fixed_function(&self) -> bool {
        matches!(
            self,
            BlendMode::Normal
                | BlendMode::Clear
                | BlendMode::Copy
                | BlendMode::Destination
                | BlendMode::SourceOver
                | BlendMode::DestinationOver
                | BlendMode::SourceIn
                | BlendMode::DestinationIn
                | BlendMode::SourceOut
                | BlendMode::DestinationOut
                | BlendMode::SourceAtop
                | BlendMode::DestinationAtop
                | BlendMode::Xor
                | BlendMode::Lighter
        )
    }

    /// `true` for the four W3C non-separable blend modes that require the
    /// shader pass (`blend.wgsl`): `hue`, `saturation`, `color`,
    /// `luminosity`. These cannot be expressed as separable per-channel
    /// curves. The remaining separable modes also need shader routing in
    /// practice — check [`BlendMode::is_fixed_function`] for that.
    pub fn needs_shader_pass(&self) -> bool {
        matches!(
            self,
            BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity
        )
    }

    /// `true` for the 13 Porter-Duff composite tokens (the JS
    /// "Composite Modes" section).
    pub fn is_porter_duff(&self) -> bool {
        !self.is_separable()
    }
}

impl std::fmt::Display for BlendMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for BlendMode {
    type Err = UnknownBlendMode;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        BlendMode::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact tokens from composite-operations.js, in order.
    const SEPARABLE_TOKENS: [&str; 15] = [
        "normal",
        "multiply",
        "overlay",
        "darken",
        "lighten",
        "color-dodge",
        "color-burn",
        "hard-light",
        "soft-light",
        "difference",
        "exclusion",
        "hue",
        "saturation",
        "color",
        "luminosity",
    ];

    const PORTER_DUFF_TOKENS: [&str; 13] = [
        "clear",
        "copy",
        "destination",
        "source-over",
        "destination-over",
        "source-in",
        "destination-in",
        "source-out",
        "destination-out",
        "source-atop",
        "destination-atop",
        "xor",
        "lighter",
    ];

    #[test]
    fn all_28_tokens_parse_and_round_trip() {
        assert_eq!(ALL_BLEND_MODES.len(), 28);

        for token in SEPARABLE_TOKENS.iter().chain(PORTER_DUFF_TOKENS.iter()) {
            let mode = BlendMode::from_str(token)
                .unwrap_or_else(|e| panic!("token {token} should parse: {e}"));
            assert_eq!(mode.as_str(), *token);
            assert_eq!(BlendMode::from_str(mode.as_str()).unwrap(), mode);
            assert_eq!(mode.to_string(), *token);
            assert_eq!(
                BlendMode::from_str(&token.to_string())
                    .map(|m| m.as_str().to_string())
                    .unwrap(),
                token.to_string()
            );
        }
    }

    #[test]
    fn unknown_token_errors_with_the_token() {
        let err = BlendMode::from_str("screen").unwrap_err();
        assert_eq!(err.0, "screen");
        assert_eq!(err.to_string(), "unknown blend mode: \"screen\"");

        assert!(BlendMode::from_str("").is_err());
        assert!(BlendMode::from_str("NORMAL").is_err());
        assert!(BlendMode::from_str("source_over").is_err());
    }

    #[test]
    fn separable_set_is_the_15_blend_mode_tokens() {
        let separable: Vec<&str> = ALL_BLEND_MODES
            .iter()
            .filter(|m| m.is_separable())
            .map(|m| m.as_str())
            .collect();
        assert_eq!(separable, SEPARABLE_TOKENS);

        for token in PORTER_DUFF_TOKENS {
            let mode = BlendMode::from_str(token).unwrap();
            assert!(!mode.is_separable());
            assert!(mode.is_porter_duff());
        }
    }

    #[test]
    fn fixed_function_is_porter_duff_plus_normal() {
        let fixed: Vec<&str> = ALL_BLEND_MODES
            .iter()
            .filter(|m| m.is_fixed_function())
            .map(|m| m.as_str())
            .collect();
        let mut expected = vec!["normal"];
        expected.extend(PORTER_DUFF_TOKENS);
        assert_eq!(fixed, expected);
        assert_eq!(fixed.len(), 14);

        for token in [
            "multiply",
            "overlay",
            "darken",
            "lighten",
            "color-dodge",
            "color-burn",
            "hard-light",
            "soft-light",
            "difference",
            "exclusion",
            "hue",
            "saturation",
            "color",
            "luminosity",
        ] {
            assert!(!BlendMode::from_str(token).unwrap().is_fixed_function());
        }
    }

    #[test]
    fn needs_shader_pass_is_the_non_separable_four() {
        let shader: Vec<&str> = ALL_BLEND_MODES
            .iter()
            .filter(|m| m.needs_shader_pass())
            .map(|m| m.as_str())
            .collect();
        assert_eq!(shader, vec!["hue", "saturation", "color", "luminosity"]);
    }

    #[test]
    fn every_token_is_in_exactly_one_section() {
        for mode in ALL_BLEND_MODES {
            assert_ne!(mode.is_separable(), mode.is_porter_duff());
        }
    }

    #[test]
    fn serde_kebab_case_matches_tokens() {
        for mode in ALL_BLEND_MODES {
            let value = serde_json::to_value(mode).unwrap();
            assert_eq!(value, serde_json::Value::String(mode.as_str().to_string()));
            let parsed: BlendMode = serde_json::from_value(value).unwrap();
            assert_eq!(parsed, *mode);
        }
    }
}
