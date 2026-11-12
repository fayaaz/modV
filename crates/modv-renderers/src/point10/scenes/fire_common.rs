//! Shared helpers for the two fire scenes (RollingFire, SmokeRoom).
//!
//! Both JS scenes are WebGPU/TSL volumetric fire simulations built from the
//! same parts: a UV sphere emitter, a `VolumeNodeMaterial` raymarch of a 3D
//! dye grid, and a lava emissive material on the sphere. SmokeRoom (and DVD
//! Room) now run the real simulation through
//! [`crate::point10::volume::VolumeSim`]; Rolling Fire still uses additive haze
//! puffs (see `rolling_fire.rs`), and both share:
//!
//! * [`build_sphere`] — `SphereGeometry(radius, widthSegments, heightSegments)`
//!   with three's UV-sphere topology and vertex order, so the emitter mesh and
//!   the lava shader's `positionLocal` match the JS.
//! * [`sphere_surface_sample`] — a deterministic point on the sphere, used to
//!   seed puffs where the JS's `emitSpherePass` writes into the dye grid.
//!
//! The remaining shared pieces (`fire_ramp`, glass fitting, the box extents,
//! the audio bands and the simulation clock) live in
//! [`crate::point10::volume::room`], which ports SmokeRoom's room maths for
//! both of its scenes.

#![forbid(unsafe_code)]

use crate::point10::scenes::cube_mesh::{Mesh, Vertex};

/// `THREE.SphereGeometry(radius, widthSegments, heightSegments)`.
///
/// three builds the sphere from top to bottom in `heightSegments + 1` rows of
/// `widthSegments + 1` vertices; the winding is counter-clockwise from
/// outside. The UVs match three (`u = ix / widthSegments`, `v = iy /
/// heightSegments` with row 0 at the +Y pole) so a texture uploaded the shared
/// loader's way samples like the JS.
pub fn build_sphere(radius: f32, width_segments: u32, height_segments: u32) -> Mesh {
    let width = width_segments.max(3);
    let height = height_segments.max(2);
    let radius = radius.max(1e-4);
    let mut mesh = Mesh::default();

    for iy in 0..=height {
        let v = iy as f32 / height as f32;
        let theta = v * std::f32::consts::PI;
        for ix in 0..=width {
            let u = ix as f32 / width as f32;
            let phi = u * std::f32::consts::TAU;
            let normal = [
                -(phi.sin()) * theta.sin(),
                theta.cos(),
                (phi.cos()) * theta.sin(),
            ];
            mesh.vertices.push(Vertex {
                position: [normal[0] * radius, normal[1] * radius, normal[2] * radius],
                normal,
                uv: [u, v],
            });
        }
    }

    for iy in 0..height {
        for ix in 0..width {
            let row = iy * (width + 1);
            let next_row = (iy + 1) * (width + 1);
            let a = row + ix;
            let b = next_row + ix;
            let c = next_row + ix + 1;
            let d = row + ix + 1;
            if iy != 0 {
                mesh.indices.extend_from_slice(&[a, b, d]);
            }
            if iy != height - 1 {
                mesh.indices.extend_from_slice(&[b, c, d]);
            }
        }
    }

    mesh
}

/// A deterministic point on a sphere's surface.
///
/// The JS emitter seeds the simulation from the sphere's vertices; the puff
/// substitutes only need a uniform-ish distribution, which the golden-angle
/// spiral gives without a growing table:
/// `y = 1 - 2 * (i + 0.5) / count`, `theta = i * goldenAngle`.
pub fn sphere_surface_sample(index: u32, count: u32, radius: f32) -> [f32; 3] {
    let count = count.max(1);
    let fraction = (index as f32 + 0.5) / count as f32;
    let y = 1.0 - 2.0 * fraction;
    let ring = (1.0 - y * y).max(0.0).sqrt();
    let theta = index as f32 * 2.399_963_2;
    [
        theta.cos() * ring * radius,
        y * radius,
        theta.sin() * ring * radius,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_has_three_topology() {
        let mesh = build_sphere(1.0, 8, 6);
        assert_eq!(mesh.vertices.len(), (9 * 7) as usize);
        // Two triangles per quad, minus the degenerate cap triangles.
        assert_eq!(mesh.indices.len(), (8 * 6 * 2 - 8 * 2) * 3);
        for vertex in &mesh.vertices {
            let length = (vertex.position[0].powi(2)
                + vertex.position[1].powi(2)
                + vertex.position[2].powi(2))
            .sqrt();
            assert!((length - 1.0).abs() < 1e-5);
        }
        assert_eq!(mesh.vertices[0].uv, [0.0, 0.0]);
    }

    #[test]
    fn sphere_samples_cover_the_surface() {
        let first = sphere_surface_sample(0, 64, 0.9);
        let last = sphere_surface_sample(63, 64, 0.9);
        assert!(first[1] > 0.0);
        assert!(last[1] < 0.0);
        for index in 0..64 {
            let point = sphere_surface_sample(index, 64, 0.9);
            let length = (point[0].powi(2) + point[1].powi(2) + point[2].powi(2)).sqrt();
            assert!((length - 0.9).abs() < 1e-4);
        }
        // Deterministic.
        assert_eq!(
            sphere_surface_sample(7, 64, 1.0),
            sphere_surface_sample(7, 64, 1.0)
        );
    }
}
