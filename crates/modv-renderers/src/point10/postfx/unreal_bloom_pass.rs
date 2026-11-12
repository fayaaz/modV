//! UnrealBloomPass — Rust/wgpu port of three's
//! `examples/jsm/postprocessing/UnrealBloomPass.js`.
//!
//! # The three.js chain, and the port's chain
//!
//! three's pass, in `render()` (`UnrealBloomPass.js:292-424`):
//!
//! 1. **Bright pass** — `LuminosityHighPassShader` into a half-resolution
//!    target: `mix(black, texel, smoothstep(threshold, threshold + 0.01, luma))`.
//! 2. **Five blur mip levels** — each level does a horizontal then a vertical
//!    separable gaussian into `renderTargetsHorizontal[i]` /
//!    `renderTargetsVertical[i]`, at half, quarter, … resolution
//!    (`setSize`, `:246-256`). The next level blurs the previous level's
//!    vertical output, so the mips are progressively blurrier *and* smaller.
//! 3. **Composite** — a weighted sum of the five vertical textures:
//!    `3 * strength * Σ lerpBloomFactor(factor_i) * tint_i * tex_i`, where
//!    `lerpBloomFactor(f) = mix(f, 1.2 - f, radius)`. Written at half
//!    resolution (into `renderTargetsHorizontal[0]`).
//! 4. **Additive blend** — the composite is added onto the input (`ONE, ONE`).
//!    In this port step 4 is the composer's combine pass, because wgpu cannot
//!    sample and render into one texture.
//!
//! # Kernel
//!
//! `kernelSizeArray = [6, 10, 14, 18, 22]` with `sigma = radius / 3`, then
//! adjacent taps merged into bilinear fetches (`_getSeparableBlurMaterial`,
//! `:531-604`). [`kernel_pairs`] is a literal port of that arithmetic, so the
//! blur radius and falloff match three exactly; only the delivery differs (a
//! uniform `array<vec4<f32>, 11>` of `(offset, weight)` instead of
//! `#define KERNEL_PAIRS` and GLSL uniform arrays — WGSL uniform arrays need
//! 16-byte strides).
//!
//! # Uniforms
//!
//! `strength` / `radius` / `threshold` keep three's names and ranges: the
//! constructor defaults are three's `(1, 0, 0)`; the scenes that build a pass
//! pass explicit values (e.g. `Mycelium.js:414`, `Wormhole.js:193`).

#![forbid(unsafe_code)]

use super::{create_target, half, linear_sampler};
use crate::point10::FULLSCREEN_TRIANGLE_WGSL;

/// `this.nMips = 5` (`UnrealBloomPass.js:83`).
pub const MIP_COUNT: usize = 5;
/// `const kernelSizeArray = [6, 10, 14, 18, 22]` (`UnrealBloomPass.js:147`).
const KERNEL_RADII: [usize; MIP_COUNT] = [6, 10, 14, 18, 22];
/// Maximum merged taps — radius 22 yields 11 pairs.
const MAX_PAIRS: usize = 11;
/// `const bloomFactors = [1.0, 0.8, 0.6, 0.4, 0.2]` (`UnrealBloomPass.js:190`).
const BLOOM_FACTORS: [f32; MIP_COUNT] = [1.0, 0.8, 0.6, 0.4, 0.2];
/// `bloomTintColors` is white for every mip (`UnrealBloomPass.js:191`).
const BLOOM_TINTS: [[f32; 3]; MIP_COUNT] = [
    [1.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
];

/// Bright-pass uniforms (`bloom_bright.wgsl`), 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct BrightUniforms {
    /// `1 / bright target size` — the uv scale of the half-res pass.
    inv_size: [f32; 2],
    threshold: f32,
    smooth_width: f32,
}

/// Blur uniforms (`bloom_blur.wgsl`): header + 11 `(offset, weight)` taps, 208 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurUniforms {
    inv_size: [f32; 2],
    center_weight: f32,
    pairs: u32,
    direction: [f32; 2],
    _pad: [f32; 2],
    taps: [[f32; 4]; MAX_PAIRS],
}

/// Composite uniforms (`bloom_composite.wgsl`): header + 5 `(factor, tint)` taps, 96 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct CompositeUniforms {
    bloom_strength: f32,
    bloom_radius: f32,
    /// Classic UnrealBloomPass composites `3 * strength`; the TSL `bloom()`
    /// node composites `strength` alone. See [`UnrealBloomPass::set_bloom_scale`].
    bloom_scale: f32,
    _pad: f32,
    taps: [[f32; 4]; MIP_COUNT],
}

/// One separable gaussian kernel: the centre weight plus the merged
/// `(offset, weight)` tap pairs.
///
/// Port of `_getSeparableBlurMaterial` (`UnrealBloomPass.js:531-585`):
///
/// ```js
/// const sigma = kernelRadius / 3;
/// for (let i = 0; i < kernelRadius; i++)
///   coefficients.push(0.39894 * Math.exp(-0.5 * i * i / (sigma * sigma)) / sigma);
/// for (let i = 1; i < kernelRadius; i += 2) {
///   const wa = coefficients[i];
///   const wb = (i + 1 < kernelRadius) ? coefficients[i + 1] : 0;
///   const w = wa + wb;
///   offsets.push((i * wa + (i + 1) * wb) / w);
///   weights.push(w);
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct SeparableKernel {
    /// `coefficients[0]`.
    pub center_weight: f32,
    /// `(offset, weight)` pairs — one bilinear fetch each.
    pub pairs: Vec<(f32, f32)>,
}

impl SeparableKernel {
    /// Build the kernel for `radius` (6, 10, 14, 18 or 22 in three).
    pub fn for_radius(radius: usize) -> Self {
        let radius = radius.max(1);
        let sigma = radius as f32 / 3.0;
        let coefficients: Vec<f32> = (0..radius)
            .map(|i| {
                let i = i as f32;
                0.39894 * (-0.5 * i * i / (sigma * sigma)).exp() / sigma
            })
            .collect();

        let mut pairs = Vec::with_capacity(radius.div_ceil(2));
        let mut i = 1;
        while i < radius {
            let wa = coefficients[i];
            let wb = if i + 1 < radius {
                coefficients[i + 1]
            } else {
                0.0
            };
            let weight = wa + wb;
            if weight > 0.0 {
                pairs.push(((i as f32 * wa + (i as f32 + 1.0) * wb) / weight, weight));
            }
            i += 2;
        }

        Self {
            center_weight: coefficients[0],
            pairs,
        }
    }
}

/// GPU-side bloom targets and the bind groups that feed them.
struct BloomTargets {
    bright: wgpu::TextureView,
    horizontal: Vec<wgpu::TextureView>,
    vertical: Vec<wgpu::TextureView>,
    composite: wgpu::TextureView,
    bright_bind: Option<wgpu::BindGroup>,
    blur_horizontal: Vec<wgpu::BindGroup>,
    blur_vertical: Vec<wgpu::BindGroup>,
    composite_bind: wgpu::BindGroup,
}

/// The Unreal bloom pass.
pub struct UnrealBloomPass {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// `this.strength`.
    strength: f32,
    /// `this.radius`.
    radius: f32,
    /// The global composite multiplier (3.0 for the classic pass, 1.0 for the
    /// TSL `bloom()` node).
    bloom_scale: f32,
    /// `this.threshold`.
    threshold: f32,
    width: u32,
    height: u32,

    sampler: wgpu::Sampler,
    bright_pipeline: wgpu::RenderPipeline,
    bright_layout: wgpu::BindGroupLayout,
    blur_pipeline: wgpu::RenderPipeline,
    blur_layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,

    /// `1 / bright target size`, refreshed on resize.
    bright_inv_size: [f32; 2],
    bright_uniforms: wgpu::Buffer,
    blur_uniforms: Vec<wgpu::Buffer>,
    blur_vertical_uniforms: Vec<wgpu::Buffer>,
    composite_uniforms: wgpu::Buffer,

    targets: Option<BloomTargets>,
}

impl UnrealBloomPass {
    /// Build the pipelines and the mip targets for `width` x `height`.
    ///
    /// Defaults follow three's constructor (`UnrealBloomPass.js:50`):
    /// `strength = 1`, `radius = 0`, `threshold = 0`. Scenes that enable bloom
    /// set their own values.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let bright_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-bloom-bright-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{FULLSCREEN_TRIANGLE_WGSL}\n{}",
                    include_str!("bloom_bright.wgsl")
                )
                .into(),
            ),
        });
        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-bloom-blur-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{FULLSCREEN_TRIANGLE_WGSL}\n{}",
                    include_str!("bloom_blur.wgsl")
                )
                .into(),
            ),
        });
        let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-bloom-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{FULLSCREEN_TRIANGLE_WGSL}\n{}",
                    include_str!("bloom_composite.wgsl")
                )
                .into(),
            ),
        });

        // Bright pass: uniform + source texture + sampler.
        let bright_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-bloom-bright-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let bright_pipeline = fullscreen_pipeline(
            device,
            "point10-bloom-bright",
            &bright_shader,
            &bright_layout,
        );

        // Blur: uniform + source texture + sampler.
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-bloom-blur-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let blur_pipeline =
            fullscreen_pipeline(device, "point10-bloom-blur", &blur_shader, &blur_layout);

        // Composite: uniform + five blur mips + sampler.
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-bloom-composite-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                texture_entry(4),
                texture_entry(5),
                sampler_entry(6),
            ],
        });
        let composite_pipeline = fullscreen_pipeline(
            device,
            "point10-bloom-composite",
            &composite_shader,
            &composite_layout,
        );

        let bright_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-bloom-bright-uniforms"),
            size: std::mem::size_of::<BrightUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blur_uniforms: Vec<wgpu::Buffer> = (0..MIP_COUNT)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("point10-bloom-blur-h-uniforms"),
                    size: std::mem::size_of::<BlurUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let blur_vertical_uniforms: Vec<wgpu::Buffer> = (0..MIP_COUNT)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("point10-bloom-blur-v-uniforms"),
                    size: std::mem::size_of::<BlurUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let composite_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-bloom-composite-uniforms"),
            size: std::mem::size_of::<CompositeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut pass = Self {
            device: device.clone(),
            queue: queue.clone(),
            strength: 1.0,
            radius: 0.0,
            bloom_scale: 3.0,
            threshold: 0.0,
            width: width.max(1),
            height: height.max(1),
            sampler: linear_sampler(device, "point10-bloom-sampler"),
            bright_pipeline,
            bright_layout,
            blur_pipeline,
            blur_layout,
            composite_pipeline,
            composite_layout,
            bright_inv_size: [1.0, 1.0],
            bright_uniforms,
            blur_uniforms,
            blur_vertical_uniforms,
            composite_uniforms,
            targets: None,
        };
        pass.rebuild_targets();
        pass.upload_composite_uniforms();
        pass.upload_bright_uniforms();
        pass
    }

    /// `this.strength`.
    pub fn strength(&self) -> f32 {
        self.strength
    }

    /// Set `this.strength` (three's scenes use 0.4–1.5).
    pub fn set_strength(&mut self, strength: f32) {
        self.strength = strength;
        self.upload_composite_uniforms();
    }

    /// `this.radius` (0..1).
    pub fn radius(&self) -> f32 {
        self.radius
    }

    /// Set `this.radius` (0..1).
    pub fn set_radius(&mut self, radius: f32) {
        self.radius = radius.clamp(0.0, 1.0);
        self.upload_composite_uniforms();
    }

    /// Set the composite multiplier: 3.0 reproduces three's classic
    /// `UnrealBloomPass`, 1.0 the TSL `bloom()` node (`BloomNode` composites
    /// `sum * strength` with no multiplier, and the WebGPU scenes use it).
    pub fn set_bloom_scale(&mut self, scale: f32) {
        self.bloom_scale = scale;
        self.upload_composite_uniforms();
    }

    /// The composite multiplier.
    pub fn bloom_scale(&self) -> f32 {
        self.bloom_scale
    }

    /// `this.threshold`.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Set `this.threshold` (the bright-pass luma cut).
    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold;
        self.upload_bright_uniforms();
    }

    /// Upload the bright-pass uniforms (`threshold`, `smoothWidth = 0.01`,
    /// and the half-res uv scale).
    fn upload_bright_uniforms(&self) {
        self.queue.write_buffer(
            &self.bright_uniforms,
            0,
            bytemuck::bytes_of(&BrightUniforms {
                inv_size: self.bright_inv_size,
                threshold: self.threshold,
                // `smoothWidth = 0.01` (`UnrealBloomPass.js:135`)
                smooth_width: 0.01,
            }),
        );
    }

    /// Resize the mip chain (`setSize`, `UnrealBloomPass.js:246-256`).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.rebuild_targets();
    }

    /// Point the bright pass at the composer's HDR input target.
    ///
    /// Call after construction and after every [`UnrealBloomPass::resize`],
    /// because the composer re-creates its targets on resize.
    pub fn set_input(&mut self, input: &wgpu::TextureView) {
        let Some(targets) = self.targets.as_mut() else {
            return;
        };
        targets.bright_bind = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-bloom-bright-bind-group"),
            layout: &self.bright_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bright_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    /// The composited bloom texture (half resolution). The composer's combine
    /// pass adds this to the scene.
    pub fn composite_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|targets| &targets.composite)
    }

    /// Record the bright pass, the five blur levels and the composite.
    ///
    /// The caller must have called [`UnrealBloomPass::set_input`] with the
    /// scene/HDR texture.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        let Some(bright_bind) = targets.bright_bind.as_ref() else {
            return;
        };

        // 1. Extract bright areas (`UnrealBloomPass.js:355-366`).
        {
            let mut pass = begin_pass(encoder, "point10-bloom-bright", &targets.bright);
            pass.set_pipeline(&self.bright_pipeline);
            pass.set_bind_group(0, bright_bind, &[]);
            pass.draw(0..3, 0..1);
        }

        // 2. Blur all the mips progressively (`UnrealBloomPass.js:368-390`):
        // each level blurs the previous level's vertical output.
        for mip in 0..MIP_COUNT {
            let mut pass = begin_pass(encoder, "point10-bloom-blur-h", &targets.horizontal[mip]);
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &targets.blur_horizontal[mip], &[]);
            pass.draw(0..3, 0..1);
            drop(pass);

            let mut pass = begin_pass(encoder, "point10-bloom-blur-v", &targets.vertical[mip]);
            pass.set_pipeline(&self.blur_pipeline);
            pass.set_bind_group(0, &targets.blur_vertical[mip], &[]);
            pass.draw(0..3, 0..1);
        }

        // 3. Composite all the mips (`UnrealBloomPass.js:392-400`).
        let mut pass = begin_pass(encoder, "point10-bloom-composite", &targets.composite);
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, &targets.composite_bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Re-create the mip targets and their bind groups for the current size.
    fn rebuild_targets(&mut self) {
        let device = &self.device;
        let bright_size = (half(self.width), half(self.height));
        self.bright_inv_size = [1.0 / bright_size.0 as f32, 1.0 / bright_size.1 as f32];

        let (_, bright) =
            create_target(device, "point10-bloom-bright", bright_size.0, bright_size.1);
        let (_, composite) = create_target(
            device,
            "point10-bloom-composite",
            bright_size.0,
            bright_size.1,
        );

        let mut horizontal = Vec::with_capacity(MIP_COUNT);
        let mut vertical = Vec::with_capacity(MIP_COUNT);
        let mut sizes = Vec::with_capacity(MIP_COUNT);
        let mut size = bright_size;
        for _ in 0..MIP_COUNT {
            sizes.push(size);
            let (_, h) = create_target(device, "point10-bloom-h", size.0, size.1);
            let (_, v) = create_target(device, "point10-bloom-v", size.0, size.1);
            horizontal.push(h);
            vertical.push(v);
            size = (half(size.0), half(size.1));
        }

        // Per-mip blur uniforms: invSize = 1 / level size, direction ±x/±y,
        // plus the merged kernel for that level.
        let mut blur_horizontal = Vec::with_capacity(MIP_COUNT);
        let mut blur_vertical = Vec::with_capacity(MIP_COUNT);
        for (index, size) in sizes.iter().enumerate() {
            let kernel = SeparableKernel::for_radius(KERNEL_RADII[index]);
            let mut uniforms = BlurUniforms {
                inv_size: [1.0 / size.0 as f32, 1.0 / size.1 as f32],
                center_weight: kernel.center_weight,
                pairs: kernel.pairs.len() as u32,
                direction: [1.0, 0.0],
                _pad: [0.0; 2],
                taps: [[0.0; 4]; MAX_PAIRS],
            };
            for (pair_index, (offset, weight)) in kernel.pairs.iter().enumerate() {
                uniforms.taps[pair_index] = [*offset, *weight, 0.0, 0.0];
            }
            self.queue
                .write_buffer(&self.blur_uniforms[index], 0, bytemuck::bytes_of(&uniforms));

            // Vertical pass samples this level's horizontal output; the
            // direction is the only difference from the horizontal pass.
            let mut vertical_uniforms = uniforms;
            vertical_uniforms.direction = [0.0, 1.0];
            self.queue.write_buffer(
                &self.blur_vertical_uniforms[index],
                0,
                bytemuck::bytes_of(&vertical_uniforms),
            );

            // Horizontal pass samples the bright target (mip 0) or the
            // previous level's vertical output.
            let source = if index == 0 {
                &bright
            } else {
                &vertical[index - 1]
            };
            blur_horizontal.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("point10-bloom-blur-h-bind-group"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.blur_uniforms[index].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));

            blur_vertical.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("point10-bloom-blur-v-bind-group"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.blur_vertical_uniforms[index].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&horizontal[index]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
        }

        let composite_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-bloom-composite-bind-group"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.composite_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&vertical[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&vertical[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&vertical[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&vertical[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&vertical[4]),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        self.upload_bright_uniforms();

        self.targets = Some(BloomTargets {
            bright,
            horizontal,
            vertical,
            composite,
            bright_bind: None,
            blur_horizontal,
            blur_vertical,
            composite_bind,
        });
    }

    /// Upload the composite uniform block from `strength`/`radius`.
    fn upload_composite_uniforms(&self) {
        let mut uniforms = CompositeUniforms {
            bloom_strength: self.strength,
            bloom_radius: self.radius,
            bloom_scale: self.bloom_scale,
            _pad: 0.0,
            taps: [[0.0; 4]; MIP_COUNT],
        };
        for (index, factor) in BLOOM_FACTORS.iter().enumerate() {
            // `lerpBloomFactor`: `mix(factor, 1.2 - factor, bloomRadius)`
            let mirror = 1.2 - factor;
            let lerped = factor + (mirror - factor) * self.radius;
            uniforms.taps[index] = [
                lerped,
                BLOOM_TINTS[index][0],
                BLOOM_TINTS[index][1],
                BLOOM_TINTS[index][2],
            ];
        }
        self.queue
            .write_buffer(&self.composite_uniforms, 0, bytemuck::bytes_of(&uniforms));
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: crate::point10::HDR_FORMAT,
                // Bloom targets are written whole; no blending.
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    target: &wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                // three clears each bloom target before drawing
                // (`renderer.clear()` at UnrealBloomPass.js:363 etc.).
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_pairs_match_the_js_merge_rule() {
        let kernel = SeparableKernel::for_radius(6);
        // radius 6 -> i = 1, 3, 5 -> 3 pairs
        assert_eq!(kernel.pairs.len(), 3);
        // sigma = 2 -> coefficients[0] = 0.39894 / 2
        assert!((kernel.center_weight - 0.199_47).abs() < 1e-4);

        for (radius, pairs) in [(6, 3), (10, 5), (14, 7), (18, 9), (22, 11)] {
            assert_eq!(
                SeparableKernel::for_radius(radius).pairs.len(),
                pairs,
                "radius {radius}"
            );
        }
    }

    #[test]
    fn kernel_weights_are_normalised_and_offsets_are_ordered() {
        let kernel = SeparableKernel::for_radius(KERNEL_RADII[MIP_COUNT - 1]);
        let total = kernel.center_weight + 2.0 * kernel.pairs.iter().map(|(_, w)| w).sum::<f32>();
        // A truncated gaussian is a little under 1; three does not renormalise.
        assert!(
            (0.90..=1.02).contains(&total),
            "kernel total weight {total} is not near 1"
        );

        let mut previous = 0.5;
        for (offset, weight) in &kernel.pairs {
            assert!(*weight > 0.0);
            // Merged offsets grow monotonically (1.5.. up to radius - 0.5).
            assert!(*offset > previous, "offset {offset} <= {previous}");
            previous = *offset;
        }
    }

    #[test]
    fn lerp_bloom_factor_matches_the_js() {
        // radius 0 -> factor; radius 1 -> 1.2 - factor.
        for factor in BLOOM_FACTORS {
            let at_zero = factor + ((1.2 - factor) - factor) * 0.0;
            assert_eq!(at_zero, factor);
            let at_one = factor + ((1.2 - factor) - factor) * 1.0;
            assert!((at_one - (1.2 - factor)).abs() < 1e-6);
        }
    }

    #[test]
    fn mip_sizes_halve_with_round() {
        // `Math.round(width / 2)` — remember Rust's f32 round is half away
        // from zero, JS rounds half up; for positive sizes they agree.
        assert_eq!(half(1920), 960);
        assert_eq!(half(960), 480);
        assert_eq!(half(481), 241); // 240.5 -> 241, like Math.round
        assert_eq!(half(1), 1);
        assert_eq!(half(0), 1);
    }

    #[test]
    fn uniform_blocks_have_the_expected_layout() {
        assert_eq!(std::mem::size_of::<BrightUniforms>(), 16);
        assert_eq!(std::mem::size_of::<BlurUniforms>(), 208);
        assert_eq!(std::mem::size_of::<CompositeUniforms>(), 96);
    }

    #[test]
    fn the_composite_scale_keeps_both_bloom_calibrations_available() {
        // The classic UnrealBloomPass shader is `3.0 * strength * sum`; the TSL
        // `bloom()` node (the WebGPU scenes) is `strength * sum`.
        let shader = include_str!("bloom_composite.wgsl");
        assert!(shader.contains("bloom = u.bloom_scale * u.bloom_strength * bloom;"));
        assert!(!shader.contains("3.0 * u.bloom_strength"));
    }

    #[test]
    fn wgsl_parses() {
        for source in [
            include_str!("bloom_bright.wgsl"),
            include_str!("bloom_blur.wgsl"),
            include_str!("bloom_composite.wgsl"),
        ] {
            let full = format!("{}\n{source}", crate::point10::FULLSCREEN_TRIANGLE_WGSL);
            if let Err(error) = naga::front::wgsl::parse_str(&full) {
                panic!("bloom shader failed to parse: {error}\n{source}");
            }
        }
    }
}
