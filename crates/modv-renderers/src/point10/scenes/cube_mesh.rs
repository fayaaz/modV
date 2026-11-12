//! Geometry and matrix helpers shared by the batch-4 cube scenes.
//!
//! Four of the batch-4 scenes (`MirrorCube`, `ColorCube`, `Boxy`, `SoundCube`)
//! are three.js `BoxGeometry` scenes with a `PerspectiveCamera`, a model
//! matrix and a view-projection matrix. All five of them also need a depth
//! attachment, because they draw closed opaque solids.
//!
//! This module is the batch-4 counterpart of [`super::particle_quad`] (the
//! instanced billboard pipeline ParticleField and SpiralParticles share): not a
//! scene, just the pieces the scenes have in common.
//!
//! [`crate::point10::scenes::around_plus`] carries its own private copies of
//! [`Mat4`] and [`create_depth`] from batch 3. They are deliberately not
//! refactored onto this module here: the two batches stay independently
//! reviewable, and moving a batch-3 scene's maths would put this batch's diff
//! into a file it does not otherwise touch.
//!
//! # `build_box`
//!
//! `BoxGeometry(width, height, depth)` with 1 segment per axis: 24 vertices
//! (4 per face), 36 indices, per-face `uv` in `0..1` with the v axis flipped
//! (`uvs.push(ix / gridX); uvs.push(1 - (iy / gridY))`). The vertex order and
//! the index order are three's, so a scene can be checked against the JS
//! side by side.
//!
//! # `uv` orientation
//!
//! three's box uvs put `v = 1` at the visual top of a face, which lines up with
//! its default `flipY = true` texture uploads. [`super::super::shared::assets`]
//! uploads images unflipped (row 0 = the image's top row = `v = 0`), so a scene
//! that samples an image with these uvs inverts `v` in its shader (Boxy does,
//! with one line and a note); scenes whose sampling is v-symmetric
//! (SoundCube's rings, MirrorCube's feedback map) leave it alone. Every triangle is counter-clockwise seen from outside (the
//! `Boxy`/`SoundCube` UVs and the `ColorCube` normals depend on it).
//!
//! # Culling
//!
//! The batch-4 pipelines draw with `cull_mode: None` and a depth test instead
//! of three's `FrontSide` cull. For a closed convex solid the result is the
//! same image (the nearest surface wins), and a winding slip can never hide
//! the object — the same trade-off `AroundPlus` documents for its inverted
//! torus. `SoundCube` is the one scene that opts out of the depth test to
//! match the JS (`depthTest: false`); it still relies on this winding being
//! outward, which its tests assert.

#![forbid(unsafe_code)]

/// One interleaved mesh vertex (`position`, `normal`, `uv`).
///
/// Attribute locations 0/1/2. Instanced scenes put their per-instance data at
/// locations 4/5 so the two layouts never collide.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// Local position.
    pub position: [f32; 3],
    /// Local normal.
    pub normal: [f32; 3],
    /// Face texture coordinate (`v` = 0 at the top row of the face).
    pub uv: [f32; 2],
}

impl Vertex {
    /// Buffer layout for `@location(0..2)`.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 24,
                    shader_location: 2,
                },
            ],
        }
    }
}

/// A CPU-side mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// Vertices.
    pub vertices: Vec<Vertex>,
    /// Triangle indices (`u32`; the knot has more than 65 535 vertices).
    pub indices: Vec<u32>,
}

/// `THREE.BoxGeometry(width, height, depth)` — one segment per axis.
pub fn build_box(width: f32, height: f32, depth: f32) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.vertices.reserve(24);
    mesh.indices.reserve(36);

    // three's buildPlane() calls, in order: px, nx, py, ny, pz, nz. The last
    // field is the signed plane offset; its sign picks the face normal.
    let planes = [
        Plane {
            u: 2,
            v: 1,
            w: 0,
            u_dir: -1.0,
            v_dir: -1.0,
            width: depth,
            height,
            depth: width,
        },
        Plane {
            u: 2,
            v: 1,
            w: 0,
            u_dir: 1.0,
            v_dir: -1.0,
            width: depth,
            height,
            depth: -width,
        },
        Plane {
            u: 0,
            v: 2,
            w: 1,
            u_dir: 1.0,
            v_dir: 1.0,
            width,
            height: depth,
            depth: height,
        },
        Plane {
            u: 0,
            v: 2,
            w: 1,
            u_dir: 1.0,
            v_dir: -1.0,
            width,
            height: depth,
            depth: -height,
        },
        Plane {
            u: 0,
            v: 1,
            w: 2,
            u_dir: 1.0,
            v_dir: -1.0,
            width,
            height,
            depth,
        },
        Plane {
            u: 0,
            v: 1,
            w: 2,
            u_dir: -1.0,
            v_dir: -1.0,
            width,
            height,
            depth: -depth,
        },
    ];
    for plane in planes {
        build_plane(&mut mesh, plane);
    }

    mesh
}

/// One `buildPlane` argument set: the axis permutation, the direction signs and
/// the (signed) plane size/offset.
#[derive(Clone, Copy, Debug)]
struct Plane {
    u: usize,
    v: usize,
    w: usize,
    u_dir: f32,
    v_dir: f32,
    width: f32,
    height: f32,
    depth: f32,
}

/// One face of [`build_box`] (`BoxGeometry.buildPlane`, 1x1 segments).
fn build_plane(mesh: &mut Mesh, plane: Plane) {
    let Plane {
        u,
        v,
        w,
        u_dir,
        v_dir,
        width,
        height,
        depth,
    } = plane;
    let base = mesh.vertices.len() as u32;
    let mut vector = [0.0_f32; 3];

    for iy in 0..2 {
        let y = iy as f32 * height - height * 0.5;
        for ix in 0..2 {
            let x = ix as f32 * width - width * 0.5;

            vector[u] = x * u_dir;
            vector[v] = y * v_dir;
            vector[w] = depth * 0.5;
            let position = vector;

            vector[u] = 0.0;
            vector[v] = 0.0;
            vector[w] = if depth > 0.0 { 1.0 } else { -1.0 };
            let normal = normalise(vector);

            mesh.vertices.push(Vertex {
                position,
                normal,
                uv: [ix as f32, 1.0 - iy as f32],
            });
        }
    }

    // gridX1 = 2: a = ix + 2 * iy, b = ix + 2 * (iy + 1), c/d one column right.
    let a = base;
    let b = base + 2;
    let c = base + 3;
    let d = base + 1;
    mesh.indices.extend_from_slice(&[a, b, d, b, c, d]);
}

/// `Vector3.normalize()` with a zero guard.
pub fn normalise(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2])
        .sqrt()
        .max(1e-9);
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

/// A mesh uploaded in `init`.
pub struct MeshGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

impl MeshGpu {
    /// Upload `mesh` as `u32` triangles.
    pub fn new(device: &wgpu::Device, label: &str, mesh: &Mesh) -> Self {
        use wgpu::util::DeviceExt;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label}-vertices")),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label}-indices")),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
        }
    }

    /// Bind and draw one instance.
    pub fn draw<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        self.draw_instanced(pass, 1);
    }

    /// Bind and draw `instances` copies.
    pub fn draw_instanced<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>, instances: u32) {
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..instances);
    }
}

// ---------------------------------------------------------------------------
// Matrices (column-major, matching WGSL's mat4x4<f32>)
// ---------------------------------------------------------------------------

/// A column-major 4x4 matrix.
pub type Mat4 = [[f32; 4]; 4];

/// Identity.
pub fn identity() -> Mat4 {
    let mut matrix = [[0.0_f32; 4]; 4];
    for (index, column) in matrix.iter_mut().enumerate() {
        column[index] = 1.0;
    }
    matrix
}

/// `a * b`.
pub fn mat4_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0_f32; 4]; 4];
    for column in 0..4 {
        for row in 0..4 {
            out[column][row] = (0..4).map(|k| a[k][row] * b[column][k]).sum();
        }
    }
    out
}

/// Translation.
pub fn translation(x: f32, y: f32, z: f32) -> Mat4 {
    let mut matrix = identity();
    matrix[3] = [x, y, z, 1.0];
    matrix
}

/// Scale.
pub fn scale(x: f32, y: f32, z: f32) -> Mat4 {
    let mut matrix = identity();
    matrix[0][0] = x;
    matrix[1][1] = y;
    matrix[2][2] = z;
    matrix
}

/// Rotation about X.
pub fn rotation_x(angle: f32) -> Mat4 {
    let (s, c) = angle.sin_cos();
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, c, s, 0.0],
        [0.0, -s, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Rotation about Y.
pub fn rotation_y(angle: f32) -> Mat4 {
    let (s, c) = angle.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Rotation about Z.
pub fn rotation_z(angle: f32) -> Mat4 {
    let (s, c) = angle.sin_cos();
    [
        [c, s, 0.0, 0.0],
        [-s, c, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// `Euler(x, y, z, "XYZ")` → `Rx * Ry * Rz` (three's `Matrix4.makeRotationFromEuler`).
pub fn euler_xyz(x: f32, y: f32, z: f32) -> Mat4 {
    mat4_mul(&rotation_x(x), &mat4_mul(&rotation_y(y), &rotation_z(z)))
}

/// `Object3D.matrix` for a `position`/`rotation(XYZ)`/`scale` triple.
pub fn trs(position: [f32; 3], rotation: [f32; 3], scaling: [f32; 3]) -> Mat4 {
    mat4_mul(
        &translation(position[0], position[1], position[2]),
        &mat4_mul(
            &euler_xyz(rotation[0], rotation[1], rotation[2]),
            &scale(scaling[0], scaling[1], scaling[2]),
        ),
    )
}

/// `PerspectiveCamera(fov, aspect, near, far).projectionMatrix`.
pub fn perspective(fov_y_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fov_y_radians * 0.5).tan();
    let range = near - far;
    [
        [f / aspect.max(1e-6), 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, (far + near) / range, -1.0],
        [0.0, 0.0, (2.0 * far * near) / range, 0.0],
    ]
}

/// `Matrix4.lookAt(eye, target, up)` (the camera's `matrixWorldInverse`).
pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Mat4 {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

    let z = normalise(sub(eye, target));
    let x = normalise(cross(up, z));
    let y = cross(z, x);

    [
        [x[0], y[0], z[0], 0.0],
        [x[1], y[1], z[1], 0.0],
        [x[2], y[2], z[2], 0.0],
        [-dot(x, eye), -dot(y, eye), -dot(z, eye), 1.0],
    ]
}

/// Project `point` through `matrix` into clip space.
pub fn transform_point(matrix: &Mat4, point: [f32; 3]) -> [f32; 4] {
    let mut out = [0.0_f32; 4];
    for (row, value) in out.iter_mut().enumerate() {
        *value = matrix[0][row] * point[0]
            + matrix[1][row] * point[1]
            + matrix[2][row] * point[2]
            + matrix[3][row];
    }
    out
}

// ---------------------------------------------------------------------------
// Depth
// ---------------------------------------------------------------------------

/// Depth format of every batch-4 scene.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Opaque depth state (`depthTest: true`, `depthWrite: true`).
pub fn depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// Create a depth texture and its view.
pub fn create_depth(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_has_three_js_vertex_and_index_counts() {
        let mesh = build_box(1.0, 1.0, 1.0);
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
        assert_eq!(std::mem::size_of::<Vertex>(), 32);
    }

    #[test]
    fn box_spans_the_requested_size() {
        let mesh = build_box(2.0, 4.0, 6.0);
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for vertex in &mesh.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
        }
        assert_eq!(min, [-1.0, -2.0, -3.0]);
        assert_eq!(max, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn box_face_normals_point_outward_and_match_the_winding() {
        let mesh = build_box(1.0, 1.0, 1.0);
        for vertex in &mesh.vertices {
            // Every vertex normal is a unit axis vector matching its face.
            assert!((length(vertex.normal) - 1.0).abs() < 1e-6);
            // The face plane the vertex sits on is `dot(normal, position) > 0`.
            let along = dot(vertex.normal, vertex.position);
            assert!(along > 0.4, "{vertex:?} normal {along}");
        }

        // Every triangle's geometric normal agrees with its vertices' normals.
        for triangle in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [
                mesh.vertices[triangle[0] as usize],
                mesh.vertices[triangle[1] as usize],
                mesh.vertices[triangle[2] as usize],
            ];
            let geometric = normalise(cross(
                sub(b.position, a.position),
                sub(c.position, a.position),
            ));
            let shaded = normalise(add(add(a.normal, b.normal), c.normal));
            assert!(
                dot(geometric, shaded) > 0.9,
                "triangle {triangle:?} winds inward: {geometric:?} vs {shaded:?}"
            );
        }
    }

    #[test]
    fn box_uvs_are_per_face_and_v_flipped() {
        let mesh = build_box(1.0, 1.0, 1.0);
        for vertex in &mesh.vertices {
            assert!((0.0..=1.0).contains(&vertex.uv[0]));
            assert!((0.0..=1.0).contains(&vertex.uv[1]));
        }
        // `BoxGeometry.buildPlane` walks `for (iy) for (ix)` with
        // `uvs.push(ix / gridX); uvs.push(1 - iy / gridY)`, so the first row
        // has v = 1 and u increases first: (0,1), (1,1), (0,0), (1,0). v = 1
        // is three's "top of the image", which needs a v flip against an
        // unflipped upload — see the module docs.
        assert_eq!(mesh.vertices[0].uv, [0.0, 1.0]);
        assert_eq!(mesh.vertices[1].uv, [1.0, 1.0]);
        assert_eq!(mesh.vertices[2].uv, [0.0, 0.0]);
        assert_eq!(mesh.vertices[3].uv, [1.0, 0.0]);
    }

    #[test]
    fn matrices_match_three() {
        // Rotation about each axis keeps the axis and maps the others.
        let spun = transform_point(&rotation_y(std::f32::consts::FRAC_PI_2), [1.0, 0.0, 0.0]);
        assert!(spun[0].abs() < 1e-6, "{spun:?}");
        assert!((spun[2] + 1.0).abs() < 1e-6, "{spun:?}");

        // Euler XYZ is R = Rx * Ry * Rz (three's column-vector convention):
        // rotating by (90°, 90°, 0) leaves +y alone under Ry and maps it to
        // +z under Rx.
        let euler = euler_xyz(
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::FRAC_PI_2,
            0.0,
        );
        let y_axis = transform_point(&euler, [0.0, 1.0, 0.0]);
        assert!((y_axis[2] - 1.0).abs() < 1e-5, "{y_axis:?}");

        // trs(): scale first, then rotate, then translate.
        let model = trs([1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [2.0, 2.0, 2.0]);
        let point = transform_point(&model, [1.0, 0.0, 0.0]);
        assert!((point[0] - 3.0).abs() < 1e-6, "{point:?}");

        // The perspective camera maps the near plane to -1 and far to +1.
        let projection = perspective(60_f32.to_radians(), 1.0, 0.1, 1000.0);
        let near = transform_point(&projection, [0.0, 0.0, -0.1]);
        assert!((near[2] / near[3] + 1.0).abs() < 1e-5, "{near:?}");
        let far = transform_point(&projection, [0.0, 0.0, -1000.0]);
        assert!((far[2] / far[3] - 1.0).abs() < 1e-4, "{far:?}");

        // A camera at +z looking at the origin sees the origin 5 units ahead.
        let view = look_at([0.0, 0.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let origin = transform_point(&view, [0.0, 0.0, 0.0]);
        assert!(origin[0].abs() < 1e-5 && origin[1].abs() < 1e-5);
        assert!((origin[2] + 5.0).abs() < 1e-5, "{origin:?}");
    }

    #[test]
    fn depth_state_is_opaque_less_than() {
        let state = depth_state();
        assert_eq!(state.format, DEPTH_FORMAT);
        assert_eq!(state.depth_write_enabled, Some(true));
        assert_eq!(state.depth_compare, Some(wgpu::CompareFunction::Less));
    }

    fn length(v: [f32; 3]) -> f32 {
        dot(v, v).sqrt()
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
}
