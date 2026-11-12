//! `Texture2d.js` — draws the module's input texture with crop/scale/constrain.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Texture2d.js`.
//! The JS gates on `props.texture.value` (`:92`) and draws `texture.value`
//! with the 9-argument `drawImage`; here the engine supplies that image as
//! [`DrawContext::input_texture`] + [`DrawContext::input_size`] (M6 fills it
//! from the media manager). The crop math (`:95-163`) is transcribed
//! unchanged; pixel rectangles are normalised for the facade's
//! `draw_image_region`.

use crate::two_d::modules::{
    DrawContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_f64, prop_i64, prop_str,
};

/// The `Texture 2D` module.
#[derive(Default)]
pub struct Texture2d;

impl TwoDModule for Texture2d {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Texture 2D",
            author: "NERDDISCO",
            version: "1.0.0",
            category: None,
            audio_features: &[],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::texture("texture"),
            TwoDProp::int("cropTop", 0)
                .label("Crop Top (px)")
                .range(0.0, 4096.0)
                .step(1.0),
            TwoDProp::int("cropRight", 0)
                .label("Crop Right (px)")
                .range(0.0, 4096.0)
                .step(1.0),
            TwoDProp::int("cropBottom", 0)
                .label("Crop Bottom (px)")
                .range(0.0, 4096.0)
                .step(1.0),
            TwoDProp::int("cropLeft", 0)
                .label("Crop Left (px)")
                .range(0.0, 4096.0)
                .step(1.0),
            TwoDProp::float("scale", 1.0)
                .label("Scale")
                .range(0.0, 5.0)
                .step(0.001),
            TwoDProp::float("offsetX", 0.0)
                .label("Offset X in %")
                .range(-100.0, 100.0)
                .step(1.0),
            TwoDProp::float("offsetY", 0.0)
                .label("Offset Y in %")
                .range(-100.0, 100.0)
                .step(1.0),
            TwoDProp::enum_prop(
                "constrain",
                "none",
                &[
                    ("None (Scale)", "none"),
                    ("Contain", "contain"),
                    ("Cover", "cover"),
                ],
            ),
        ]
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Texture2d.js:92`: an unset texture draws nothing.
        let Some((view, (image_pixel_width, image_pixel_height))) = ctx.input() else {
            return;
        };
        let constrain = prop_str(ctx.props, "constrain", "none");
        let offset_x = prop_f64(ctx.props, "offsetX", 0.0) as f32;
        let offset_y = prop_f64(ctx.props, "offsetY", 0.0) as f32;
        let scale = prop_f64(ctx.props, "scale", 1.0) as f32;

        let image_width_px = image_pixel_width as i64;
        let image_height_px = image_pixel_height as i64;
        // `Texture2d.js:95-104`: clamp the crop to the image bounds.
        let clamped_top = prop_i64(ctx.props, "cropTop", 0).clamp(0, image_height_px - 1);
        let clamped_bottom = prop_i64(ctx.props, "cropBottom", 0).clamp(0, image_height_px - 1);
        let clamped_left = prop_i64(ctx.props, "cropLeft", 0).clamp(0, image_width_px - 1);
        let clamped_right = prop_i64(ctx.props, "cropRight", 0).clamp(0, image_width_px - 1);

        let sx = clamped_left as f32;
        let sy = clamped_top as f32;
        let source_width = (image_width_px - clamped_left - clamped_right).max(1) as f32;
        let source_height = (image_height_px - clamped_top - clamped_bottom).max(1) as f32;

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;

        // `Texture2d.js:106-147`: destination size from the CROPPED source
        // aspect ratio.
        let source_aspect = source_height / source_width;
        let (image_width, image_height, x, y) = if constrain == "contain" {
            let mut image_height = source_aspect * width;
            let mut image_width = width;
            let mut y = (height - image_height) / 2.0;
            let mut x = 0.0;
            if image_height > height {
                image_width = (source_width / source_height) * height;
                image_height = height;
                y = 0.0;
                x = (width - image_width) / 2.0;
            }
            (image_width, image_height, x, y)
        } else if constrain == "cover" {
            let canvas_ratio = height / width;
            if source_aspect < canvas_ratio {
                // Make height fill canvas.
                let image_height = height;
                let image_width = height / source_aspect;
                (image_width, image_height, (width - image_width) / 2.0, 0.0)
            } else {
                // Make width fill canvas.
                let image_width = width;
                let image_height = width * source_aspect;
                (
                    image_width,
                    image_height,
                    0.0,
                    (height - image_height) / 2.0,
                )
            }
        } else {
            let image_width = source_width * scale;
            let image_height = source_height * scale;
            (
                image_width,
                image_height,
                (width - image_width) / 2.0,
                (height - image_height) / 2.0,
            )
        };

        let calculated_offset_x = (width / 100.0) * offset_x;
        let calculated_offset_y = (height / 100.0) * offset_y;

        // `Texture2d.js:152-163`.
        ctx.canvas.draw_image_region(
            view,
            sx / image_width_px as f32,
            sy / image_height_px as f32,
            source_width / image_width_px as f32,
            source_height / image_height_px as f32,
            x + calculated_offset_x,
            y + calculated_offset_y,
            image_width,
            image_height,
        );
    }
}
