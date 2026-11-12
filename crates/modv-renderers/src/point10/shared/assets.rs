//! Texture assets for the Point10 scenes.
//!
//! Port of the *loading* half of `src/renderer/src/application/point10/shared/textures.js`
//! (the `file://` vs `fetch` split the JS needs because the worker has
//! `nodeIntegrationInWorker`). In Rust the split disappears: assets are read
//! from disk, decoded with the `image` crate and uploaded as RGBA8 textures.
//!
//! # Resolution order
//!
//! [`AssetLoader`] holds an ordered list of directories and resolves a file
//! name (a plain `/`-separated relative path, e.g. `stormy/posz.jpg`) against
//! them:
//!
//! 1. `$MODV_ASSETS_DIR` (the packaged app's asset directory);
//! 2. the `assets_dir` the engine passes in [`SceneContext::assets_dir`]
//!    (the planned `--assets-dir` switch — `modv-app` does not pass one yet);
//! 3. the dev fallback, the repo's own
//!    `src/renderer/src/application/point10/textures` (via
//!    `CARGO_MANIFEST_DIR`, so it only resolves inside a checkout).
//!
//! A missing file never fails a scene: [`load_texture`] builds a procedural
//! checkerboard/gradient stand-in and logs one line, so the scene still
//! renders (the SQL-style "asset absent" case is a packaging problem, not a
//! render crash).
//!
//! # JS-visible conventions
//!
//! * `texture.colorSpace = THREE.SRGBColorSpace` decodes sRGB → linear before
//!   sampling; [`ColourSpace::Srgb`] uploads as `Rgba8UnormSrgb`, which the
//!   hardware decodes identically. Assets the JS leaves at the default
//!   (`waternormals.jpg` — a normal map) use [`ColourSpace::Linear`]
//!   (`Rgba8Unorm`, no decode).
//! * `THREE.Texture` defaults to `LinearMipmapLinearFilter` +
//!   `generateMipmaps = true`, so [`load_texture`] builds a box-filtered mip
//!   chain and the sampler is trilinear. Scenes that need tiling set
//!   `wrapS/wrapT = RepeatWrapping`; that is [`Wrap::Repeat`], otherwise
//!   [`Wrap::Clamp`] (`ClampToEdgeWrapping`).
//! * `texture.anisotropy = maxAniso` (AroundPlus) is **not** reproduced: the
//!   renderer crate does not see the device limits and `max_anisotropy` may
//!   legitimately be 1, so the sampler keeps wgpu's default anisotropy of 1
//!   (slightly softer tiling at grazing angles — the same deviation batch 2
//!   documents for the tunnel checkerboard).
//!
//! [`SceneContext::assets_dir`]: crate::point10::SceneContext::assets_dir

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

/// Environment variable holding the packaged asset directory.
pub const ASSETS_DIR_ENV: &str = "MODV_ASSETS_DIR";

/// The repo's texture directory, relative to this crate. Only resolves inside
/// a checkout (dev and the Docker toolchain); a packaged build uses
/// `$MODV_ASSETS_DIR` or the engine's `--assets-dir`.
pub fn dev_fallback_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src/renderer/src/application/point10/textures")
}

/// A decoded RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8, row 0 = top.
    pub rgba: Vec<u8>,
}

/// How the shader should read the texels (`THREE.Texture.colorSpace`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColourSpace {
    /// `SRGBColorSpace` — upload with hardware sRGB decode.
    Srgb,
    /// The default (no colour-space conversion), e.g. normal maps.
    Linear,
}

impl ColourSpace {
    /// The wgpu upload format for this colour space.
    pub fn format(self) -> wgpu::TextureFormat {
        match self {
            Self::Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
            Self::Linear => wgpu::TextureFormat::Rgba8Unorm,
        }
    }
}

/// Address mode (`THREE.Texture.wrapS/wrapT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrap {
    /// `ClampToEdgeWrapping` (the `TextureLoader` default).
    Clamp,
    /// `RepeatWrapping` (the rescaled UVs of AroundPlus' torus).
    Repeat,
}

/// Ordered texture-directory resolver.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetLoader {
    dirs: Vec<PathBuf>,
}

impl AssetLoader {
    /// `$MODV_ASSETS_DIR`, then the dev fallback.
    ///
    /// The engine's per-context directory is prepended by
    /// [`AssetLoader::for_assets_dir`].
    pub fn new() -> Self {
        Self::for_assets_dir(None)
    }

    /// The loader for a [`SceneContext`](crate::point10::SceneContext):
    /// `$MODV_ASSETS_DIR` first, then the context's directory, then the dev
    /// fallback.
    pub fn for_assets_dir(assets_dir: Option<&Path>) -> Self {
        let mut dirs = Vec::with_capacity(3);
        if let Some(from_env) = std::env::var_os(ASSETS_DIR_ENV) {
            if !from_env.is_empty() {
                dirs.push(PathBuf::from(from_env));
            }
        }
        if let Some(dir) = assets_dir {
            dirs.push(dir.to_path_buf());
        }
        dirs.push(dev_fallback_dir());
        Self { dirs }
    }

    /// An explicit directory list, in resolution order.
    pub fn from_dirs(dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            dirs: dirs.into_iter().collect(),
        }
    }

    /// The directories, in resolution order.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// Resolve a file name against the directories, in order.
    ///
    /// A name is a `/`-separated chain of plain path components: absolute
    /// paths, empty components and `.`/`..` are rejected, so a scene can ask for
    /// a nested asset (`stormy/posz.jpg`, Boxy's Stormy Days faces) without the
    /// loader ever escaping an asset directory.
    pub fn resolve(&self, name: &str) -> Option<PathBuf> {
        if name.is_empty() || name.contains('\\') || name.starts_with('/') {
            return None;
        }

        let components: Vec<&str> = name.split('/').collect();
        if components
            .iter()
            .any(|component| component.is_empty() || *component == "." || *component == "..")
        {
            return None;
        }

        self.dirs
            .iter()
            .map(|dir| {
                components
                    .iter()
                    .fold(dir.clone(), |path, part| path.join(part))
            })
            .find(|path| path.is_file())
    }

    /// Decode `name` into RGBA8, or `None` when it is absent or undecodable.
    pub fn load(&self, name: &str) -> Option<DecodedImage> {
        let path = self.resolve(name)?;
        match decode_file(&path) {
            Ok(image) => Some(image),
            Err(error) => {
                eprintln!(
                    "point10 assets: \"{name}\" at {} could not be decoded ({error}); using the procedural stand-in",
                    path.display()
                );
                None
            }
        }
    }
}

/// Decode a PNG/JPEG (the formats the Point10 textures use) into RGBA8.
pub fn decode_file(path: &Path) -> Result<DecodedImage, image::ImageError> {
    let image = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()?;
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(DecodedImage {
        width,
        height,
        rgba: rgba.into_raw(),
    })
}

/// A texture plus its view, sampler and the metadata scenes need.
pub struct LoadedTexture {
    /// The GPU texture.
    pub texture: wgpu::Texture,
    /// A default view of it.
    pub view: wgpu::TextureView,
    /// Trilinear sampler with the requested address mode.
    pub sampler: wgpu::Sampler,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// True when the procedural stand-in was used.
    pub fallback: bool,
}

/// Load `name` as a texture, or build the procedural stand-in.
///
/// A missing asset logs one line and returns the checkerboard/gradient
/// fallback so the scene keeps its bind group shape.
pub fn load_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    loader: &AssetLoader,
    name: &str,
    colour_space: ColourSpace,
    wrap: Wrap,
) -> LoadedTexture {
    match loader.load(name) {
        Some(image) => upload(device, queue, name, &image, colour_space, wrap, false),
        None => {
            eprintln!(
                "point10 assets: \"{name}\" not found in {:?}; using the procedural fallback",
                loader.dirs()
            );
            procedural_texture(device, queue, colour_space, wrap)
        }
    }
}

/// Upload a decoded image with a generated mip chain.
pub fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    image: &DecodedImage,
    colour_space: ColourSpace,
    wrap: Wrap,
    fallback: bool,
) -> LoadedTexture {
    let width = image.width.max(1);
    let height = image.height.max(1);
    let levels = mip_chain(width, height, &image.rgba);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&format!("point10-asset-{label}")),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: colour_space.format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    for (level, (level_width, level_height, pixels)) in levels.iter().enumerate() {
        // Every mip's rows are padded to WebGPU's 256-byte copy alignment:
        // `bytes_per_row = width * 4` violates it for the small levels
        // (2x2, 1x1, …) and makes the upload a validation error.
        let (data, bytes_per_row) = padded_mip_rows(*level_width, *level_height, pixels);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(*level_height),
            },
            wgpu::Extent3d {
                width: *level_width,
                height: *level_height,
                depth_or_array_layers: 1,
            },
        );
    }

    LoadedTexture {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        sampler: asset_sampler(device, label, wrap),
        texture,
        width,
        height,
        fallback,
    }
}

/// The 256x256 stand-in: a 32 px checkerboard tinted by a diagonal gradient,
/// so a missing asset is obvious but still lights the scene.
pub const FALLBACK_SIZE: u32 = 256;

/// Procedural RGBA8 stand-in for a missing asset.
pub fn build_fallback_image() -> DecodedImage {
    let size = FALLBACK_SIZE;
    let block = size / 8;
    let mut rgba = vec![0_u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let checker = ((x / block) + (y / block)) % 2 == 1;
            let gradient = (x + y) as f32 / (2 * size) as f32;
            let tone = if checker {
                0.55 + 0.35 * gradient
            } else {
                0.18 + 0.25 * gradient
            };
            let index = ((y * size + x) * 4) as usize;
            let byte = (tone * 255.0).round() as u8;
            rgba[index] = byte;
            rgba[index + 1] = byte;
            rgba[index + 2] = byte;
            rgba[index + 3] = 255;
        }
    }

    DecodedImage {
        width: size,
        height: size,
        rgba,
    }
}

/// Build the procedural fallback texture.
pub fn procedural_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    colour_space: ColourSpace,
    wrap: Wrap,
) -> LoadedTexture {
    upload(
        device,
        queue,
        "fallback",
        &build_fallback_image(),
        colour_space,
        wrap,
        true,
    )
}

/// Trilinear sampler with the requested address mode (`Texture`'s defaults,
/// including generated mipmaps).
pub fn asset_sampler(device: &wgpu::Device, label: &str, wrap: Wrap) -> wgpu::Sampler {
    let address_mode = match wrap {
        Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
        Wrap::Repeat => wgpu::AddressMode::Repeat,
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(&format!("point10-asset-{label}-sampler")),
        address_mode_u: address_mode,
        address_mode_v: address_mode,
        address_mode_w: address_mode,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

/// Box-filtered mip chain, level 0 first. Halves each axis (rounding down with
/// a floor of 1) until 1x1, like three's `generateMipmaps`.
pub fn mip_chain(width: u32, height: u32, rgba: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(width.max(1), height.max(1), rgba.to_vec())];

    while let Some((level_width, level_height, pixels)) = levels.last() {
        if *level_width == 1 && *level_height == 1 {
            break;
        }
        levels.push(downsample_2x2(*level_width, *level_height, pixels));
    }

    levels
}

/// Pad one mip level's rows to `COPY_BYTES_PER_ROW_ALIGNMENT`.
///
/// Returns the padded pixel bytes and the row stride to pass as
/// `bytes_per_row` (always a multiple of 256). The texel data itself is
/// unchanged; the trailing bytes of each row are zero.
fn padded_mip_rows(width: u32, height: u32, pixels: &[u8]) -> (Vec<u8>, u32) {
    let unpadded = width as usize * 4;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(alignment) * alignment;
    let mut data = vec![0_u8; padded * height as usize];
    for row in 0..height as usize {
        let source = &pixels[row * unpadded..(row + 1) * unpadded];
        let target = &mut data[row * padded..row * padded + unpadded];
        target.copy_from_slice(source);
    }
    (data, padded as u32)
}

/// One box-filter step.
fn downsample_2x2(width: u32, height: u32, source: &[u8]) -> (u32, u32, Vec<u8>) {
    let width = width.max(1);
    let height = height.max(1);
    let next_width = (width / 2).max(1);
    let next_height = (height / 2).max(1);
    let mut data = vec![0_u8; (next_width * next_height * 4) as usize];

    for y in 0..next_height {
        for x in 0..next_width {
            for channel in 0..4 {
                let mut sum = 0_u32;
                let mut count = 0_u32;
                for offset_y in 0..2 {
                    for offset_x in 0..2 {
                        let sx = (x * 2 + offset_x).min(width - 1);
                        let sy = (y * 2 + offset_y).min(height - 1);
                        let index = ((sy * width + sx) * 4) as usize + channel;
                        if let Some(value) = source.get(index) {
                            sum += *value as u32;
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    data[((y * next_width + x) * 4) as usize + channel] =
                        ((sum + count / 2) / count) as u8;
                }
            }
        }
    }

    (next_width, next_height, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_fallback_points_at_the_repo_textures() {
        let dir = dev_fallback_dir();
        assert!(dir.ends_with("application/point10/textures"), "{dir:?}");
    }

    #[test]
    fn resolves_in_directory_order() {
        let root = std::env::temp_dir().join("modv-point10-assets-test");
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).expect("temp dir");
        std::fs::create_dir_all(&second).expect("temp dir");
        std::fs::write(first.join("shared.png"), b"first").expect("write");
        std::fs::write(second.join("shared.png"), b"second").expect("write");
        std::fs::write(second.join("only.png"), b"only").expect("write");

        let loader = AssetLoader::from_dirs([first.clone(), second.clone()]);
        assert_eq!(loader.resolve("shared.png"), Some(first.join("shared.png")));
        assert_eq!(loader.resolve("only.png"), Some(second.join("only.png")));
        assert_eq!(loader.resolve("missing.png"), None);
        // Nested names resolve inside the directory…
        std::fs::create_dir_all(second.join("stormy")).expect("temp dir");
        std::fs::write(second.join("stormy/posz.jpg"), b"face").expect("write");
        assert_eq!(
            loader.resolve("stormy/posz.jpg"),
            Some(second.join("stormy/posz.jpg"))
        );
        // …but never outside it.
        assert_eq!(loader.resolve("../shared.png"), None);
        assert_eq!(loader.resolve("stormy/../../shared.png"), None);
        assert_eq!(loader.resolve("/etc/passwd"), None);
        assert_eq!(loader.resolve("stormy//posz.jpg"), None);
        assert_eq!(loader.resolve(""), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn loads_the_repo_jpeg() {
        let loader = AssetLoader::new();
        if loader.resolve("pcb.jpg").is_none() {
            // Packaged/CI checkout without the renderer sources: the loader's
            // fallback path is covered below.
            return;
        }
        let image = loader.load("pcb.jpg").expect("pcb.jpg must decode");
        assert!(image.width > 1 && image.height > 1);
        assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
    }

    #[test]
    fn missing_asset_yields_none_so_the_loader_falls_back() {
        let loader = AssetLoader::from_dirs([std::env::temp_dir().join("modv-no-such-dir")]);
        assert!(loader.load("concrete044c_col_1k.jpg").is_none());
    }

    #[test]
    fn context_directory_comes_after_the_environment() {
        let dir = std::env::temp_dir().join("modv-context-assets");
        let loader = AssetLoader::for_assets_dir(Some(&dir));
        assert!(loader.dirs().contains(&dir));
        // The dev fallback is always last.
        assert_eq!(loader.dirs().last(), Some(&dev_fallback_dir()));

        // `$MODV_ASSETS_DIR` wins when it is set; the harness usually leaves it
        // unset, so only assert the ordering when it is present.
        if std::env::var_os(ASSETS_DIR_ENV).is_some() {
            assert_eq!(
                loader.dirs()[0],
                PathBuf::from(std::env::var_os(ASSETS_DIR_ENV).unwrap())
            );
        } else {
            assert_eq!(loader.dirs()[0], dir);
        }
    }

    #[test]
    fn fallback_image_is_a_deterministic_opaque_checker() {
        let image = build_fallback_image();
        assert_eq!(image.width, FALLBACK_SIZE);
        assert_eq!(image.height, FALLBACK_SIZE);
        assert_eq!(build_fallback_image(), image);

        let pixel = |x: u32, y: u32| {
            let index = ((y * image.width + x) * 4) as usize;
            [
                image.rgba[index],
                image.rgba[index + 1],
                image.rgba[index + 2],
                image.rgba[index + 3],
            ]
        };
        // The first checker block is the dark tone, its neighbour the light one.
        assert!(pixel(4, 4)[0] < pixel(40, 4)[0]);
        assert!(pixel(40, 40)[0] < pixel(40, 4)[0] + 200);
        assert!(image.rgba.chunks_exact(4).all(|texel| texel[3] == 255));
    }

    #[test]
    fn mip_chain_halves_each_axis_to_one() {
        let rgba = vec![128_u8; 8 * 4 * 4];
        let levels = mip_chain(8, 4, &rgba);
        let sizes: Vec<(u32, u32)> = levels.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(sizes, vec![(8, 4), (4, 2), (2, 1), (1, 1)]);
        for (width, height, pixels) in &levels {
            assert_eq!(pixels.len(), (width * height * 4) as usize);
            assert!(pixels.iter().all(|value| *value == 128));
        }
    }

    #[test]
    fn colour_space_selects_the_upload_format() {
        assert_eq!(
            ColourSpace::Srgb.format(),
            wgpu::TextureFormat::Rgba8UnormSrgb
        );
        assert_eq!(
            ColourSpace::Linear.format(),
            wgpu::TextureFormat::Rgba8Unorm
        );
    }

    #[test]
    fn every_mip_upload_stride_is_256_aligned_and_keeps_its_content() {
        let image = DecodedImage {
            width: 4,
            height: 4,
            // Distinct values per texel so a mis-copied row is visible.
            rgba: (0..4 * 4 * 4).map(|value| value as u8).collect(),
        };
        let levels = mip_chain(image.width, image.height, &image.rgba);
        assert_eq!(levels.len(), 3, "4x4 -> 2x2 -> 1x1");
        for (width, height, pixels) in &levels {
            let (data, stride) = padded_mip_rows(*width, *height, pixels);
            assert_eq!(
                stride % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
                0,
                "mip {width}x{height} stride {stride} is not 256-aligned"
            );
            assert_eq!(data.len(), stride as usize * *height as usize);
            let unpadded = *width as usize * 4;
            for row in 0..*height as usize {
                assert_eq!(
                    &data[row * stride as usize..row * stride as usize + unpadded],
                    &pixels[row * unpadded..(row + 1) * unpadded],
                    "mip {width}x{height} row {row} content changed"
                );
                assert!(
                    data[row * stride as usize + unpadded..(row + 1) * stride as usize]
                        .iter()
                        .all(|byte| *byte == 0),
                    "padding must be zero"
                );
            }
        }
    }

    #[test]
    fn small_mips_upload_without_validation_errors() {
        // A 4x4 image has 2x2 and 1x1 mips, whose natural `width * 4` stride
        // used to violate WebGPU's `bytes_per_row` alignment.
        let Some((device, queue)) = crate::point10::test_support::gpu() else {
            return;
        };
        let image = DecodedImage {
            width: 4,
            height: 4,
            rgba: vec![200_u8; 4 * 4 * 4],
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let loaded = upload(
            &device,
            &queue,
            "mip-alignment-test",
            &image,
            ColourSpace::Linear,
            Wrap::Clamp,
            false,
        );
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        let error = crate::point10::test_support::block_on(scope.pop());
        assert!(error.is_none(), "mip upload validation error: {error:?}");
        assert_eq!((loaded.width, loaded.height), (4, 4));
        assert_eq!(loaded.texture.mip_level_count(), 3);
    }
}
