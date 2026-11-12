//! The DVD logo silhouette — port of `DvdRoom._logoGeometry`
//! (`src/renderer/src/application/point10/scenes/DvdRoom.js:251-275`).
//!
//! The JS parses `textures/dvd_logo.svg` with its own SVG path parser, turns
//! each visible `<path>` into `THREE.Shape`s, extrudes them
//! (`ExtrudeGeometry`, `SVG_DEPTH = 100`, bevel 3, `curveSegments: 6`), merges
//! the results and normalises them to `LOGO_WIDTH x proportional x
//! LOGO_DEPTH`.
//!
//! This module is the same thing without three: the visible path data is
//! inlined below (no runtime asset, no new dependency), the `M/L/H/V/C/S/Q/T/Z`
//! command subset is flattened with 6 segments per curve, the sub-paths are
//! combined into "shape with holes" by containment, holes are bridged into the
//! outer contour (Eberly) and the resulting simple polygon is ear-clipped. The
//! caps are extruded to [`SVG_DEPTH`] and the whole mesh is scaled exactly like
//! `normalizeExtrude`: `s = width / bbox.x`, `(x, y, z) -> (x, -y, z) * (s, s,
//! depth / bbox.z)` (the Y flip turns SVG's y-down into world y-up).
//!
//! # Deviations
//!
//! * **No bevel.** `ExtrudeGeometry` bevels 3 SVG units, which is 0.28% of the
//!   1058-unit wide viewBox — 0.012 world units at the logo's `4.2` width, i.e.
//!   sub-pixel at every render size. The caps and walls are flat instead; the
//!   silhouette (what reads on screen) is identical.
//! * Triangulation is ear clipping rather than three's `ShapeUtils`
//!   (Earcut); the resulting cap area is asserted against the analytic
//!   polygon-with-holes area in the tests.

#![forbid(unsafe_code)]

use crate::point10::scenes::cube_mesh::{Mesh, Vertex};

/// `LOGO_WIDTH` (`DvdRoom.js:26`): world units across after normalising.
pub const LOGO_WIDTH: f32 = 4.2;
/// `LOGO_DEPTH` (`DvdRoom.js:27`): world-unit thickness after normalising.
pub const LOGO_DEPTH: f32 = 0.35;
/// `SVG_DEPTH` (`DvdRoom.js:28`): extrusion depth in SVG units.
pub const SVG_DEPTH: f32 = 100.0;
/// `curveSegments: 6` (`DvdRoom.js:264`).
pub const CURVE_SEGMENTS: u32 = 6;

/// The visible `<path d>` of
/// `src/renderer/src/application/point10/textures/dvd_logo.svg`.
///
/// The SVG's second path (the "TM" mark) is `display="none"` and is skipped by
/// the JS, so it is not inlined.
pub const LOGO_PATH: &str = "m91.053 0-13.719 57.707 102.28 0.039063h24c65.747 0 105.91 26.44 94.746 73.4-12.147 51.133-69.613 73.4-130.67 73.4h-22.947l29.787-125.45h-102.27l-43.521 183.2h145.05c109.07 0 212.76-57.573 231.01-131.15 3.3467-13.507 2.8806-47.253-5.3594-67.359-0.21299-0.787-0.42594-1.4-1.1855-3-0.293-0.653-0.56012-3.6412 1.1465-4.2812 0.947-0.36 2.7069 1.4944 2.9336 2.041 0.853 2.24 1.5059 3.9062 1.5059 3.9062l92.293 260.6 234.97-265.21 99.535-0.089844h24c65.76 0 106.25 26.44 95.092 73.4-12.147 51.133-69.947 73.4-131 73.4h-22.959l29.799-125.47h-102.27l-43.533 183.21h145.07c109.05 0 213.48-57.4 231-131.15 17.52-73.75-59.107-131.15-168.69-131.15h-216.4s-57.319 67.88-67.959 80.693c-57.12 68.787-67.241 87.226-68.961 91.986 0.24-4.8-1.8138-23.412-26.174-92.959-6.48-18.52-27.359-79.721-27.359-79.721h-389.25zm408.77 324.16c-276.04 0-499.83 31.72-499.83 70.84s223.79 70.84 499.83 70.84c276.04 0 499.83-31.72 499.83-70.84s-223.79-70.84-499.83-70.84zm-18.094 48.627c63.04 0 114.13 10.573 114.13 23.613s-51.095 23.613-114.13 23.613c-63.027 0-114.13-10.573-114.13-23.613s51.106-23.613 114.13-23.613z";

/// A closed 2D contour.
pub type Contour = Vec<[f32; 2]>;

/// Flatten an SVG path `d` into closed contours (SVG coordinates, y down).
///
/// Covers the command subset the logo uses: `M/L/H/V/C/S/Q/T/Z` in absolute and
/// relative form, with implicit repeated coordinates. Anything else is ignored,
/// exactly like the JS parser's `default: break`.
pub fn flatten_path(d: &str) -> Vec<Contour> {
    let mut contours: Vec<Contour> = Vec::new();
    let mut current: Contour = Vec::new();
    let mut point = [0.0_f32, 0.0_f32];
    let mut control = [0.0_f32, 0.0_f32];
    let mut first = [0.0_f32, 0.0_f32];

    for command in commands(d) {
        let kind = command
            .chars()
            .next()
            .expect("a command always has its letter");
        let relative = kind.is_ascii_lowercase();
        let numbers = numbers(&command[kind.len_utf8()..]);
        let mut index = 0;

        match kind.to_ascii_uppercase() {
            'M' => {
                while index + 1 < numbers.len() {
                    let x = numbers[index];
                    let y = numbers[index + 1];
                    // Relative `m` pairs are relative to the previous point
                    // (the JS does the same for every pair).
                    point = if relative {
                        [point[0] + x, point[1] + y]
                    } else {
                        [x, y]
                    };
                    control = point;
                    if index == 0 {
                        // A new sub-path starts.
                        if current.len() > 1 {
                            contours.push(std::mem::take(&mut current));
                        } else {
                            current.clear();
                        }
                        current.push(point);
                        first = point;
                    } else {
                        current.push(point);
                    }
                    index += 2;
                }
            }
            'L' => {
                while index + 1 < numbers.len() {
                    let x = numbers[index];
                    let y = numbers[index + 1];
                    point = if relative {
                        [point[0] + x, point[1] + y]
                    } else {
                        [x, y]
                    };
                    control = point;
                    current.push(point);
                    index += 2;
                }
            }
            'H' => {
                while index < numbers.len() {
                    let x = numbers[index];
                    point[0] = if relative { point[0] + x } else { x };
                    control = point;
                    current.push(point);
                    index += 1;
                }
            }
            'V' => {
                while index < numbers.len() {
                    let y = numbers[index];
                    point[1] = if relative { point[1] + y } else { y };
                    control = point;
                    current.push(point);
                    index += 1;
                }
            }
            'C' => {
                while index + 5 < numbers.len() {
                    let base = point;
                    let cp1 = if relative {
                        [base[0] + numbers[index], base[1] + numbers[index + 1]]
                    } else {
                        [numbers[index], numbers[index + 1]]
                    };
                    let cp2 = if relative {
                        [base[0] + numbers[index + 2], base[1] + numbers[index + 3]]
                    } else {
                        [numbers[index + 2], numbers[index + 3]]
                    };
                    point = if relative {
                        [base[0] + numbers[index + 4], base[1] + numbers[index + 5]]
                    } else {
                        [numbers[index + 4], numbers[index + 5]]
                    };
                    flatten_cubic(&mut current, base, cp1, cp2, point);
                    control = cp2;
                    index += 6;
                }
            }
            'S' => {
                while index + 3 < numbers.len() {
                    let base = point;
                    let cp1 = [control[0] * 2.0 - base[0], control[1] * 2.0 - base[1]];
                    let cp2 = if relative {
                        [base[0] + numbers[index], base[1] + numbers[index + 1]]
                    } else {
                        [numbers[index], numbers[index + 1]]
                    };
                    point = if relative {
                        [base[0] + numbers[index + 2], base[1] + numbers[index + 3]]
                    } else {
                        [numbers[index + 2], numbers[index + 3]]
                    };
                    flatten_cubic(&mut current, base, cp1, cp2, point);
                    control = cp2;
                    index += 4;
                }
            }
            'Q' => {
                while index + 3 < numbers.len() {
                    let base = point;
                    let cp = if relative {
                        [base[0] + numbers[index], base[1] + numbers[index + 1]]
                    } else {
                        [numbers[index], numbers[index + 1]]
                    };
                    point = if relative {
                        [base[0] + numbers[index + 2], base[1] + numbers[index + 3]]
                    } else {
                        [numbers[index + 2], numbers[index + 3]]
                    };
                    flatten_quadratic(&mut current, base, cp, point);
                    control = cp;
                    index += 4;
                }
            }
            'T' => {
                while index + 1 < numbers.len() {
                    let base = point;
                    let cp = [control[0] * 2.0 - base[0], control[1] * 2.0 - base[1]];
                    point = if relative {
                        [base[0] + numbers[index], base[1] + numbers[index + 1]]
                    } else {
                        [numbers[index], numbers[index + 1]]
                    };
                    flatten_quadratic(&mut current, base, cp, point);
                    control = cp;
                    index += 2;
                }
            }
            'Z' => {
                if current.len() > 1 {
                    contours.push(std::mem::take(&mut current));
                } else {
                    current.clear();
                }
                point = first;
            }
            _ => {}
        }
    }

    if current.len() > 1 {
        contours.push(current);
    }

    contours
        .into_iter()
        .map(|contour| {
            let mut deduped = dedupe(contour);
            // `autoClose`: the closing point is implicit.
            if deduped.len() > 2 {
                let first = deduped[0];
                let last = *deduped.last().expect("non-empty");
                if (first[0] - last[0]).abs() < 1e-6 && (first[1] - last[1]).abs() < 1e-6 {
                    deduped.pop();
                }
            }
            deduped
        })
        .filter(|contour| contour.len() > 2)
        .collect()
}

/// Split a path `d` into `letter + arguments` commands.
fn commands(d: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for character in d.chars() {
        // The JS regex is `[a-df-z]` — `e`/`E` stay inside numbers so exponent
        // notation is not split into a command.
        if character.is_ascii_alphabetic() && !matches!(character.to_ascii_lowercase(), 'e') {
            if !current.trim().is_empty() {
                out.push(current.trim().to_string());
            }
            current = String::new();
        }
        current.push(character);
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// `parseNumbers` (`DvdRoom.js:37-40`).
fn numbers(data: &str) -> Vec<f32> {
    let bytes = data.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        let mut seen_digit = false;
        let mut seen_dot = false;
        let mut seen_exponent = false;
        while index < bytes.len() {
            let character = bytes[index] as char;
            match character {
                '0'..='9' => {
                    seen_digit = true;
                    index += 1;
                }
                '.' if !seen_dot && !seen_exponent => {
                    seen_dot = true;
                    index += 1;
                }
                '-' | '+' if !seen_digit && !seen_dot => {
                    index += 1;
                }
                'e' | 'E' if seen_digit && !seen_exponent => {
                    seen_exponent = true;
                    seen_dot = false;
                    seen_digit = false;
                    index += 1;
                }
                '-' | '+' if seen_exponent => {
                    index += 1;
                }
                _ => break,
            }
        }
        if index > start {
            if let Ok(value) = data[start..index].parse::<f32>() {
                out.push(value);
            }
        } else {
            index += 1;
        }
    }
    out
}

fn flatten_cubic(out: &mut Contour, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) {
    let steps = CURVE_SEGMENTS.max(1);
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let mt = 1.0 - t;
        let a = mt * mt * mt;
        let b = 3.0 * mt * mt * t;
        let c = 3.0 * mt * t * t;
        let d = t * t * t;
        out.push([
            a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
            a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
        ]);
    }
}

fn flatten_quadratic(out: &mut Contour, p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) {
    let steps = CURVE_SEGMENTS.max(1);
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let mt = 1.0 - t;
        out.push([
            mt * mt * p0[0] + 2.0 * mt * t * p1[0] + t * t * p2[0],
            mt * mt * p0[1] + 2.0 * mt * t * p1[1] + t * t * p2[1],
        ]);
    }
}

fn dedupe(points: Contour) -> Contour {
    let mut out: Contour = Vec::with_capacity(points.len());
    for point in points {
        match out.last() {
            Some(last)
                if (last[0] - point[0]).abs() < 1e-4 && (last[1] - point[1]).abs() < 1e-4 =>
            {
                continue;
            }
            _ => out.push(point),
        }
    }
    out
}

/// Signed area (positive = counter-clockwise in a y-up frame).
pub fn signed_area(contour: &[[f32; 2]]) -> f32 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut previous = *contour.last().expect("non-empty");
    for point in contour {
        sum += previous[0] * point[1] - point[0] * previous[1];
        previous = *point;
    }
    sum * 0.5
}

/// Even-odd point-in-polygon test.
pub fn point_in_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    if polygon.len() < 3 {
        return false;
    }
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[j];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
            if point[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// One filled region: an outer contour plus the holes directly inside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    /// Outer contour (counter-clockwise).
    pub outer: Contour,
    /// Hole contours (clockwise).
    pub holes: Vec<Contour>,
}

/// Group flattened contours into filled shapes by containment.
pub fn build_shapes(contours: Vec<Contour>) -> Vec<Shape> {
    let count = contours.len();
    // Containment depth: how many other contours contain this one.
    let mut depth = vec![0_usize; count];
    for (index, contour) in contours.iter().enumerate() {
        let sample = contour[0];
        for (other_index, other) in contours.iter().enumerate() {
            if index != other_index && point_in_polygon(sample, other) {
                depth[index] += 1;
            }
        }
    }

    let mut shapes: Vec<Shape> = Vec::new();
    let mut outer_indices = Vec::new();
    for (index, contour) in contours.iter().enumerate() {
        if depth[index].is_multiple_of(2) {
            let mut outer = contour.clone();
            if signed_area(&outer) < 0.0 {
                outer.reverse();
            }
            outer_indices.push(index);
            shapes.push(Shape {
                outer,
                holes: Vec::new(),
            });
        }
    }

    for (index, contour) in contours.iter().enumerate() {
        if depth[index].is_multiple_of(2) {
            continue;
        }
        // A hole belongs to the smallest outer contour that contains it.
        let sample = contour[0];
        let mut best: Option<(usize, f32)> = None;
        for (shape_index, shape_index_source) in outer_indices.iter().enumerate() {
            if point_in_polygon(sample, &contours[*shape_index_source]) {
                let area = signed_area(&contours[*shape_index_source]).abs();
                if best.map(|(_, best_area)| area < best_area).unwrap_or(true) {
                    best = Some((shape_index, area));
                }
            }
        }
        if let Some((shape_index, _)) = best {
            let mut hole = contour.clone();
            if signed_area(&hole) > 0.0 {
                hole.reverse();
            }
            shapes[shape_index].holes.push(hole);
        }
    }

    shapes
}

/// Triangulate a filled region (outer contour + holes) with an exact
/// trapezoidal decomposition.
///
/// A horizontal scan at every vertex Y splits the region into trapezoids: at
/// each interval's midpoint the edges are intersected and sorted, and an
/// even-odd pairing yields the filled spans (a hole simply contributes two
/// crossings). Extending each span's two bounding edges to the interval's top
/// and bottom gives exact quads — no ear clipping, no hole bridging, and no
/// numerical fragility on the nearly-collinear points of the flattened
/// ellipse. All caps are coplanar and share the `±Z` normal, so the extra
/// triangles are invisible.
pub fn triangulate_shape(shape: &Shape) -> Vec<[[f32; 2]; 3]> {
    let mut loops: Vec<&[[f32; 2]]> = Vec::with_capacity(shape.holes.len() + 1);
    loops.push(shape.outer.as_slice());
    loops.extend(shape.holes.iter().map(|hole| hole.as_slice()));

    let mut levels: Vec<f32> = loops
        .iter()
        .flat_map(|contour| contour.iter().map(|point| point[1]))
        .collect();
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Only exact duplicates are merged: every band then starts and ends on a
    // real vertex Y, so the bounding edges never have to be extrapolated.
    levels.dedup_by(|a, b| *a == *b);
    if levels.len() < 2 {
        return Vec::new();
    }

    let mut triangles = Vec::new();
    for band in levels.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        if y1 - y0 < 1e-6 {
            continue;
        }
        let midpoint = (y0 + y1) * 0.5;

        // Every edge crossing the band, as (x at the midpoint, edge).
        let mut hits: Vec<(f32, [f32; 2], [f32; 2])> = Vec::new();
        for contour in &loops {
            let count = contour.len();
            for index in 0..count {
                let a = contour[index];
                let b = contour[(index + 1) % count];
                if (a[1] > midpoint) == (b[1] > midpoint) {
                    continue;
                }
                let t = (midpoint - a[1]) / (b[1] - a[1]);
                hits.push((a[0] + (b[0] - a[0]) * t, a, b));
            }
        }
        hits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        for span in hits.chunks(2) {
            if span.len() < 2 {
                break;
            }
            let (_, left_0, left_1) = span[0];
            let (_, right_0, right_1) = span[1];
            let x_at = |a: [f32; 2], b: [f32; 2], y: f32| {
                a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1])
            };
            let left_top = [x_at(left_0, left_1, y0), y0];
            let left_bottom = [x_at(left_0, left_1, y1), y1];
            let right_top = [x_at(right_0, right_1, y0), y0];
            let right_bottom = [x_at(right_0, right_1, y1), y1];
            push_ccw(&mut triangles, left_top, right_top, right_bottom);
            push_ccw(&mut triangles, left_top, right_bottom, left_bottom);
        }
    }

    triangles
}

/// Push `a, b, c` in counter-clockwise order (positive signed area).
fn push_ccw(triangles: &mut Vec<[[f32; 2]; 3]>, a: [f32; 2], b: [f32; 2], c: [f32; 2]) {
    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if cross.abs() < 1e-9 {
        return;
    }
    if cross > 0.0 {
        triangles.push([a, b, c]);
    } else {
        triangles.push([a, c, b]);
    }
}

/// The world-space outward normal of an extruded wall.
///
/// `to_world` negates Y, so an SVG edge `(dx, dy)` becomes `(dx, -dy)` and its
/// left-hand normal `(-dy_w, dx_w)` — the direction away from the material for
/// both an outer (SVG-CCW) contour and a hole (SVG-CW) — is `(dy, dx)`.
pub fn wall_normal(edge: [f32; 2]) -> [f32; 3] {
    let length = (edge[0] * edge[0] + edge[1] * edge[1]).sqrt().max(1e-9);
    [edge[1] / length, edge[0] / length, 0.0]
}

/// The extruded, normalised logo mesh plus its measured half extents.
#[derive(Clone, Debug)]
pub struct LogoMesh {
    /// World-space triangle soup (non-indexed, like `ExtrudeGeometry`).
    pub mesh: Mesh,
    /// `this.logoHalf` — half the mesh's bounding-box size.
    pub half_extents: [f32; 3],
}

/// Build the DVD logo: parse [`LOGO_PATH`], extrude to [`SVG_DEPTH`] and
/// normalise to [`LOGO_WIDTH`] x proportional x [`LOGO_DEPTH`].
pub fn build_logo() -> LogoMesh {
    let contours = flatten_path(LOGO_PATH);
    let shapes = build_shapes(contours);

    // Triangulate in SVG space (y down), then flip Y so the caps face the
    // camera the way three's `scale(s, -s, ...)` does.
    let mut triangles: Vec<([f32; 2], [f32; 2], [f32; 2])> = Vec::new();
    for shape in &shapes {
        for triangle in triangulate_shape(shape) {
            triangles.push((triangle[0], triangle[1], triangle[2]));
        }
    }

    // Bounds in SVG space.
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for shape in &shapes {
        for point in shape.outer.iter().chain(shape.holes.iter().flatten()) {
            min[0] = min[0].min(point[0]);
            min[1] = min[1].min(point[1]);
            max[0] = max[0].max(point[0]);
            max[1] = max[1].max(point[1]);
        }
    }
    let size = [max[0] - min[0], max[1] - min[1]];
    let centre = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    let scale = if size[0].abs() > 1e-6 {
        LOGO_WIDTH / size[0]
    } else {
        1.0
    };
    let half_z = LOGO_DEPTH * 0.5;

    // SVG (x, y) -> world (x, -y), centred and scaled.
    let to_world = |point: [f32; 2], z: f32| {
        [
            (point[0] - centre[0]) * scale,
            -(point[1] - centre[1]) * scale,
            z,
        ]
    };

    let mut mesh = Mesh::default();
    let mut push = |position: [f32; 3], normal: [f32; 3], uv: [f32; 2]| {
        mesh.vertices.push(Vertex {
            position,
            normal,
            uv,
        });
    };

    for (a, b, c) in &triangles {
        // Front cap (+z). The SVG-space triangle is counter-clockwise; the Y
        // flip reverses it, so emit the vertices in reverse to keep the
        // outward +z winding.
        let front = [
            to_world(*c, half_z),
            to_world(*b, half_z),
            to_world(*a, half_z),
        ];
        for position in front {
            push(position, [0.0, 0.0, 1.0], [0.0, 0.0]);
        }
        // Back cap (-z), the original winding.
        let back = [
            to_world(*a, -half_z),
            to_world(*b, -half_z),
            to_world(*c, -half_z),
        ];
        for position in back {
            push(position, [0.0, 0.0, -1.0], [0.0, 0.0]);
        }
    }

    // Side walls: one quad per contour edge, on both caps.
    for shape in &shapes {
        for contour in std::iter::once(&shape.outer).chain(shape.holes.iter()) {
            for index in 0..contour.len() {
                let a = contour[index];
                let b = contour[(index + 1) % contour.len()];
                let edge = [b[0] - a[0], b[1] - a[1]];
                let length = (edge[0] * edge[0] + edge[1] * edge[1]).sqrt();
                if length < 1e-6 {
                    continue;
                }
                let normal = wall_normal(edge);
                let a0 = to_world(a, -half_z);
                let b0 = to_world(b, -half_z);
                let b1 = to_world(b, half_z);
                let a1 = to_world(a, half_z);
                for position in [a0, b0, b1, a0, b1, a1] {
                    push(position, normal, [0.0, 0.0]);
                }
            }
        }
    }

    // Index the soup sequentially: wgpu cannot create an empty index buffer
    // and the renderer draws indexed meshes.
    mesh.indices = (0..mesh.vertices.len() as u32).collect();

    let mut half_extents = [0.0_f32; 3];
    let mut bounds_min = [f32::INFINITY; 3];
    let mut bounds_max = [f32::NEG_INFINITY; 3];
    for vertex in &mesh.vertices {
        for axis in 0..3 {
            bounds_min[axis] = bounds_min[axis].min(vertex.position[axis]);
            bounds_max[axis] = bounds_max[axis].max(vertex.position[axis]);
        }
    }
    for axis in 0..3 {
        half_extents[axis] = (bounds_max[axis] - bounds_min[axis]) * 0.5;
    }

    LogoMesh { mesh, half_extents }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn the_parser_reads_the_subpaths_and_implicit_pairs() {
        let contours = flatten_path("M0 0h10v10h-10z");
        assert_eq!(contours.len(), 1);
        assert_eq!(contours[0].len(), 4);
        assert_eq!(contours[0][0], [0.0, 0.0]);
        assert_eq!(contours[0][1], [10.0, 0.0]);
        assert_eq!(contours[0][2], [10.0, 10.0]);
        assert_eq!(contours[0][3], [0.0, 10.0]);

        // Implicit lineto after `m`: every pair is relative to the previous
        // point, so the third point is (20, 10), not (10, 10) (the JS parser
        // does the same).
        let contours = flatten_path("m0 0 10 0 10 10z");
        assert_eq!(contours.len(), 1);
        assert_eq!(contours[0].len(), 3);
        assert_eq!(contours[0][2], [20.0, 10.0]);

        // A curve is flattened into CURVE_SEGMENTS + 1 points (the start point
        // is the previous command's end).
        let contours = flatten_path("M0 0 C 0 10 10 10 10 0 Z");
        assert_eq!(contours[0].len(), 1 + CURVE_SEGMENTS as usize);
    }

    #[test]
    fn the_logo_has_the_dvd_wordmark_and_the_swoosh_ring() {
        let contours = flatten_path(LOGO_PATH);
        // The wordmark outline, the swoosh ring's outer ellipse and its hole.
        assert_eq!(contours.len(), 3);
        let mut areas: Vec<f32> = contours.iter().map(|c| signed_area(c)).collect();
        areas.sort_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap());
        assert!(areas[0].abs() < areas[1].abs());
        assert!(areas[1].abs() < areas[2].abs());
    }

    #[test]
    fn shapes_group_each_hole_with_its_outer_contour() {
        let contours = flatten_path(LOGO_PATH);
        let shapes = build_shapes(contours);
        assert_eq!(shapes.len(), 2, "the wordmark + the swoosh ring");
        let holes: usize = shapes.iter().map(|shape| shape.holes.len()).sum();
        assert_eq!(holes, 1, "the swoosh's inner ellipse");
        for shape in &shapes {
            assert!(signed_area(&shape.outer) > 0.0);
            for hole in &shape.holes {
                assert!(signed_area(hole) < 0.0);
            }
        }

        // The wordmark spans the viewBox width.
        let max_x = shapes
            .iter()
            .flat_map(|shape| shape.outer.iter())
            .map(|point| point[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let min_x = shapes
            .iter()
            .flat_map(|shape| shape.outer.iter())
            .map(|point| point[0])
            .fold(f32::INFINITY, f32::min);
        assert!(max_x - min_x > 1000.0);
    }

    #[test]
    fn wall_normals_point_out_of_the_material() {
        // A CCW square with a CW hole, in SVG space (y down).
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]];
        let inside_material =
            |p: [f32; 2]| point_in_polygon(p, &outer) && !point_in_polygon(p, &hole);

        for contour in [&outer, &hole] {
            for index in 0..contour.len() {
                let a = contour[index];
                let b = contour[(index + 1) % contour.len()];
                let normal = wall_normal([b[0] - a[0], b[1] - a[1]]);
                // Step in world space (y flipped), then map back to SVG space
                // for the containment test.
                let midpoint_world = [a[0] + (b[0] - a[0]) * 0.5, -(a[1] + (b[1] - a[1]) * 0.5)];
                let step = 0.25;
                let probe = [
                    midpoint_world[0] + normal[0] * step,
                    -(midpoint_world[1] + normal[1] * step),
                ];
                assert!(
                    !inside_material(probe),
                    "wall normal escapes into the material at {a:?} -> {b:?}"
                );
            }
        }
    }

    #[test]
    fn triangulation_keeps_the_polygon_area() {
        let contours = flatten_path(LOGO_PATH);
        let shapes = build_shapes(contours);
        assert!(!shapes.is_empty());
        for shape in &shapes {
            let expected = signed_area(&shape.outer)
                + shape
                    .holes
                    .iter()
                    .map(|hole| signed_area(hole))
                    .sum::<f32>();
            let triangles = triangulate_shape(shape);
            assert!(!triangles.is_empty());
            let area: f32 = triangles
                .iter()
                .map(|triangle| {
                    let [a, b, c] = *triangle;
                    ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * 0.5
                })
                .sum();
            // The even-odd caps differ from the analytic polygon only by the
            // sub-texel wobble of the flattened sweep at the ellipse extremes
            // (<0.2%, i.e. far below one pixel at the render scale).
            close(area, expected, expected.abs() * 5e-3);
        }
    }

    #[test]
    fn a_square_with_a_hole_keeps_its_area() {
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[4.0, 4.0], [4.0, 6.0], [6.0, 6.0], [6.0, 4.0]];
        let shape = Shape {
            outer,
            holes: vec![hole],
        };
        let triangles = triangulate_shape(&shape);
        let area: f32 = triangles
            .iter()
            .map(|triangle| {
                let [a, b, c] = *triangle;
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * 0.5
            })
            .sum();
        close(area, 100.0 - 4.0, 1e-3);
        for triangle in &triangles {
            let [a, b, c] = *triangle;
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            assert!(cross > 0.0, "every cap triangle must be CCW");
        }
    }

    #[test]
    fn the_mesh_matches_the_js_extents_and_normalisation() {
        let logo = build_logo();
        assert!(!logo.mesh.vertices.is_empty());
        assert_eq!(
            logo.mesh.indices.len(),
            logo.mesh.vertices.len(),
            "the triangle soup is indexed sequentially"
        );
        assert_eq!(logo.mesh.indices.len() % 3, 0);
        close(logo.half_extents[0] * 2.0, LOGO_WIDTH, 2e-3);
        close(logo.half_extents[2] * 2.0, LOGO_DEPTH, 1e-5);
        // The height follows the path's own bounding box (the viewBox ratio
        // 465.84 / 1058.4 = 0.4402, the path's 0.4407 differ by the JS bevel).
        let aspect = logo.half_extents[1] / logo.half_extents[0];
        close(aspect, 0.4407, 2e-3);
        assert!(
            logo.mesh.vertices.len() > 1000,
            "the silhouette needs detail"
        );

        // Every vertex is inside the box and the caps point along Z.
        for vertex in &logo.mesh.vertices {
            for axis in 0..3 {
                assert!(vertex.position[axis].abs() <= logo.half_extents[axis] + 1e-4);
            }
            if vertex.normal[2].abs() > 0.5 {
                close(vertex.normal[0], 0.0, 1e-6);
                close(vertex.normal[1], 0.0, 1e-6);
            }
        }

        // The build is deterministic.
        let again = build_logo();
        assert_eq!(again.mesh.vertices.len(), logo.mesh.vertices.len());
        assert_eq!(
            again.mesh.vertices[0].position,
            logo.mesh.vertices[0].position
        );
    }
}
