//! NDI send/receive (WS-11), ported from
//! `src/renderer/src/application/setup-grandiose.js`,
//! `src/renderer/src/application/worker/ndi-worker.worker.js` and
//! `src/renderer/src/application/worker/store/modules/ndi.js`.
//!
//! The legacy renderer uses the `grandiose` native addon:
//!
//! * the sender reads the composited main canvas (`getImageData`) and submits
//!   one `grandiose` video frame per rendered frame with
//!   `fourCC = FOURCC_RGBX`, `lineStrideBytes = width * 4` and
//!   `frameFormatType = FORMAT_TYPE_PROGRESSIVE`
//!   (`worker/ndi-worker.worker.js:29-61`, `:121-159`);
//! * the receiver pulls `receiver.data()`, treats it as `ImageData` RGBA and
//!   draws it into an auxiliary canvas (`store/modules/ndi.js:48-87`), with
//!   `colorFormat = COLOR_FORMAT_RGBX_RGBA`, `bandwidth = BANDWIDTH_HIGHEST`
//!   and `allowVideoFields = false` (`store/modules/ndi.js:116-123`).
//!
//! This module talks to the NDI SDK directly through a hand-written FFI layer
//! that is **loaded at runtime** (`dlopen` via `libloading`). libndi is not a
//! link-time dependency: the Docker toolchain installs the SDK for
//! `grandiose`, but CI images and user machines routinely lack it. Every
//! public entry point therefore degrades to [`NdiError::Unavailable`] and a
//! single log line instead of failing to start.
//!
//! # Library resolution order
//!
//! [`ndi_library_search_path`] returns, in order:
//!
//! 1. `$MODV_NDI_LIB` (an explicit override — a file or soname);
//! 2. `/usr/lib/libndi.so.5`;
//! 3. `/usr/local/lib/libndi.so.5`;
//! 4. the SDK copy shipped inside `grandiose`, relative to the running
//!    executable:
//!    `app.asar.unpacked/node_modules/grandiose/ndi/lib/lnx-x64/libndi.so.5`
//!    (the engine binary is `resources/modv-engine`, so this is
//!    `resources/app.asar.unpacked/…` in a packaged app), then
//!    `node_modules/grandiose/…`, `grandiose/…` and `ndi/…` for packaged,
//!    development and extracted layouts.
//!
//! Note that `electron-builder.yml:22` excludes `node_modules/grandiose/ndi/**`
//! from the app bundle (the SDK is only needed while *building* grandiose), so
//! on a normal install candidates 2/3 or `$MODV_NDI_LIB` are the ones that
//! matter.
//!
//! # Frame lifetime strategy
//!
//! **Send** uses the blocking `NDIlib_send_send_video_v2`
//! (`Processing.NDI.Send.h:68-70`), not `send_video_async_v2`. The synchronous
//! call does not return until the SDK has consumed the buffer, so
//! [`NdiSender::send_rgba`] can convert RGBA→BGRA into a local `Vec<u8>`, point
//! the frame at it, submit, and drop the `Vec` on return. The async variant
//! would require the buffer to stay alive and untouched until a later
//! synchronising event (`Processing.NDI.Send.h:72-87`), i.e. a pinned
//! per-frame buffer ring with completion bookkeeping; that is a latency
//! optimisation, not a correctness requirement, and is deferred.
//!
//! **Receive** copies the frame out before calling
//! `NDIlib_recv_free_video_v2` (`Processing.NDI.Recv.h:221`); captured buffers
//! belong to the SDK and must never outlive the free call.
//!
//! # Format
//!
//! Sender: caller supplies RGBA8 with an arbitrary row stride (the GPU readback
//! may be padded); we repack it into a tightly packed BGRA8 buffer, which is
//! what `NDIlib_FourCC_video_type_BGRA` describes
//! (`Processing.NDI.structs.h`: "Color ordering in memory is blue, green, red,
//! alpha").
//!
//! Receiver: `color_format = NDIlib_recv_color_format_RGBX_RGBA` (the legacy
//! store's `COLOR_FORMAT_RGBX_RGBA`), so [`NdiFrame::pixels`] is RGBA8
//! (alpha 255 when the source has no alpha) with
//! `stride == width * 4`.
//!
//! # `unsafe`
//!
//! All `unsafe` in `modv-media` lives in this module: the FFI declarations are
//! plain `extern "C"` signatures and `#[repr(C)]` structs, and every call site
//! carries a `SAFETY` comment describing the pointer/lifetime contract from
//! the SDK headers.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

// ---------------------------------------------------------------------------
// NDI SDK constants (Processing.NDI.structs.h / Processing.NDI.Recv.h)
// ---------------------------------------------------------------------------

/// `NDI_LIB_FOURCC` (`Processing.NDI.structs.h:33-36`).
pub const fn fourcc(code: &[u8; 4]) -> u32 {
    (code[0] as u32) | ((code[1] as u32) << 8) | ((code[2] as u32) << 16) | ((code[3] as u32) << 24)
}

/// `NDIlib_FourCC_video_type_BGRA` (`Processing.NDI.structs.h:102`).
pub const FOURCC_BGRA: u32 = fourcc(b"BGRA");

/// `NDIlib_FourCC_video_type_RGBA` (`Processing.NDI.structs.h:112`).
pub const FOURCC_RGBA: u32 = fourcc(b"RGBA");

/// `NDIlib_frame_format_type_progressive` (`Processing.NDI.structs.h:140`).
pub const FRAME_FORMAT_PROGRESSIVE: c_int = 1;

/// `NDIlib_send_timecode_synthesize` (`Processing.NDI.structs.h:176`): let the
/// SDK derive the timecode, as the JS sender does by omitting it.
pub const SEND_TIMECODE_SYNTHESIZE: i64 = i64::MAX;

/// `NDIlib_frame_type_none` (`Processing.NDI.structs.h:38`).
pub const FRAME_TYPE_NONE: c_int = 0;

/// `NDIlib_frame_type_video` (`Processing.NDI.structs.h:39`).
pub const FRAME_TYPE_VIDEO: c_int = 1;

/// `NDIlib_frame_type_error` (`Processing.NDI.structs.h:42`).
pub const FRAME_TYPE_ERROR: c_int = 4;

/// `NDIlib_recv_color_format_RGBX_RGBA` (`Processing.NDI.Recv.h:56`) — the
/// legacy store's `COLOR_FORMAT_RGBX_RGBA` (`store/modules/ndi.js:119`).
pub const COLOR_FORMAT_RGBX_RGBA: c_int = 2;

/// `NDIlib_recv_bandwidth_highest` (`Processing.NDI.Recv.h:39`).
pub const BANDWIDTH_HIGHEST: c_int = 100;

/// The soname installed by the NDI SDK and by the Docker toolchain
/// (`/usr/local/lib/libndi.so.5`).
pub const NDI_SONAME: &str = "libndi.so.5";

/// The NDI SDK copy shipped by `grandiose` (`node_modules/grandiose/ndi/lib/lnx-x64`).
pub const GRANDIOSE_NDI_SUFFIX: &str = "node_modules/grandiose/ndi/lib/lnx-x64/libndi.so.5";

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why an NDI operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NdiError {
    /// libndi could not be loaded (or initialised). Every NDI entry point
    /// returns this when the SDK is not installed — never a panic.
    Unavailable,
    /// The SDK was loaded but refused to create an instance.
    CreateFailed(&'static str),
    /// A name contained an interior NUL byte.
    InvalidName,
    /// The frame does not match the contract of the call (empty, stride too
    /// small, buffer shorter than `stride * height`, …).
    InvalidFrame(String),
    /// `NDIlib_recv_capture_v2` reported a lost connection.
    ConnectionLost,
    /// The instance was already `destroy`ed.
    Destroyed,
}

impl fmt::Display for NdiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => write!(f, "the NDI SDK (libndi) is not available"),
            Self::CreateFailed(what) => write!(f, "the NDI SDK could not create a {what} instance"),
            Self::InvalidName => write!(f, "the NDI name contains an interior NUL byte"),
            Self::InvalidFrame(reason) => write!(f, "invalid NDI frame: {reason}"),
            Self::ConnectionLost => write!(f, "the NDI connection was lost"),
            Self::Destroyed => write!(f, "the NDI instance was already destroyed"),
        }
    }
}

impl std::error::Error for NdiError {}

// ---------------------------------------------------------------------------
// SDK structs (layout verified against the Shipping headers)
// ---------------------------------------------------------------------------

/// `NDIlib_source_t` (`Processing.NDI.structs.h:182-201`).
///
/// `p_ndi_name` and the `p_url_address`/`p_ip_address` union are both pointers,
/// so the struct is two pointers wide (16 bytes on x86_64).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
// Fields mirror the SDK layout and are only read by the C side.
#[allow(dead_code)]
struct NdiSource {
    p_ndi_name: *const c_char,
    p_url_address: *const c_char,
}

impl NdiSource {
    const fn empty() -> Self {
        Self {
            p_ndi_name: ptr::null(),
            p_url_address: ptr::null(),
        }
    }
}

/// `NDIlib_send_create_t` (`Processing.NDI.Send.h:36-57`).
///
/// `clock_video` / `clock_audio` are C `bool`s; modelled as `u8` because only
/// the 1-byte layout matters and `0`/`1` are always valid.
#[repr(C)]
// Fields mirror the SDK layout and are only read by the C side.
#[allow(dead_code)]
struct NdiSendCreate {
    p_ndi_name: *const c_char,
    p_groups: *const c_char,
    clock_video: u8,
    clock_audio: u8,
}

/// `NDIlib_video_frame_v2_t` (`Processing.NDI.structs.h:208-263`).
///
/// Field order and offsets (x86_64): `xres` 0, `yres` 4, `FourCC` 8,
/// `frame_rate_N` 12, `frame_rate_D` 16, `picture_aspect_ratio` 20,
/// `frame_format_type` 24, `timecode` 32, `p_data` 40,
/// `line_stride_in_bytes` 48, `p_metadata` 56, `timestamp` 64; size 72.
#[repr(C)]
#[derive(Debug)]
// Fields mirror the SDK layout: `copy_video_frame` reads the geometry, the rest
// is only consumed by the C side.
#[allow(dead_code)]
struct NdiVideoFrameV2 {
    xres: c_int,
    yres: c_int,
    four_cc: u32,
    frame_rate_n: c_int,
    frame_rate_d: c_int,
    picture_aspect_ratio: f32,
    frame_format_type: c_int,
    timecode: i64,
    p_data: *mut u8,
    line_stride_in_bytes: c_int,
    p_metadata: *const c_char,
    timestamp: i64,
}

impl NdiVideoFrameV2 {
    const fn empty() -> Self {
        Self {
            xres: 0,
            yres: 0,
            four_cc: 0,
            frame_rate_n: 0,
            frame_rate_d: 0,
            picture_aspect_ratio: 0.0,
            frame_format_type: 0,
            timecode: 0,
            p_data: ptr::null_mut(),
            line_stride_in_bytes: 0,
            p_metadata: ptr::null(),
            timestamp: 0,
        }
    }
}

/// `NDIlib_recv_create_v3_t` (`Processing.NDI.Recv.h:104-138`).
///
/// Field order: embedded [`NdiSource`] 0, `color_format` 16, `bandwidth` 20,
/// `allow_video_fields` 24, `p_ndi_recv_name` 32; size 40.
#[repr(C)]
// Fields mirror the SDK layout and are only read by the C side.
#[allow(dead_code)]
struct NdiRecvCreateV3 {
    source_to_connect_to: NdiSource,
    color_format: c_int,
    bandwidth: c_int,
    allow_video_fields: u8,
    p_ndi_recv_name: *const c_char,
}

/// `NDIlib_find_create_t` (`Processing.NDI.Find.h:36-59`).
#[repr(C)]
// Fields mirror the SDK layout and are only read by the C side.
#[allow(dead_code)]
struct NdiFindCreate {
    show_local_sources: u8,
    p_groups: *const c_char,
    p_extra_ips: *const c_char,
}

// Compile-time layout guards: a mismatch here means every FFI call would read
// the wrong offsets, so fail the build instead.
const _: () = assert!(core::mem::size_of::<NdiSource>() == 16);
const _: () = assert!(core::mem::size_of::<NdiSendCreate>() == 24);
const _: () = assert!(core::mem::size_of::<NdiVideoFrameV2>() == 72);
const _: () = assert!(core::mem::size_of::<NdiRecvCreateV3>() == 40);
const _: () = assert!(core::mem::size_of::<NdiFindCreate>() == 24);

// ---------------------------------------------------------------------------
// FFI function signatures (Processing.NDI.Send.h, .Recv.h, .Find.h, .Lib.h)
// ---------------------------------------------------------------------------

type InitializeFn = unsafe extern "C" fn() -> u8;
type IsSupportedCpuFn = unsafe extern "C" fn() -> u8;

type SendCreateFn = unsafe extern "C" fn(*const NdiSendCreate) -> *mut c_void;
type SendDestroyFn = unsafe extern "C" fn(*mut c_void);
type SendSendVideoV2Fn = unsafe extern "C" fn(*mut c_void, *const NdiVideoFrameV2);

type RecvCreateV3Fn = unsafe extern "C" fn(*const NdiRecvCreateV3) -> *mut c_void;
type RecvDestroyFn = unsafe extern "C" fn(*mut c_void);
type RecvConnectFn = unsafe extern "C" fn(*mut c_void, *const NdiSource);
type RecvCaptureV2Fn =
    unsafe extern "C" fn(*mut c_void, *mut NdiVideoFrameV2, *mut c_void, *mut c_void, u32) -> c_int;
type RecvFreeVideoV2Fn = unsafe extern "C" fn(*mut c_void, *const NdiVideoFrameV2);

type FindCreateV2Fn = unsafe extern "C" fn(*const NdiFindCreate) -> *mut c_void;
type FindDestroyFn = unsafe extern "C" fn(*mut c_void);
type FindGetCurrentSourcesFn = unsafe extern "C" fn(*mut c_void, *mut u32) -> *const NdiSource;
type FindWaitForSourcesFn = unsafe extern "C" fn(*mut c_void, u32) -> u8;

/// The resolved entry points of one loaded `libndi`.
struct NdiApi {
    initialize: InitializeFn,
    is_supported_cpu: Option<IsSupportedCpuFn>,
    send_create: SendCreateFn,
    send_destroy: SendDestroyFn,
    send_send_video_v2: SendSendVideoV2Fn,
    recv_create_v3: RecvCreateV3Fn,
    recv_destroy: RecvDestroyFn,
    recv_connect: RecvConnectFn,
    recv_capture_v2: RecvCaptureV2Fn,
    recv_free_video_v2: RecvFreeVideoV2Fn,
    find_create_v2: FindCreateV2Fn,
    find_destroy: FindDestroyFn,
    find_get_current_sources: FindGetCurrentSourcesFn,
    find_wait_for_sources: FindWaitForSourcesFn,
}

/// Resolves one symbol, copying the function pointer out of the loader handle.
///
/// `name` must be a NUL-terminated byte literal.
fn resolve<T: Copy>(library: &libloading::Library, name: &'static [u8]) -> Result<T, String> {
    // SAFETY: `name` is a NUL-terminated literal whose symbol has type `T` — the
    // signature taken verbatim from the SDK header cited next to each field in
    // `NdiApi::resolve`. The returned pointer is copied out, and the library is
    // owned by `NdiLibrary`, which is never unloaded while the pointer is used
    // (the global instance lives for the whole process).
    unsafe {
        library
            .get::<T>(name)
            .map(|symbol| *symbol)
            .map_err(|error| {
                format!(
                    "{}: {error}",
                    String::from_utf8_lossy(&name[..name.len() - 1])
                )
            })
    }
}

impl NdiApi {
    fn resolve(library: &libloading::Library) -> Result<Self, String> {
        Ok(Self {
            // Processing.NDI.Lib.h:117
            initialize: resolve(library, b"NDIlib_initialize\0")?,
            // Processing.NDI.Lib.h:127 — optional: it is only an extra guard.
            is_supported_cpu: resolve(library, b"NDIlib_is_supported_CPU\0").ok(),
            // Processing.NDI.Send.h:62
            send_create: resolve(library, b"NDIlib_send_create\0")?,
            // Processing.NDI.Send.h:66
            send_destroy: resolve(library, b"NDIlib_send_destroy\0")?,
            // Processing.NDI.Send.h:70
            send_send_video_v2: resolve(library, b"NDIlib_send_send_video_v2\0")?,
            // Processing.NDI.Recv.h:178
            recv_create_v3: resolve(library, b"NDIlib_recv_create_v3\0")?,
            // Processing.NDI.Recv.h:182 (destroy)
            recv_destroy: resolve(library, b"NDIlib_recv_destroy\0")?,
            // Processing.NDI.Recv.h:187
            recv_connect: resolve(library, b"NDIlib_recv_connect\0")?,
            // Processing.NDI.Recv.h:196
            recv_capture_v2: resolve(library, b"NDIlib_recv_capture_v2\0")?,
            // Processing.NDI.Recv.h:221
            recv_free_video_v2: resolve(library, b"NDIlib_recv_free_video_v2\0")?,
            // Processing.NDI.Find.h:75
            find_create_v2: resolve(library, b"NDIlib_find_create_v2\0")?,
            // Processing.NDI.Find.h:68
            find_destroy: resolve(library, b"NDIlib_find_destroy\0")?,
            // Processing.NDI.Find.h:75
            find_get_current_sources: resolve(library, b"NDIlib_find_get_current_sources\0")?,
            // Processing.NDI.Find.h:79
            find_wait_for_sources: resolve(library, b"NDIlib_find_wait_for_sources\0")?,
        })
    }
}

/// A loaded NDI SDK. Holds the `dlopen` handle and the resolved entry points.
pub struct NdiLibrary {
    /// The path (or soname) the library was loaded from.
    path: PathBuf,
    /// Kept alive so the resolved function pointers stay valid.
    _library: libloading::Library,
    api: NdiApi,
}

impl fmt::Debug for NdiLibrary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NdiLibrary")
            .field("path", &self.path)
            .finish()
    }
}

impl NdiLibrary {
    /// Loads and initialises the SDK, trying [`ndi_library_search_path`] in
    /// order. On failure the reason is logged once (see [`ndi_available`]).
    pub fn load() -> Result<Arc<Self>, NdiError> {
        let candidates = ndi_library_search_path();

        match Self::try_load(&candidates) {
            Ok(library) => {
                eprintln!(
                    "engine: NDI available (libndi loaded from {})",
                    library.path.display()
                );
                Ok(library)
            }
            Err(reason) => {
                warn_unavailable_once(&reason);
                Err(NdiError::Unavailable)
            }
        }
    }

    fn try_load(candidates: &[PathBuf]) -> Result<Arc<Self>, String> {
        let mut errors = Vec::new();

        for candidate in candidates {
            // SAFETY: `dlopen` of the candidate path. A failure is reported by
            // the loader as an `Err`; nothing is dereferenced here.
            let library = match unsafe { libloading::Library::new(candidate) } {
                Ok(library) => library,
                Err(error) => {
                    errors.push(format!("{}: {error}", candidate.display()));
                    continue;
                }
            };

            let api = match NdiApi::resolve(&library) {
                Ok(api) => api,
                Err(error) => {
                    errors.push(format!("{}: {error}", candidate.display()));
                    continue;
                }
            };

            // SAFETY: `NDIlib_initialize` has no arguments, returns a C `bool`
            // (read as `u8`, non-zero = success; Processing.NDI.Lib.h:117) and
            // is safe to call before any other SDK function. It is called once
            // per successful load, and the library stays alive in `Self`.
            if unsafe { (api.initialize)() } == 0 {
                errors.push(format!(
                    "{}: NDIlib_initialize() returned false",
                    candidate.display()
                ));
                continue;
            }

            if let Some(is_supported_cpu) = api.is_supported_cpu {
                // SAFETY: no arguments, C `bool` return; called after
                // initialize succeeded, as the header requires
                // (Processing.NDI.Lib.h:110-127).
                if unsafe { is_supported_cpu() } == 0 {
                    errors.push(format!(
                        "{}: the CPU is not supported by the NDI SDK",
                        candidate.display()
                    ));
                    continue;
                }
            }

            return Ok(Arc::new(Self {
                path: candidate.clone(),
                _library: library,
                api,
            }));
        }

        Err(if errors.is_empty() {
            "no libndi candidates".to_string()
        } else {
            errors.join("; ")
        })
    }
}

/// Logs the "NDI is unavailable" line at most once per process.
fn warn_unavailable_once(reason: &str) {
    static WARNED: AtomicBool = AtomicBool::new(false);

    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "engine: NDI unavailable — {reason}. NDI send/receive will report Unavailable. \
             Set MODV_NDI_LIB or install the NDI SDK to enable it."
        );
    }
}

/// The library search path, in resolution order (see the module docs).
///
/// The executable's directory is used for the `grandiose`-shipped candidates.
pub fn ndi_library_search_path() -> Vec<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));

    let env_override = std::env::var("MODV_NDI_LIB").ok();
    ndi_library_candidates_with_env(env_override.as_deref(), &exe_dir)
}

/// Pure candidate-list builder behind [`ndi_library_search_path`].
///
/// `env_override` is `$MODV_NDI_LIB`; `exe_dir` is the directory containing the
/// running engine binary.
pub fn ndi_library_candidates_with_env(env_override: Option<&str>, exe_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(9);

    if let Some(path) = env_override.filter(|path| !path.is_empty()) {
        candidates.push(PathBuf::from(path));
    }

    // The two install locations the Docker toolchain / NDI SDK use.
    candidates.push(PathBuf::from("/usr/lib").join(NDI_SONAME));
    candidates.push(PathBuf::from("/usr/local/lib").join(NDI_SONAME));

    // The `grandiose`-shipped copy, relative to the running executable:
    // packaged (`resources/modv-engine` → `resources/app.asar.unpacked/…`),
    // plain-asar, development and extracted layouts.
    for prefix in [
        "app.asar.unpacked",
        "resources/app.asar.unpacked",
        ".",
        "../../..",
    ] {
        candidates.push(exe_dir.join(prefix).join(GRANDIOSE_NDI_SUFFIX));
    }
    // The `grandiose` package layout without the `node_modules/` prefix.
    candidates.push(exe_dir.join("grandiose/ndi/lib/lnx-x64/libndi.so.5"));

    candidates
}

static NDI_LIBRARY: OnceLock<Option<Arc<NdiLibrary>>> = OnceLock::new();

/// The process-wide SDK handle, loading it on first use.
///
/// `None` when libndi is missing; the failure is logged once by
/// [`NdiLibrary::load`].
pub fn ndi_library() -> Option<&'static Arc<NdiLibrary>> {
    NDI_LIBRARY.get_or_init(|| NdiLibrary::load().ok()).as_ref()
}

/// Whether libndi could be loaded and initialised.
///
/// A missing SDK is not an error condition: callers check this and report NDI
/// as disabled.
pub fn ndi_available() -> bool {
    ndi_library().is_some()
}

/// The path libndi was loaded from, if it is available (diagnostics/logging).
pub fn ndi_library_path() -> Option<PathBuf> {
    ndi_library().map(|library| library.path.clone())
}

// ---------------------------------------------------------------------------
// Frame helpers (pure; unit tested)
// ---------------------------------------------------------------------------

/// Converts a strided RGBA8 buffer into a tightly packed BGRA8 buffer.
///
/// `stride` is the source row stride in bytes and must be at least
/// `width * 4`; `pixels` must hold `stride * height` bytes. Only the visible
/// `width * 4` bytes of each row are copied, so GPU readback padding is
/// dropped.
///
/// The resulting layout is exactly `NDIlib_FourCC_video_type_BGRA`
/// (`Processing.NDI.structs.h:93-103`).
pub fn rgba_to_bgra_frame(
    width: u32,
    height: u32,
    stride: u32,
    pixels: &[u8],
) -> Result<Vec<u8>, NdiError> {
    if width == 0 || height == 0 {
        return Err(NdiError::InvalidFrame("the frame is empty".to_string()));
    }

    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| NdiError::InvalidFrame("width overflows usize".to_string()))?;
    let stride = stride as usize;
    if stride < row_bytes {
        return Err(NdiError::InvalidFrame(format!(
            "stride {stride} is smaller than an RGBA row ({row_bytes})"
        )));
    }

    let required = stride
        .checked_mul(height as usize)
        .ok_or_else(|| NdiError::InvalidFrame("frame size overflows usize".to_string()))?;
    if pixels.len() < required {
        return Err(NdiError::InvalidFrame(format!(
            "buffer holds {} bytes, expected {required}",
            pixels.len()
        )));
    }

    let mut bgra = Vec::with_capacity(row_bytes * height as usize);
    for row in pixels.chunks_exact(stride).take(height as usize) {
        for pixel in row[..row_bytes].chunks_exact(4) {
            bgra.push(pixel[2]); // B
            bgra.push(pixel[1]); // G
            bgra.push(pixel[0]); // R
            bgra.push(pixel[3]); // A
        }
    }

    Ok(bgra)
}

/// One received NDI video frame, copied out of the SDK buffer.
///
/// `pixels` is RGBA8 (`COLOR_FORMAT_RGBX_RGBA`) and `stride == width * 4`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NdiFrame {
    /// Frame width in pixels (`xres`).
    pub width: u32,
    /// Frame height in pixels (`yres`).
    pub height: u32,
    /// Row stride in bytes; always `width * 4` for a captured frame.
    pub stride: u32,
    /// RGBA8 pixels, `stride * height` bytes.
    pub pixels: Vec<u8>,
}

/// Copies a captured SDK frame into a packed [`NdiFrame`].
fn copy_video_frame(frame: &NdiVideoFrameV2) -> Result<NdiFrame, NdiError> {
    if frame.xres <= 0 || frame.yres <= 0 {
        return Err(NdiError::InvalidFrame(format!(
            "the captured frame has no pixels ({}×{})",
            frame.xres, frame.yres
        )));
    }
    if frame.p_data.is_null() {
        return Err(NdiError::InvalidFrame(
            "the captured frame has no data pointer".to_string(),
        ));
    }

    let width = frame.xres as usize;
    let height = frame.yres as usize;
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| NdiError::InvalidFrame("width overflows usize".to_string()))?;
    let source_stride = if frame.line_stride_in_bytes > 0 {
        frame.line_stride_in_bytes as usize
    } else {
        // `line_stride_in_bytes == 0` means "sizeof(one pixel) * xres"
        // (Processing.NDI.structs.h:233-237).
        row_bytes
    };
    if source_stride < row_bytes {
        return Err(NdiError::InvalidFrame(format!(
            "the captured stride {source_stride} is smaller than a row ({row_bytes})"
        )));
    }

    let source_len = source_stride
        .checked_mul(height)
        .ok_or_else(|| NdiError::InvalidFrame("frame size overflows usize".to_string()))?;

    // SAFETY: the NDI SDK guarantees `p_data` is readable for
    // `line_stride_in_bytes * yres` bytes for the lifetime of a captured frame
    // (Processing.NDI.Recv.h:196-221); the frame is freed by the caller only
    // after this copy returns.
    let source = unsafe { slice::from_raw_parts(frame.p_data, source_len) };

    let mut pixels = Vec::with_capacity(row_bytes * height);
    for row in source.chunks_exact(source_stride).take(height) {
        pixels.extend_from_slice(&row[..row_bytes]);
    }

    Ok(NdiFrame {
        width: frame.xres as u32,
        height: frame.yres as u32,
        stride: row_bytes as u32,
        pixels,
    })
}

// ---------------------------------------------------------------------------
// Sender
// ---------------------------------------------------------------------------

/// An NDI video sender (`getGrandioseSender`, `setup-grandiose.js:40-53`).
///
/// The JS sender is named `VmodV` with `clockVideo: false, clockAudio: false`;
/// [`NdiSender::create`] takes those as arguments.
pub struct NdiSender {
    library: Arc<NdiLibrary>,
    instance: *mut c_void,
    frame_rate_n: c_int,
    frame_rate_d: c_int,
}

impl fmt::Debug for NdiSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NdiSender")
            .field("library", &self.library.path)
            .field("instance", &self.instance)
            .field("frame_rate", &(self.frame_rate_n, self.frame_rate_d))
            .finish()
    }
}

impl NdiSender {
    /// Creates a sender named `name` (the legacy name is `"VmodV"`).
    ///
    /// `clock_video` maps to `NDIlib_send_create_t::clock_video`: when true the
    /// SDK rate-limits submissions to the observed frame rate
    /// (`Processing.NDI.Send.h:43-48`). `clock_audio` stays false — the engine
    /// sends no audio, matching `setup-grandiose.js:48-50`.
    pub fn create(name: &str, clock_video: bool) -> Result<Self, NdiError> {
        let library = ndi_library().ok_or(NdiError::Unavailable)?;
        let name = CString::new(name).map_err(|_| NdiError::InvalidName)?;

        let settings = NdiSendCreate {
            p_ndi_name: name.as_ptr(),
            p_groups: ptr::null(),
            clock_video: u8::from(clock_video),
            clock_audio: 0,
        };

        // SAFETY: `settings` is a valid `NDIlib_send_create_t`
        // (Processing.NDI.Send.h:36-62) whose string pointers stay valid for
        // the call; the SDK copies the name into the instance. A null return
        // means creation failed and is reported as an error.
        let instance = unsafe { (library.api.send_create)(&settings) };
        if instance.is_null() {
            return Err(NdiError::CreateFailed("sender"));
        }

        Ok(Self {
            library: Arc::clone(library),
            instance,
            // `frameRateN: actualFps * 1000, frameRateD: 1000`
            // (ndi-worker.worker.js:148-149); 60 fps until the caller measures.
            frame_rate_n: 60_000,
            frame_rate_d: 1_000,
        })
    }

    /// Overrides the advertised frame rate (`frameRateN / frameRateD`,
    /// ndi-worker.worker.js:148-149).
    pub fn set_frame_rate(&mut self, numerator: c_int, denominator: c_int) {
        if numerator > 0 && denominator > 0 {
            self.frame_rate_n = numerator;
            self.frame_rate_d = denominator;
        }
    }

    /// Submits one RGBA8 frame (strided) as a BGRA NDI video frame.
    ///
    /// The frame is converted into an owned buffer and submitted with the
    /// blocking `NDIlib_send_send_video_v2`, which returns once the SDK has
    /// consumed it; the buffer is dropped immediately after. See the module
    /// docs for why the async variant is not used.
    pub fn send_rgba(
        &self,
        width: u32,
        height: u32,
        stride: u32,
        pixels: &[u8],
    ) -> Result<(), NdiError> {
        if self.instance.is_null() {
            return Err(NdiError::Destroyed);
        }

        // `xres`, `yres` and `line_stride_in_bytes` are C `int`s.
        if width > (c_int::MAX as u32) / 4 || height > c_int::MAX as u32 {
            return Err(NdiError::InvalidFrame(format!(
                "frame {width}×{height} does not fit the NDI int geometry fields"
            )));
        }

        let mut bgra = rgba_to_bgra_frame(width, height, stride, pixels)?;

        let frame = NdiVideoFrameV2 {
            xres: width as c_int,
            yres: height as c_int,
            four_cc: FOURCC_BGRA,
            frame_rate_n: self.frame_rate_n,
            frame_rate_d: self.frame_rate_d,
            picture_aspect_ratio: width as f32 / height as f32,
            frame_format_type: FRAME_FORMAT_PROGRESSIVE,
            timecode: SEND_TIMECODE_SYNTHESIZE,
            p_data: bgra.as_mut_ptr(),
            line_stride_in_bytes: (width as usize * 4) as c_int,
            p_metadata: ptr::null(),
            timestamp: 0,
        };

        // SAFETY: `self.instance` is a live `NDIlib_send_instance_t` (checked
        // above and only freed in `destroy`); `frame` is a valid
        // `NDIlib_video_frame_v2_t` whose `p_data` points at `bgra`, which
        // stays allocated and unborrowed until this blocking call returns
        // (Processing.NDI.Send.h:68-70; the sync call is listed as a
        // synchronising event for async frames, i.e. it completes on return).
        unsafe { (self.library.api.send_send_video_v2)(self.instance, &frame) };

        Ok(())
    }

    /// Destroys the sender instance. Idempotent; `Drop` calls it too.
    pub fn destroy(&mut self) {
        if self.instance.is_null() {
            return;
        }

        // SAFETY: `self.instance` was created by `NDIlib_send_create` and is
        // freed exactly once (the field is nulled immediately after), as
        // required by Processing.NDI.Send.h:64-66.
        unsafe { (self.library.api.send_destroy)(self.instance) };
        self.instance = ptr::null_mut();
    }
}

impl Drop for NdiSender {
    fn drop(&mut self) {
        self.destroy();
    }
}

// ---------------------------------------------------------------------------
// Receiver
// ---------------------------------------------------------------------------

/// An NDI receiver (`store/modules/ndi.js:116-142`).
///
/// `create` builds a receiver that is not connected yet; [`Self::connect`]
/// resolves the name filter through `NDIlib_find_*` and connects, mirroring the
/// legacy flow where a source is picked from `discoverSources` and then passed
/// to `grandiose.receive`.
pub struct NdiReceiver {
    library: Arc<NdiLibrary>,
    instance: *mut c_void,
    name_filter: Option<String>,
    connected_source: Option<String>,
}

impl fmt::Debug for NdiReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NdiReceiver")
            .field("library", &self.library.path)
            .field("instance", &self.instance)
            .field("name_filter", &self.name_filter)
            .field("connected_source", &self.connected_source)
            .finish()
    }
}

impl NdiReceiver {
    /// Creates a receiver for an optional source-name filter.
    ///
    /// `color_format`, `bandwidth` and `allow_video_fields` match
    /// `store/modules/ndi.js:119-121`: RGBA output, highest bandwidth, no
    /// fielded video.
    pub fn create(name_filter: Option<&str>) -> Result<Self, NdiError> {
        let library = ndi_library().ok_or(NdiError::Unavailable)?;

        let settings = NdiRecvCreateV3 {
            // A null source means "no connection yet"
            // (Processing.NDI.Recv.h:106-107); `connect` selects one.
            source_to_connect_to: NdiSource::empty(),
            color_format: COLOR_FORMAT_RGBX_RGBA,
            bandwidth: BANDWIDTH_HIGHEST,
            allow_video_fields: 0,
            // Null lets the SDK derive a receiver name
            // (Processing.NDI.Recv.h:123-127).
            p_ndi_recv_name: ptr::null(),
        };

        // SAFETY: `settings` is a valid `NDIlib_recv_create_v3_t`
        // (Processing.NDI.Recv.h:104-178); the SDK copies it. A null return
        // means creation failed.
        let instance = unsafe { (library.api.recv_create_v3)(&settings) };
        if instance.is_null() {
            return Err(NdiError::CreateFailed("receiver"));
        }

        Ok(Self {
            library: Arc::clone(library),
            instance,
            name_filter: name_filter.map(str::to_string),
            connected_source: None,
        })
    }

    /// The source name this receiver is connected to, if any.
    pub fn connected_source(&self) -> Option<&str> {
        self.connected_source.as_deref()
    }

    /// Waits up to `timeout_ms` for a matching source and connects to it.
    ///
    /// A `None` filter selects the first advertised source; a `Some` filter
    /// matches the first source whose NDI name contains it (case-insensitive).
    /// Returns the connected source's name, or `Ok(None)` when nothing matched
    /// within the timeout.
    pub fn connect(&mut self, timeout_ms: u32) -> Result<Option<String>, NdiError> {
        if self.instance.is_null() {
            return Err(NdiError::Destroyed);
        }

        let settings = NdiFindCreate {
            show_local_sources: 1,
            p_groups: ptr::null(),
            p_extra_ips: ptr::null(),
        };

        // SAFETY: `settings` is a valid `NDIlib_find_create_t`
        // (Processing.NDI.Find.h:36-64); a null return means no finder.
        let finder = unsafe { (self.library.api.find_create_v2)(&settings) };
        if finder.is_null() {
            return Err(NdiError::CreateFailed("finder"));
        }

        // SAFETY: `finder` is live. `find_wait_for_sources` blocks for at most
        // `timeout_ms` and only reports whether the source list changed; we
        // enumerate afterwards regardless (Processing.NDI.Find.h:76-79).
        unsafe { (self.library.api.find_wait_for_sources)(finder, timeout_ms) };

        let mut count: u32 = 0;
        // SAFETY: `finder` is live; the returned array of `NDIlib_source_t`
        // stays valid until the next `find_get_current_sources` or
        // `find_destroy` (Processing.NDI.Find.h:70-75), both of which happen
        // after the connection is made.
        let sources = unsafe { (self.library.api.find_get_current_sources)(finder, &mut count) };

        let filter = self.name_filter.as_ref().map(|name| name.to_lowercase());
        let mut chosen: Option<(usize, String)> = None;

        if !sources.is_null() {
            for index in 0..count as usize {
                // SAFETY: `sources` points at `count` consecutive
                // `NDIlib_source_t` values; `index < count`.
                let source = unsafe { &*sources.add(index) };
                if source.p_ndi_name.is_null() {
                    continue;
                }
                // SAFETY: `p_ndi_name` is a NUL-terminated UTF-8 string owned by
                // the finder, valid until `find_destroy`.
                let name = unsafe { CStr::from_ptr(source.p_ndi_name) }
                    .to_string_lossy()
                    .into_owned();

                let matches = match &filter {
                    Some(filter) => name.to_lowercase().contains(filter.as_str()),
                    None => true,
                };
                if matches {
                    chosen = Some((index, name));
                    break;
                }
            }
        }

        let Some((index, name)) = chosen else {
            // SAFETY: `finder` was created above and has not been destroyed.
            unsafe { (self.library.api.find_destroy)(finder) };
            return Ok(None);
        };

        // SAFETY: `sources.add(index)` is one of the finder's live source
        // records and `self.instance` is a live receiver; `recv_connect`
        // copies the source description (Processing.NDI.Recv.h:184-187). The
        // finder is destroyed only after the call.
        unsafe {
            (self.library.api.recv_connect)(self.instance, sources.add(index));
            // SAFETY: destroying the finder releases the source array; it is
            // not used afterwards.
            (self.library.api.find_destroy)(finder);
        }

        self.connected_source = Some(name.clone());
        Ok(Some(name))
    }

    /// Captures the next video frame, waiting up to `timeout_ms`.
    ///
    /// Returns `Ok(None)` when no frame arrived (the SDK's
    /// `NDIlib_frame_type_none`), and `Err(ConnectionLost)` on
    /// `NDIlib_frame_type_error`. Audio and metadata are not requested. The
    /// captured SDK buffer is always freed with
    /// `NDIlib_recv_free_video_v2` after the pixels are copied out.
    pub fn capture(&mut self, timeout_ms: u32) -> Result<Option<NdiFrame>, NdiError> {
        if self.instance.is_null() {
            return Err(NdiError::Destroyed);
        }

        let mut frame = NdiVideoFrameV2::empty();

        // SAFETY: `self.instance` is live; `frame` is a valid writable
        // `NDIlib_video_frame_v2_t` and the audio/metadata pointers are null
        // so the SDK captures only video
        // (Processing.NDI.Recv.h:190-205). On `frame_type_video` the SDK fills
        // `frame` and transfers ownership of `frame.p_data` to us.
        let frame_type = unsafe {
            (self.library.api.recv_capture_v2)(
                self.instance,
                &mut frame,
                ptr::null_mut(),
                ptr::null_mut(),
                timeout_ms,
            )
        };

        match frame_type {
            FRAME_TYPE_NONE => Ok(None),
            FRAME_TYPE_ERROR => Err(NdiError::ConnectionLost),
            FRAME_TYPE_VIDEO => {
                let copied = copy_video_frame(&frame);

                // SAFETY: the frame was filled by `recv_capture_v2` and has not
                // been freed yet; `copy_video_frame` has already copied every
                // byte it needs. `recv_free_video_v2` must be called exactly
                // once per captured video frame (Processing.NDI.Recv.h:219-221).
                unsafe { (self.library.api.recv_free_video_v2)(self.instance, &frame) };

                copied.map(Some)
            }
            // `audio` / `metadata` / `status_change`: no buffers were requested,
            // so there is nothing to free.
            _ => Ok(None),
        }
    }

    /// Destroys the receiver instance. Idempotent; `Drop` calls it too.
    pub fn destroy(&mut self) {
        if self.instance.is_null() {
            return;
        }

        // SAFETY: `self.instance` was created by `NDIlib_recv_create_v3` and is
        // freed exactly once (nulled immediately after), as required by
        // Processing.NDI.Recv.h:180-182.
        unsafe { (self.library.api.recv_destroy)(self.instance) };
        self.instance = ptr::null_mut();
        self.connected_source = None;
    }
}

impl Drop for NdiReceiver {
    fn drop(&mut self) {
        self.destroy();
    }
}

#[cfg(test)]
mod tests {
    //! The FFI itself needs the SDK at runtime, so these tests cover everything
    //! that can be checked without it: availability detection, the resolution
    //! order, the RGBA→BGRA conversion, and the captured-frame repack. The
    //! tests that need libndi skip when it is absent (and vice versa when it is
    //! present but a network source is not).
    use super::*;

    #[test]
    fn fourcc_matches_the_sdk_macro() {
        // NDI_LIB_FOURCC('B','G','R','A') == 0x41524742
        assert_eq!(FOURCC_BGRA, 0x4152_4742);
        assert_eq!(FOURCC_RGBA, 0x4142_4752);
    }

    #[test]
    fn availability_detection_does_not_panic() {
        let first = ndi_available();
        assert_eq!(first, ndi_available());
        // A second read must not re-load or log.
        assert_eq!(ndi_library_path().is_some(), first);
    }

    #[test]
    fn resolution_order_prefers_the_env_override_then_the_system_paths() {
        let exe_dir = Path::new("/opt/vmodv/resources");

        let with_env = ndi_library_candidates_with_env(Some("/tmp/libndi.so.5"), exe_dir);
        assert_eq!(with_env[0], PathBuf::from("/tmp/libndi.so.5"));
        assert_eq!(with_env[1], PathBuf::from("/usr/lib/libndi.so.5"));
        assert_eq!(with_env[2], PathBuf::from("/usr/local/lib/libndi.so.5"));
        assert!(
            with_env.iter().any(|path| path
                .ends_with("app.asar.unpacked/node_modules/grandiose/ndi/lib/lnx-x64/libndi.so.5")),
            "the packaged grandiose copy must be a candidate: {with_env:?}"
        );

        let without_env = ndi_library_candidates_with_env(None, exe_dir);
        assert_eq!(without_env[0], PathBuf::from("/usr/lib/libndi.so.5"));
        assert_eq!(without_env.len(), with_env.len() - 1);

        // An empty override is ignored, not treated as a path.
        assert_eq!(
            ndi_library_candidates_with_env(Some(""), exe_dir),
            without_env
        );
    }

    #[test]
    fn the_search_path_is_absolute_or_relative_to_the_executable() {
        let candidates = ndi_library_search_path();
        assert!(candidates.len() >= 3);

        // The system locations come first unless $MODV_NDI_LIB overrides them;
        // one environment variable must not make this test fragile.
        if std::env::var("MODV_NDI_LIB").is_err() {
            assert_eq!(candidates[0], PathBuf::from("/usr/lib/libndi.so.5"));
            assert_eq!(candidates[1], PathBuf::from("/usr/local/lib/libndi.so.5"));
            // Every entry names the shared object.
            assert!(candidates.iter().all(|path| path.ends_with(NDI_SONAME)));
        }
    }

    #[test]
    fn a_missing_library_is_reported_without_panicking() {
        let error = NdiLibrary::try_load(&[PathBuf::from("/nonexistent/libndi.so.5")])
            .expect_err("a bogus path cannot load");
        assert!(error.contains("libndi.so.5"), "{error}");
    }

    #[test]
    fn api_operations_report_unavailable_when_the_library_is_missing() {
        if ndi_available() {
            eprintln!("skipping: libndi is available in this environment");
            return;
        }

        assert_eq!(
            NdiSender::create("modV test", false).expect_err("no library"),
            NdiError::Unavailable
        );
        assert_eq!(
            NdiReceiver::create(None).expect_err("no library"),
            NdiError::Unavailable
        );
    }

    #[test]
    fn rgba_to_bgra_swaps_red_and_blue() {
        // One pixel: R=1 G=2 B=3 A=4
        let rgba = [1u8, 2, 3, 4, 5, 6, 7, 8];

        let bgra = rgba_to_bgra_frame(2, 1, 8, &rgba).expect("convert");

        assert_eq!(bgra, vec![3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn rgba_to_bgra_drops_the_row_padding() {
        // 2×2 RGBA with a 12-byte stride (one padding pixel per row).
        let mut source = vec![0u8; 24];
        source[0..4].copy_from_slice(&[10, 11, 12, 13]); // row 0, pixel 0
        source[4..8].copy_from_slice(&[20, 21, 22, 23]); // row 0, pixel 1
        source[8..12].copy_from_slice(&[0xEE; 4]); // padding
        source[12..16].copy_from_slice(&[30, 31, 32, 33]); // row 1, pixel 0
        source[16..20].copy_from_slice(&[40, 41, 42, 43]); // row 1, pixel 1
        source[20..24].copy_from_slice(&[0xEE; 4]); // padding

        let bgra = rgba_to_bgra_frame(2, 2, 12, &source).expect("convert");

        assert_eq!(
            bgra,
            vec![
                12, 11, 10, 13, 22, 21, 20, 23, // row 0
                32, 31, 30, 33, 42, 41, 40, 43, // row 1
            ]
        );
    }

    #[test]
    fn rgba_to_bgra_rejects_invalid_frames() {
        assert!(matches!(
            rgba_to_bgra_frame(0, 1, 4, &[0; 4]),
            Err(NdiError::InvalidFrame(_))
        ));
        assert!(matches!(
            rgba_to_bgra_frame(1, 0, 4, &[0; 4]),
            Err(NdiError::InvalidFrame(_))
        ));
        // Stride smaller than one row.
        assert!(matches!(
            rgba_to_bgra_frame(2, 1, 4, &[0; 8]),
            Err(NdiError::InvalidFrame(_))
        ));
        // Buffer shorter than stride * height.
        assert!(matches!(
            rgba_to_bgra_frame(1, 2, 4, &[0; 7]),
            Err(NdiError::InvalidFrame(_))
        ));
    }

    #[test]
    fn captured_frames_are_repacked_to_a_tight_stride() {
        // 2×2 RGBA with a 12-byte stride.
        let mut data = vec![0u8; 24];
        data[0..4].copy_from_slice(&[1, 2, 3, 4]);
        data[4..8].copy_from_slice(&[5, 6, 7, 8]);
        data[8..12].copy_from_slice(&[0xEE; 4]);
        data[12..16].copy_from_slice(&[9, 10, 11, 12]);
        data[16..20].copy_from_slice(&[13, 14, 15, 16]);
        data[20..24].copy_from_slice(&[0xEE; 4]);

        let frame = NdiVideoFrameV2 {
            xres: 2,
            yres: 2,
            line_stride_in_bytes: 12,
            p_data: data.as_mut_ptr(),
            ..NdiVideoFrameV2::empty()
        };

        let copied = copy_video_frame(&frame).expect("copy");

        assert_eq!(copied.width, 2);
        assert_eq!(copied.height, 2);
        assert_eq!(copied.stride, 8);
        assert_eq!(
            copied.pixels,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn captured_frames_default_a_zero_stride_to_width_times_four() {
        let mut data = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let frame = NdiVideoFrameV2 {
            xres: 2,
            yres: 1,
            line_stride_in_bytes: 0,
            p_data: data.as_mut_ptr(),
            ..NdiVideoFrameV2::empty()
        };

        let copied = copy_video_frame(&frame).expect("copy");

        assert_eq!(copied.stride, 8);
        assert_eq!(copied.pixels, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn invalid_captured_frames_are_rejected() {
        let mut data = vec![0u8; 4];

        let empty = NdiVideoFrameV2 {
            xres: 0,
            yres: 1,
            p_data: data.as_mut_ptr(),
            ..NdiVideoFrameV2::empty()
        };
        assert!(matches!(
            copy_video_frame(&empty),
            Err(NdiError::InvalidFrame(_))
        ));

        let null_data = NdiVideoFrameV2 {
            xres: 1,
            yres: 1,
            line_stride_in_bytes: 4,
            ..NdiVideoFrameV2::empty()
        };
        assert!(matches!(
            copy_video_frame(&null_data),
            Err(NdiError::InvalidFrame(_))
        ));

        let short_stride = NdiVideoFrameV2 {
            xres: 2,
            yres: 1,
            line_stride_in_bytes: 4,
            p_data: data.as_mut_ptr(),
            ..NdiVideoFrameV2::empty()
        };
        assert!(matches!(
            copy_video_frame(&short_stride),
            Err(NdiError::InvalidFrame(_))
        ));
    }
}
