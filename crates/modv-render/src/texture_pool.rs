//! GPU texture pool keyed by `(width, height, format, usage class)`.
//!
//! Implements the "Texture pool" section of `docs/rust-hybrid.md` (WS-2):
//! textures are pooled **only** for short-lived scratch classes; group
//! targets are allocated once per layout change in the M3 wiring (the doc:
//! "pool class" per usage, LRU eviction, ≤16 row-fade scratch).
//!
//! ## Lifetime model (why cloned handles are safe)
//!
//! `wgpu::Texture` is an `Arc`-like handle: cloning one is a refcount bump,
//! not a second GPU allocation. The pool owns one handle per entry;
//! [`TexturePool::acquire`] returns a [`PooledTexture`] holding a *clone* of
//! the same handle, so:
//!
//! - the caller may hold the clone as long as it wants; the GPU resource it
//!   names stays alive until the last clone (and any in-flight command
//!   buffers that reference it) is dropped;
//! - [`TexturePool::evict_lru`] drops the pool's own handle for the oldest
//!   entries. A clone still outstanding stays valid — eviction removes the
//!   entry from the map, it never frees out from under a caller. A later
//!   `acquire` for the same key simply creates a fresh texture;
//! - "return to the pool on drop" is therefore realised as "the clone is
//!   released": [`PooledTexture`] carries no handle back into the map, it
//!   just stops pinning the old allocation. The pool's own handle persists
//!   in the map until eviction, so the texture is reused next frame without
//!   re-creating it.
//!
//! This is the pragmatic reading of pooling in wgpu (there is no linear
//! resource transfer to enforce); the doc's promise — bounded scratch
//! memory, reuse across frames — is kept via [`TexturePool::tick`] +
//! [`TexturePool::evict_lru`], which the executor calls once per frame
//! (see `frame_graph::Executor::execute`).
//!
//! ## Zero-initialisation
//!
//! wgpu does not guarantee texture contents. On creation the pool writes
//! all-zero bytes through the provided queue for the 4-byte-per-pixel 8-bit
//! formats (the scratch classes used so far); other formats are left
//! undefined and callers must clear them before sampling. Reading an
//! undefined texture is never UB in wgpu, but it could produce NaNs the
//! compositor cannot blend away.

use std::collections::HashMap;

/// Key for pooled textures: exact size, format and usage class must match.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TexturePoolKey {
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
    /// Texture format (`Rgba8Unorm` internally; sRGB is a present-time concern).
    pub format: wgpu::TextureFormat,
    /// Why the texture exists — determines usages and frees by class.
    pub usage_class: UsageClass,
}

/// Pooled-texture usage classes (WS-2 "Texture pool" list).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UsageClass {
    /// Group render targets (RGBA8, render + sample, COPY_SRC for read-back).
    /// Long-lived per Group — **not pooled** yet (M3 wiring allocates one per
    /// layout change); the class exists so the key space is complete.
    GroupTarget,
    /// Pipeline scratch: the ping-pong target of the compositor itself
    /// (composite "bounce" texture) and, later, module ping-pong buffers.
    PipelineScratch,
    /// Row fade scratch — one per fading row, ≤16 rows (WS-2 "RowFade scratch").
    RowFadeScratch,
    /// Transition scratch (whole-scene ISF transitions), bounded in size.
    TransitionScratch,
    /// Input textures (video / webcam / screen / image).
    InputFrame,
    /// The 512×1 RGBA8 FFT texture (matches `renderers/shader/index.js:260-278`).
    Fft,
}

impl UsageClass {
    fn usage_flags(self) -> wgpu::TextureUsages {
        use wgpu::TextureUsages as U;
        match self {
            Self::GroupTarget => {
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC | U::COPY_DST
            }
            Self::PipelineScratch => {
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC | U::COPY_DST
            }
            Self::RowFadeScratch => U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_DST,
            Self::TransitionScratch => {
                U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC | U::COPY_DST
            }
            Self::InputFrame => U::TEXTURE_BINDING | U::COPY_DST,
            Self::Fft => U::TEXTURE_BINDING | U::COPY_DST,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::GroupTarget => "group-target",
            Self::PipelineScratch => "pipeline-scratch",
            Self::RowFadeScratch => "row-fade-scratch",
            Self::TransitionScratch => "transition-scratch",
            Self::InputFrame => "input-frame",
            Self::Fft => "fft",
        }
    }

    /// Bytes per texel for the zero-initialisation write; `None` = skip.
    fn bytes_per_pixel(format: wgpu::TextureFormat) -> Option<u32> {
        match format {
            wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Bgra8UnormSrgb => Some(4),
            _ => None,
        }
    }
}

struct PoolEntry {
    texture: wgpu::Texture,
    /// Value of the pool clock at the last [`TexturePool::acquire`]; LRU
    /// eviction removes the smallest values first.
    last_used: u64,
    /// Monotonic creation stamp (diagnostics/tests).
    generation: u64,
}

/// A pooled texture; dropping it releases the caller's reference.
///
/// The GPU texture stays alive as long as any clone (or the pool's own
/// entry) exists; see the module docs for the lifetime model.
pub struct PooledTexture {
    key: TexturePoolKey,
    texture: wgpu::Texture,
    generation: u64,
}

impl PooledTexture {
    /// The pooled texture's key.
    pub fn key(&self) -> &TexturePoolKey {
        &self.key
    }

    /// Borrow the underlying [`wgpu::Texture`].
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Consume the wrapper and return the texture handle.
    pub fn into_texture(self) -> wgpu::Texture {
        self.texture
    }

    /// Creation stamp within the pool (0 = pre-existing fixture).
    #[cfg(test)]
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

impl std::ops::Deref for PooledTexture {
    type Target = wgpu::Texture;

    fn deref(&self) -> &Self::Target {
        &self.texture
    }
}

impl std::fmt::Debug for PooledTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledTexture")
            .field("key", &self.key)
            .field("size", &self.texture.size())
            .field("generation", &self.generation)
            .finish()
    }
}

/// LRU texture pool. See the module docs for the lifetime model.
pub struct TexturePool {
    entries: HashMap<TexturePoolKey, PoolEntry>,
    /// Monotonic clock advanced by [`TexturePool::tick`] (once per frame by the executor).
    clock: u64,
    /// Monotonic creation counter (diagnostics/tests).
    generation: u64,
}

impl TexturePool {
    /// Create an empty pool. `device` is accepted for forward compatibility
    /// (the pool may pre-validate formats in the future); nothing is
    /// allocated at construction time.
    pub fn new(device: &wgpu::Device) -> Self {
        let _ = device;
        Self {
            entries: HashMap::new(),
            clock: 0,
            generation: 0,
        }
    }

    /// Acquire a texture for `key`, creating (and zero-initialising) one if
    /// none is pooled. Cloning the pool's handle is free; the returned
    /// [`PooledTexture`] pins the allocation until dropped.
    pub fn acquire(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: TexturePoolKey,
    ) -> PooledTexture {
        match self.entries.get_mut(&key) {
            Some(entry) => {
                entry.last_used = self.clock;
                PooledTexture {
                    key,
                    texture: entry.texture.clone(),
                    generation: entry.generation,
                }
            }
            None => {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&format!(
                        "modv-pool-{}-{}x{}",
                        key.usage_class.label(),
                        key.width,
                        key.height
                    )),
                    size: wgpu::Extent3d {
                        width: key.width.max(1),
                        height: key.height.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: key.format,
                    usage: key.usage_class.usage_flags(),
                    view_formats: &[],
                });
                if let Some(bpp) = UsageClass::bytes_per_pixel(key.format) {
                    let byte_len = (key.width as u64) * (key.height as u64) * u64::from(bpp);
                    let zeros = vec![0u8; byte_len as usize];
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        &zeros,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(key.width * bpp),
                            rows_per_image: None,
                        },
                        wgpu::Extent3d {
                            width: key.width.max(1),
                            height: key.height.max(1),
                            depth_or_array_layers: 1,
                        },
                    );
                }
                self.generation += 1;
                let generation = self.generation;
                self.entries.insert(
                    key,
                    PoolEntry {
                        texture: texture.clone(),
                        last_used: self.clock,
                        generation,
                    },
                );
                PooledTexture {
                    key,
                    texture,
                    generation,
                }
            }
        }
    }

    /// Advance the LRU clock. Call once per frame (the executor does).
    pub fn tick(&mut self) {
        self.clock = self.clock.wrapping_add(1);
    }

    /// Number of pooled entries (textures, not outstanding clones).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no textures are pooled.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop the least-recently-used entries until at most `keep` remain.
    ///
    /// Evicted textures may still be alive through outstanding
    /// [`PooledTexture`] clones or submitted command buffers; wgpu keeps
    /// the allocation alive until the last reference is released.
    pub fn evict_lru(&mut self, keep: usize) {
        while self.entries.len() > keep {
            // `min_by_key` on the raw entry list; ties are broken arbitrarily
            // (same-frame acquires share a clock tick — both are equally old).
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }
}

impl Default for TexturePool {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            generation: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{Engine, EngineConfig};

    fn try_engine() -> Option<Engine> {
        match Engine::new(&EngineConfig {
            force_fallback_adapter: true,
        }) {
            Ok(engine) => Some(engine),
            Err(error) => {
                eprintln!("skipping texture pool test: no adapter ({error})");
                None
            }
        }
    }

    fn key(index: u32) -> TexturePoolKey {
        TexturePoolKey {
            width: (index + 1) * 8,
            height: 8,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage_class: UsageClass::PipelineScratch,
        }
    }

    #[test]
    fn texture_pool_lru_eviction() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut pool = TexturePool::new(&engine.device);
        assert!(pool.is_empty());

        let keys: Vec<TexturePoolKey> = (0..4).map(key).collect();
        for &k in &keys {
            pool.tick();
            let _ = pool.acquire(&engine.device, &engine.queue, k);
        }
        assert_eq!(pool.len(), 4);

        pool.evict_lru(2);
        assert_eq!(pool.len(), 2);
        // The two most recent keys survive an acquire without growth.
        let _ = pool.acquire(&engine.device, &engine.queue, keys[3]);
        let _ = pool.acquire(&engine.device, &engine.queue, keys[2]);
        assert_eq!(pool.len(), 2);
        // The two oldest were evicted; re-acquiring creates fresh textures.
        let _ = pool.acquire(&engine.device, &engine.queue, keys[1]);
        assert_eq!(pool.len(), 3);
        let _ = pool.acquire(&engine.device, &engine.queue, keys[0]);
        assert_eq!(pool.len(), 4);
    }

    #[test]
    fn pool_reuses_textures_across_frames() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut pool = TexturePool::new(&engine.device);
        let k = key(0);
        let first = pool.acquire(&engine.device, &engine.queue, k);
        let first_generation = first.generation();
        drop(first);
        pool.tick();
        let second = pool.acquire(&engine.device, &engine.queue, k);
        assert_eq!(first_generation, second.generation());
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn evict_lru_keep_zero_empties() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut pool = TexturePool::new(&engine.device);
        for &k in &(0..3).map(key).collect::<Vec<_>>() {
            pool.tick();
            let _ = pool.acquire(&engine.device, &engine.queue, k);
        }
        pool.evict_lru(0);
        assert_eq!(pool.len(), 0);
        assert!(pool.is_empty());
    }
}
