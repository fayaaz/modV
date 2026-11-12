//! GPU tests for the post-processing chain.
//!
//! Rendered on the fallback (software / lavapipe) adapter and read back, in the
//! style of `crates/modv-render/src/graph_tests.rs`. Every test is skipped with
//! a log line when there is no adapter.

use crate::point10::FULLSCREEN_TRIANGLE_WGSL;
use crate::point10::test_support::{TestTarget, any_lit, any_visible, gpu};

use super::Composer;

const SIZE: u32 = 64;

/// A minimal stand-in for a scene: fills the target with one colour, optionally
/// limited to a scissored patch.
struct SolidScene {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    queue: wgpu::Queue,
    colour: [f32; 4],
    patch: Option<(u32, u32, u32, u32)>,
}

impl SolidScene {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        colour: [f32; 4],
        patch: Option<(u32, u32, u32, u32)>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-test-solid-scene"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{FULLSCREEN_TRIANGLE_WGSL}\n\
                     struct Solid {{ colour: vec4<f32> }}\n\
                     @group(0) @binding(0) var<uniform> solid: Solid;\n\
                     @fragment fn fs_main() -> @location(0) vec4<f32> {{ return solid.colour; }}\n"
                )
                .into(),
            ),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-test-solid-scene-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-test-solid-scene-uniforms"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&colour));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-test-solid-scene-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-test-solid-scene-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-test-solid-scene-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            bind_group,
            uniform_buffer,
            queue: queue.clone(),
            colour,
            patch,
        }
    }

    fn render(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        // Keep the buffer referenced for clarity (values never change).
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.colour));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("point10-test-solid-scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        if let Some((x, y, width, height)) = self.patch {
            pass.set_scissor_rect(x, y, width, height);
        }
        pass.draw(0..3, 0..1);
    }
}

/// Render one frame through the composer and read the destination back.
fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    composer: &mut Composer,
    scene: &SolidScene,
) -> TestTarget {
    let target = TestTarget::new(device, SIZE, SIZE);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("point10-postfx-test"),
    });
    scene.render(&mut encoder, composer.scene_view());
    composer.render(&mut encoder, &target.view);
    queue.submit(Some(encoder.finish()));
    target
}

#[test]
fn composer_renders_a_solid_colour_scene() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut composer = Composer::new(&device, &queue, SIZE, SIZE, wgpu::TextureFormat::Rgba8Unorm);
    // Linear red; the composer's output pass applies the sRGB transfer for a
    // non-sRGB destination, so the pixel reads back as (255, 0, 0, 255).
    let scene = SolidScene::new(
        &device,
        &queue,
        Composer::scene_format(),
        [1.0, 0.0, 0.0, 1.0],
        None,
    );

    let target = render_frame(&device, &queue, &mut composer, &scene);
    let pixels = target.read(&device, &queue);
    assert!(any_lit(&pixels), "the composer produced an empty frame");

    let centre = target.pixel(&device, &queue, SIZE / 2, SIZE / 2);
    assert!(
        centre[0] > 240 && centre[1] < 8 && centre[2] < 8 && centre[3] > 240,
        "expected opaque red, got {centre:?}"
    );
}

#[test]
fn composer_preserves_transparency() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut composer = Composer::new(&device, &queue, SIZE, SIZE, wgpu::TextureFormat::Rgba8Unorm);
    // Point10 scenes clear to transparent black; the composer must not invent
    // an opaque background (the scene composites over the layers below).
    let scene = SolidScene::new(
        &device,
        &queue,
        Composer::scene_format(),
        [0.0, 0.0, 0.0, 0.0],
        None,
    );
    let target = render_frame(&device, &queue, &mut composer, &scene);
    let pixels = target.read(&device, &queue);
    assert!(!any_visible(&pixels), "expected a fully transparent frame");
}

#[test]
fn bloom_spreads_energy_to_neighbours() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    // A 4x4 white patch at the centre; everything else is transparent black.
    let patch = (30, 30, 4, 4);
    let scene = SolidScene::new(
        &device,
        &queue,
        Composer::scene_format(),
        [1.0, 1.0, 1.0, 1.0],
        Some(patch),
    );

    let neighbour = (SIZE / 2 + 6, SIZE / 2);

    let mut plain = Composer::new(&device, &queue, SIZE, SIZE, wgpu::TextureFormat::Rgba8Unorm);
    let without = render_frame(&device, &queue, &mut plain, &scene);
    let without_pixel = without.pixel(&device, &queue, neighbour.0, neighbour.1);

    let mut bloomed = Composer::new(&device, &queue, SIZE, SIZE, wgpu::TextureFormat::Rgba8Unorm);
    bloomed.enable_bloom(1.0, 0.5, 0.0);
    let with = render_frame(&device, &queue, &mut bloomed, &scene);
    let with_pixel = with.pixel(&device, &queue, neighbour.0, neighbour.1);

    assert!(
        without_pixel[3] == 0 || without_pixel[0] == 0,
        "the un-bloomed neighbour should be empty, got {without_pixel:?}"
    );
    assert!(
        with_pixel[0] > 8,
        "bloom should light the neighbour: {with_pixel:?} vs {without_pixel:?}"
    );
    // The patch itself must stay bright.
    let centre = with.pixel(&device, &queue, SIZE / 2, SIZE / 2);
    assert!(
        centre[0] > 240,
        "the bloom must not darken the source: {centre:?}"
    );
}

/// `preserveBloomAlpha` (Boxy.js:73-86, used by Wormhole): the bloom still
/// lights the neighbours, but it must not add into the frame's alpha.
#[test]
fn preserve_bloom_alpha_keeps_the_destination_alpha() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    // A 4x4 white patch with a transparent frame: the same setup as
    // `bloom_spreads_energy_to_neighbours`.
    let patch = (30, 30, 4, 4);
    let scene = SolidScene::new(
        &device,
        &queue,
        Composer::scene_format(),
        [1.0, 1.0, 1.0, 1.0],
        Some(patch),
    );
    let neighbour = (SIZE / 2 + 6, SIZE / 2);

    let mut additive = Composer::new(&device, &queue, SIZE, SIZE, wgpu::TextureFormat::Rgba8Unorm);
    additive.enable_bloom(1.0, 0.5, 0.0);
    let additive_frame = render_frame(&device, &queue, &mut additive, &scene);
    let additive_pixel = additive_frame.pixel(&device, &queue, neighbour.0, neighbour.1);

    let mut preserved = Composer::new(&device, &queue, SIZE, SIZE, wgpu::TextureFormat::Rgba8Unorm);
    preserved.enable_bloom(1.0, 0.5, 0.0);
    preserved.set_preserve_bloom_alpha(true);
    assert!(preserved.preserve_bloom_alpha());
    let preserved_frame = render_frame(&device, &queue, &mut preserved, &scene);
    let preserved_pixel = preserved_frame.pixel(&device, &queue, neighbour.0, neighbour.1);

    // Both add the bloom colour...
    assert!(
        additive_pixel[0] > 8 && preserved_pixel[0] > 8,
        "bloom should light the neighbour in both modes: {additive_pixel:?} / {preserved_pixel:?}"
    );
    // ...but only the default mode adds it into the alpha.
    assert!(
        additive_pixel[3] > 0,
        "the additive combine is expected to raise alpha: {additive_pixel:?}"
    );
    assert_eq!(
        preserved_pixel[3], 0,
        "preserveBloomAlpha must keep the transparent background: {preserved_pixel:?}"
    );
}

#[test]
fn composer_resize_rebuilds_the_chain() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut composer = Composer::new(&device, &queue, SIZE, SIZE, wgpu::TextureFormat::Rgba8Unorm);
    composer.enable_bloom(1.0, 0.5, 0.2);
    composer.resize(SIZE * 2, SIZE * 2);
    assert_eq!(composer.size(), (SIZE * 2, SIZE * 2));

    let scene = SolidScene::new(
        &device,
        &queue,
        Composer::scene_format(),
        [0.2, 0.4, 0.6, 1.0],
        None,
    );
    let target = TestTarget::new(&device, SIZE * 2, SIZE * 2);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("point10-postfx-resize-test"),
    });
    scene.render(&mut encoder, composer.scene_view());
    composer.render(&mut encoder, &target.view);
    queue.submit(Some(encoder.finish()));

    let pixel = target.pixel(&device, &queue, SIZE, SIZE);
    assert!(
        pixel[3] > 240,
        "resized composer lost its output: {pixel:?}"
    );
    assert!(
        pixel[0] > 60 && pixel[1] > 90 && pixel[2] > 130,
        "expected a bright blue-grey, got {pixel:?}"
    );
}
