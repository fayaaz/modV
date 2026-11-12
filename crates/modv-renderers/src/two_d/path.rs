//! CPU path builder, affine transforms and fill/stroke tessellation for the
//! 2D canvas facade.
//!
//! Everything here is pure geometry (no wgpu), so it is unit-testable in plain
//! Node-less Rust — see the tests in [`crate::two_d::tests`].
//!
//! # Space conventions
//!
//! Paths store **canvas-space** points: [`Canvas`](crate::two_d::canvas::Canvas)
//! transforms a point by the current transform when it is added, exactly like
//! canvas 2D. `fill`/`stroke` therefore need no transform of their own, and a
//! path survives a `save()`/`restore()` pair around the point calls
//! (`Polygon.js` relies on this).
//!
//! # Fill tessellation
//!
//! Each subpath is tessellated independently:
//!
//! * a subpath tagged as a full circle (set by `Canvas::arc` when a single
//!   zero-length subpath receives a full-turn arc) becomes a triangle fan from
//!   the transformed centre — exact for circles and cheap;
//! * every other subpath is ear-clipped. That handles the convex polygons
//!   `Polygon.js` makes and concave ones equally well.
//!
//! Non-zero winding *across* subpaths (holes) is not implemented: the ported
//! corpus never draws a path with a hole (`docs/rust-hybrid.md` WS-5 lists the
//! exact operations used — no `evenodd`, no compound fills).
//!
//! # Stroke expansion
//!
//! A stroke becomes one quad per segment plus a small fan at each interior
//! join. Caps are butt (the canvas default); joins are round rather than the
//! canvas default miter. The corpus only strokes 1-segment lines
//! (`Bar`, `Line`), polyline waveforms and circles, where miter vs. round is
//! not observable.

/// A 2D affine transform in canvas order (`x' = a x + c y + e`,
/// `y' = b x + d y + f`), matching the canvas `setTransform(a, b, c, d, e, f)`
/// component order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    /// x scale / rotation cosine term.
    pub a: f32,
    /// y shear / rotation sine term.
    pub b: f32,
    /// x shear term.
    pub c: f32,
    /// y scale term.
    pub d: f32,
    /// x translation.
    pub e: f32,
    /// y translation.
    pub f: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    /// The identity transform.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// A pure translation matrix.
    pub fn translate(x: f32, y: f32) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    /// A pure rotation matrix (radians, clockwise because canvas y grows down).
    pub fn rotate(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// Matrix product `self · rhs`: `rhs` is applied first, then `self`.
    ///
    /// Matches canvas' `CTM = CTM · newOperation`, i.e. the newly applied
    /// operation acts in the already-transformed space.
    pub fn multiply(&self, rhs: &Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }

    /// Transform a point.
    pub fn transform_point(&self, point: [f32; 2]) -> [f32; 2] {
        [
            self.a * point[0] + self.c * point[1] + self.e,
            self.b * point[0] + self.d * point[1] + self.f,
        ]
    }

    /// Uniform-ish scale factor, used to scale a circle radius (canvas scales
    /// an arc radius by the CTM). `sqrt(|det|)` is exact for uniform scale and
    /// a reasonable approximation for shears; the corpus only rotates and
    /// translates.
    pub fn scale_factor(&self) -> f32 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }
}

/// A circle in canvas space (also the analytic clip shape).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    /// Centre x.
    pub cx: f32,
    /// Centre y.
    pub cy: f32,
    /// Radius.
    pub r: f32,
}

/// A clip shape in canvas pixels.
///
/// `Rect` is the plan's stated minimum (the transformed path bounds);
/// `Circle` keeps `X-Drips.js`' circular `clip()` notches round instead of
/// squaring them off to the bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipShape {
    /// Axis-aligned rectangle.
    Rect {
        /// Left edge.
        x0: f32,
        /// Top edge.
        y0: f32,
        /// Right edge.
        x1: f32,
        /// Bottom edge.
        y1: f32,
    },
    /// Circle (analytic, evaluated in the fragment shader).
    Circle {
        /// Centre x.
        cx: f32,
        /// Centre y.
        cy: f32,
        /// Radius.
        r: f32,
    },
}

impl ClipShape {
    /// Axis-aligned bounds of the shape.
    pub fn bounds(&self) -> [f32; 4] {
        match *self {
            ClipShape::Rect { x0, y0, x1, y1 } => [x0, y0, x1, y1],
            ClipShape::Circle { cx, cy, r } => [cx - r, cy - r, cx + r, cy + r],
        }
    }
}

/// A path subpath: the flattened points plus the circle hint.
#[derive(Clone, Debug, Default)]
pub struct SubPath {
    /// Canvas-space points, in drawing order.
    pub points: Vec<[f32; 2]>,
    /// Whether `closePath` was called.
    pub closed: bool,
    /// Set when the whole subpath is a single full-turn arc; enables the exact
    /// fan fill and the analytic circle clip.
    pub circle: Option<Circle>,
}

impl SubPath {
    /// Add a point; adding a point after an arc invalidates the circle hint
    /// (the subpath is no longer a pure circle).
    pub fn push(&mut self, point: [f32; 2]) {
        self.points.push(point);
        self.circle = None;
    }

    /// The last point, if any.
    pub fn last(&self) -> Option<[f32; 2]> {
        self.points.last().copied()
    }
}

/// A canvas path: an ordered list of subpaths.
#[derive(Clone, Debug, Default)]
pub struct Path {
    /// The recorded subpaths.
    pub subpaths: Vec<SubPath>,
}

impl Path {
    /// Remove every subpath.
    pub fn clear(&mut self) {
        self.subpaths.clear();
    }

    /// Whether the path has no points at all.
    pub fn is_empty(&self) -> bool {
        self.subpaths.iter().all(|sp| sp.points.is_empty())
    }

    /// Start a new subpath at `point`.
    pub fn move_to(&mut self, point: [f32; 2]) {
        self.subpaths.push(SubPath {
            points: vec![point],
            closed: false,
            circle: None,
        });
    }

    /// Append a point to the current subpath (starting one if there is none).
    pub fn line_to(&mut self, point: [f32; 2]) {
        match self.subpaths.last_mut() {
            Some(subpath) => subpath.push(point),
            None => self.move_to(point),
        }
    }

    /// The current point (`None` before any `moveTo`).
    pub fn current_point(&self) -> Option<[f32; 2]> {
        self.subpaths.last().and_then(SubPath::last)
    }

    /// Mark the current subpath closed.
    pub fn close(&mut self) {
        if let Some(subpath) = self.subpaths.last_mut() {
            subpath.closed = true;
        }
    }

    /// Whether the last subpath has no points yet (used by `arc` to decide
    /// whether it starts a fresh subpath and may claim the circle hint).
    pub fn last_subpath_len(&self) -> usize {
        self.subpaths.last().map_or(0, |sp| sp.points.len())
    }

    /// Set the circle hint on the last subpath.
    pub fn set_last_circle(&mut self, circle: Circle) {
        if let Some(subpath) = self.subpaths.last_mut() {
            subpath.circle = Some(circle);
        }
    }

    /// Bounding box of every point (`[x0, y0, x1, y1]`), if non-empty.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let mut bounds: Option<[f32; 4]> = None;
        for point in self.subpaths.iter().flat_map(|sp| sp.points.iter()) {
            bounds = Some(match bounds {
                None => [point[0], point[1], point[0], point[1]],
                Some([x0, y0, x1, y1]) => [
                    x0.min(point[0]),
                    y0.min(point[1]),
                    x1.max(point[0]),
                    y1.max(point[1]),
                ],
            });
        }
        bounds
    }

    /// The single circle this path represents, if it is exactly one full-turn
    /// arc — the analytic clip case.
    pub fn sole_circle(&self) -> Option<Circle> {
        match self.subpaths.as_slice() {
            [subpath] => subpath.circle,
            _ => None,
        }
    }
}

/// Flatten an arc into points, in the supplied (local) coordinate space.
///
/// * `start`/`end` are radians; `anticlockwise` mirrors canvas' `ccw` flag.
/// * 32 segments per full turn, scaled by the swept angle and clamped to at
///   least one segment. See the module docs for why a fixed count.
///
/// Returns the points *excluding* the start point (the caller decides whether
/// the arc connects to the current point with a `lineTo`).
pub fn flatten_arc(
    cx: f32,
    cy: f32,
    r: f32,
    start: f32,
    end: f32,
    anticlockwise: bool,
) -> Vec<[f32; 2]> {
    let sweep = arc_sweep(start, end, anticlockwise);
    let segments = ((sweep.abs() / std::f32::consts::TAU) * 32.0)
        .ceil()
        .max(1.0) as usize;
    let mut points = Vec::with_capacity(segments);
    for index in 1..=segments {
        let angle = start + sweep * (index as f32 / segments as f32);
        points.push([cx + r * angle.cos(), cy + r * angle.sin()]);
    }
    points
}

/// The signed sweep canvas uses for `arc(x, y, r, start, end, ccw)`.
pub fn arc_sweep(start: f32, end: f32, anticlockwise: bool) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut delta = (end - start) % tau;
    if anticlockwise {
        if delta > 0.0 {
            delta -= tau;
        }
        if delta == 0.0 {
            delta = -tau;
        }
    } else {
        if delta < 0.0 {
            delta += tau;
        }
        if delta == 0.0 {
            delta = tau;
        }
    }
    delta
}

/// Whether an arc covers a full turn (used for the circle hint/fan fill).
pub fn arc_is_full_turn(start: f32, end: f32, anticlockwise: bool) -> bool {
    arc_sweep(start, end, anticlockwise).abs() >= std::f32::consts::TAU - 1e-3
}

/// Tessellate a subpath into triangles, returning three points per triangle.
pub fn triangulate_subpath(subpath: &SubPath) -> Vec<[f32; 2]> {
    let points = dedupe(&subpath.points);
    if points.len() < 3 {
        return Vec::new();
    }
    if let Some(circle) = subpath.circle {
        let mut out = Vec::with_capacity(points.len() * 3);
        for index in 0..points.len() {
            let a = points[index];
            let b = points[(index + 1) % points.len()];
            out.push([circle.cx, circle.cy]);
            out.push(a);
            out.push(b);
        }
        return out;
    }
    ear_clip(&points)
}

/// Remove consecutive duplicate points (including the closing duplicate).
fn dedupe(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(points.len());
    for &point in points {
        match out.last() {
            Some(last) if close(*last, point) => {}
            _ => out.push(point),
        }
    }
    if out.len() > 1 && close(out[0], *out.last().expect("non-empty")) {
        out.pop();
    }
    out
}

fn close(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5
}

fn cross(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Twice the signed area (positive = counter-clockwise in a y-down space
/// means "clockwise on screen"; the sign is only used consistently).
pub fn signed_area(points: &[[f32; 2]]) -> f32 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    area
}

/// Ear-clip a simple polygon. Falls back to a triangle fan if clipping stalls
/// (self-intersecting input), so a fill always produces *something*.
pub fn ear_clip(points: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let count = points.len();
    if count < 3 {
        return Vec::new();
    }
    if count == 3 {
        return points.to_vec();
    }

    let area = signed_area(points);
    if area.abs() < 1e-6 {
        // Degenerate (collinear) polygon: nothing to fill.
        return Vec::new();
    }
    let counter_clockwise = area > 0.0;

    let mut indices: Vec<usize> = (0..count).collect();
    let mut out = Vec::with_capacity((count - 2) * 3);
    let mut guard = 0usize;
    let max_steps = count * count;

    while indices.len() > 3 && guard < max_steps {
        guard += 1;
        let len = indices.len();
        let mut clipped = false;
        for position in 0..len {
            let prev = indices[(position + len - 1) % len];
            let current = indices[position];
            let next = indices[(position + 1) % len];
            let (a, b, c) = (points[prev], points[current], points[next]);

            let turn = cross(a, b, c);
            let convex = if counter_clockwise {
                turn > 1e-6
            } else {
                turn < -1e-6
            };
            if !convex {
                continue;
            }
            let mut contains_other = false;
            for &candidate in &indices {
                if candidate == prev || candidate == current || candidate == next {
                    continue;
                }
                if point_in_triangle(points[candidate], a, b, c) {
                    contains_other = true;
                    break;
                }
            }
            if contains_other {
                continue;
            }
            out.extend_from_slice(&[a, b, c]);
            indices.remove(position);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }

    if indices.len() > 2 {
        // Fan the remainder (either the final ear or a stalled self-intersecting
        // polygon). Keeps the fill non-empty rather than dropping it.
        let first = indices[0];
        for window in indices[1..].windows(2) {
            out.extend_from_slice(&[points[first], points[window[0]], points[window[1]]]);
        }
    }
    out
}

/// Inclusive point-in-triangle test (a point on an edge counts as inside).
fn point_in_triangle(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    let has_negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_negative && has_positive)
}

/// Expand a polyline into stroke triangles (three points per triangle).
///
/// `half_width` is `lineWidth / 2` in canvas pixels. Segments are quads;
/// interior vertices get a round join disc, which also covers the caps of a
/// closed subpath.
pub fn stroke_geometry(points: &[[f32; 2]], closed: bool, half_width: f32) -> Vec<[f32; 2]> {
    let points = dedupe(points);
    let mut out = Vec::new();
    if points.len() < 2 || !(half_width > 0.0) {
        return out;
    }
    let width = half_width.max(0.01);
    let count = points.len();
    let segments = if closed { count } else { count - 1 };

    for index in 0..segments {
        let a = points[index];
        let b = points[(index + 1) % count];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = (dx * dx + dy * dy).sqrt();
        if length < 1e-6 {
            continue;
        }
        let nx = -dy / length * width;
        let ny = dx / length * width;
        let p0 = [a[0] + nx, a[1] + ny];
        let p1 = [b[0] + nx, b[1] + ny];
        let p2 = [b[0] - nx, b[1] - ny];
        let p3 = [a[0] - nx, a[1] - ny];
        out.extend_from_slice(&[p0, p1, p2, p0, p2, p3]);
    }

    // Interior joins. Thin strokes (1 px) do not need them and the extra
    // geometry would only darken the line.
    if width > 0.75 {
        let first_join = if closed { 0 } else { 1 };
        let last_join = if closed { count } else { count - 1 };
        for index in first_join..last_join {
            let point = points[index % count];
            disc(&mut out, point, width, 10);
        }
    }
    out
}

/// Append a triangle fan disc.
pub fn disc(out: &mut Vec<[f32; 2]>, centre: [f32; 2], radius: f32, segments: usize) {
    let segments = segments.max(3);
    for index in 0..segments {
        let a = index as f32 / segments as f32 * std::f32::consts::TAU;
        let b = (index + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        out.push(centre);
        out.push([centre[0] + radius * a.cos(), centre[1] + radius * a.sin()]);
        out.push([centre[0] + radius * b.cos(), centre[1] + radius * b.sin()]);
    }
}

/// Transform the four corners of a rectangle and emit two triangles.
pub fn rect_geometry(x: f32, y: f32, width: f32, height: f32, transform: &Affine) -> Vec<[f32; 2]> {
    if !(width > 0.0) || !(height > 0.0) {
        return Vec::new();
    }
    let p0 = transform.transform_point([x, y]);
    let p1 = transform.transform_point([x + width, y]);
    let p2 = transform.transform_point([x + width, y + height]);
    let p3 = transform.transform_point([x, y + height]);
    vec![p0, p1, p2, p0, p2, p3]
}

/// Transform the axis-aligned bounding box of a rectangle.
pub fn transformed_bounds(x: f32, y: f32, width: f32, height: f32, transform: &Affine) -> [f32; 4] {
    let corners = [
        transform.transform_point([x, y]),
        transform.transform_point([x + width, y]),
        transform.transform_point([x + width, y + height]),
        transform.transform_point([x, y + height]),
    ];
    let mut bounds = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for point in corners {
        bounds[0] = bounds[0].min(point[0]);
        bounds[1] = bounds[1].min(point[1]);
        bounds[2] = bounds[2].max(point[0]);
        bounds[3] = bounds[3].max(point[1]);
    }
    bounds
}
