//! An interactive 3D model of the rider's moped, modelled in code and drawn by a small software renderer. The side
//! outlines are traced from a side photo at 470 pixels to the metre, and the widths come from front and three-quarter
//! photos. Metres: x forward, y up, z to the rider's right.

use std::sync::OnceLock;

type V3 = [f32; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let len = dot(a, a).sqrt();
    if len > 1e-6 {
        scale(a, 1.0 / len)
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// A point in the side photo, in metres on the moped.
fn px(x: f32, y: f32) -> (f32, f32) {
    ((525.0 - x) / 470.0, (661.0 - y) / 470.0)
}
fn at(x: f32, y: f32, z: f32) -> V3 {
    let p = px(x, y);
    [p.0, p.1, z]
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mat {
    Body,
    Trim,
    Seat,
    Tyre,
    Rim,
    Metal,
    Silver,
    Dark,
    Head,
    Lens,
    Glass,
    Tail,
    Signal,
    Screen,
    Cavity,
    BatteryCase,
    /// One charge light: which battery, and which of the five.
    Pip(usize, u8),
}

struct Part {
    mat: Mat,
    on_seat: bool,
    pos: Vec<V3>,
    nor: Vec<V3>,
    tris: Vec<[u32; 3]>,
}

fn normals(pos: &[V3], tris: &[[u32; 3]]) -> Vec<V3> {
    let mut out = vec![[0.0f32; 3]; pos.len()];
    for t in tris {
        let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
        let n = cross(sub(b, a), sub(c, a));
        for &i in t {
            out[i as usize] = add(out[i as usize], n);
        }
    }
    out.iter().map(|n| norm(*n)).collect()
}

impl Part {
    /// Smooth normals, with the winding turned outward so that back faces can be skipped.
    fn new(mat: Mat, pos: Vec<V3>, tris: Vec<[u32; 3]>) -> Self {
        let mut part = Self { mat, on_seat: false, nor: normals(&pos, &tris), pos, tris };
        let count = part.pos.len().max(1) as f32;
        let centre = scale(part.pos.iter().fold([0.0; 3], |sum, p| add(sum, *p)), 1.0 / count);
        let outward: f32 = part.pos.iter().zip(&part.nor).map(|(p, n)| dot(sub(*p, centre), *n)).sum();
        if outward < 0.0 {
            part.flip();
        }
        part
    }

    /// For a thin sheet with no inside: it faces the way it's told to.
    fn facing(mat: Mat, pos: Vec<V3>, tris: Vec<[u32; 3]>, toward: V3) -> Self {
        let mut part = Self { mat, on_seat: false, nor: normals(&pos, &tris), pos, tris };
        let sum = part.nor.iter().fold([0.0; 3], |sum, n| add(sum, *n));
        if dot(sum, toward) < 0.0 {
            part.flip();
        }
        part
    }

    fn flip(&mut self) {
        for t in &mut self.tris {
            t.swap(1, 2);
        }
        self.nor = normals(&self.pos, &self.tris);
    }
}

/// One slice of a body. In the side view it runs from `front` to `back`; `w` is half its width and `cz` where its
/// middle sits sideways. `p` near 2 makes it an ellipse, and higher values a soft-cornered box.
#[derive(Clone, Copy)]
struct Slice {
    front: (f32, f32),
    back: (f32, f32),
    w: f32,
    p: f32,
    cz: f32,
}

/// A slice between two points of the side photo.
fn slice(front: (f32, f32), back: (f32, f32), w: f32, p: f32) -> Slice {
    Slice { front: px(front.0, front.1), back: px(back.0, back.1), w, p, cz: 0.0 }
}

fn shifted(slices: &[Slice], cz: f32) -> Vec<Slice> {
    slices.iter().map(|s| Slice { cz, ..*s }).collect()
}

/// A smooth curve through the slices, so that traced outlines don't show their corners.
fn slice_at(slices: &[Slice], t: f32) -> Slice {
    let last = slices.len() - 1;
    let along = t.clamp(0.0, 1.0) * last as f32;
    let i = (along.floor() as usize).min(last.saturating_sub(1));
    let f = along - i as f32;
    let get = |k: isize| slices[k.clamp(0, last as isize) as usize];
    let (a, b, c, d) = (get(i as isize - 1), get(i as isize), get(i as isize + 1), get(i as isize + 2));
    let curve = |a: f32, b: f32, c: f32, d: f32| {
        0.5 * (2.0 * b + (c - a) * f + (2.0 * a - 5.0 * b + 4.0 * c - d) * f * f + (3.0 * b - a - 3.0 * c + d) * f * f * f)
    };
    Slice {
        front: (curve(a.front.0, b.front.0, c.front.0, d.front.0), curve(a.front.1, b.front.1, c.front.1, d.front.1)),
        back: (curve(a.back.0, b.back.0, c.back.0, d.back.0), curve(a.back.1, b.back.1, c.back.1, d.back.1)),
        w: curve(a.w, b.w, c.w, d.w).max(0.002),
        p: b.p + (c.p - b.p) * f,
        cz: b.cz + (c.cz - b.cz) * f,
    }
}

const STEPS_PER_SLICE: usize = 5;
const AROUND: usize = 28;

fn loft(mat: Mat, slices: &[Slice]) -> Part {
    let n = (slices.len() - 1) * STEPS_PER_SLICE + 1;
    let mut pos = Vec::with_capacity((n + 2) * AROUND + 2);
    let mut tris = Vec::new();
    let ring_of = |s: &Slice| -> Vec<V3> {
        let mid = [(s.front.0 + s.back.0) / 2.0, (s.front.1 + s.back.1) / 2.0];
        let up = [(s.front.0 - s.back.0) / 2.0, (s.front.1 - s.back.1) / 2.0];
        (0..AROUND)
            .map(|j| {
                let angle = j as f32 / AROUND as f32 * std::f32::consts::TAU;
                let shape = |v: f32| v.signum() * v.abs().powf(2.0 / s.p);
                let (sz, su) = (shape(angle.cos()), shape(angle.sin()));
                [mid[0] + up[0] * su, mid[1] + up[1] * su, s.cz + s.w * sz]
            })
            .collect()
    };
    for i in 0..n {
        pos.extend(ring_of(&slice_at(slices, i as f32 / (n - 1) as f32)));
    }
    let ring = |i: usize, j: usize| (i * AROUND + j % AROUND) as u32;
    for i in 0..n - 1 {
        for j in 0..AROUND {
            tris.push([ring(i, j), ring(i + 1, j), ring(i + 1, j + 1)]);
            tris.push([ring(i, j), ring(i + 1, j + 1), ring(i, j + 1)]);
        }
    }
    // The ends get their own corners, so that they stay crisp and flat.
    for source in [0usize, n - 1] {
        let copied: Vec<V3> = pos[source * AROUND..(source + 1) * AROUND].to_vec();
        pos.extend(copied);
    }
    for (end, flip) in [(n, true), (n + 1, false)] {
        let centre = pos.len() as u32;
        let sum = (0..AROUND).fold([0.0; 3], |sum, j| add(sum, pos[ring(end, j) as usize]));
        pos.push(scale(sum, 1.0 / AROUND as f32));
        for j in 0..AROUND {
            let (a, b) = (ring(end, j), ring(end, j + 1));
            tris.push(if flip { [centre, b, a] } else { [centre, a, b] });
        }
    }
    Part::new(mat, pos, tris)
}

/// A place on the surface of a body: `t` along its slices, `z` sideways, on the `front` side of the slice.
fn on_hull(slices: &[Slice], t: f32, z: f32, lift: f32) -> V3 {
    let s = slice_at(slices, t);
    let rel = ((z - s.cz) / s.w).abs().min(1.0);
    let su = (1.0 - rel.powf(s.p)).max(0.0).powf(1.0 / s.p);
    let mid = [(s.front.0 + s.back.0) / 2.0, (s.front.1 + s.back.1) / 2.0];
    let up = [(s.front.0 - s.back.0) / 2.0, (s.front.1 - s.back.1) / 2.0];
    let length = (up[0] * up[0] + up[1] * up[1]).sqrt().max(1e-4);
    let grow = su + lift / length;
    [mid[0] + up[0] * grow, mid[1] + up[1] * grow, z]
}

/// A band painted along a body, such as the stripe.
fn hull_band(mat: Mat, slices: &[Slice], t: (f32, f32), half: (f32, f32), lift: f32) -> Part {
    const LONG: usize = 26;
    const WIDE: usize = 6;
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    for i in 0..=LONG {
        let f = i as f32 / LONG as f32;
        let tt = t.0 + (t.1 - t.0) * f;
        let w = half.0 + (half.1 - half.0) * f;
        for j in 0..=WIDE {
            pos.push(on_hull(slices, tt, w * (j as f32 / WIDE as f32 * 2.0 - 1.0), lift));
        }
    }
    let idx = |i: usize, j: usize| (i * (WIDE + 1) + j) as u32;
    for i in 0..LONG {
        for j in 0..WIDE {
            tris.push([idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            tris.push([idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    let middle = slice_at(slices, (t.0 + t.1) / 2.0);
    let toward = [middle.front.0 - middle.back.0, middle.front.1 - middle.back.1, 0.0];
    Part::facing(mat, pos, tris, toward)
}

/// A soft-cornered shape sitting on a body, such as a headlight. `inner` above 0 leaves a hole, for a ring of light.
fn hull_badge(mat: Mat, slices: &[Slice], centre: (f32, f32), half: (f32, f32), inner: f32, lift: f32) -> Part {
    const AROUND_BADGE: usize = 32;
    const RINGS: usize = 4;
    const ROUND: f32 = 3.6;
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    for r in 0..=RINGS {
        let s = inner + (1.0 - inner) * r as f32 / RINGS as f32;
        for j in 0..AROUND_BADGE {
            let a = j as f32 / AROUND_BADGE as f32 * std::f32::consts::TAU;
            let shape = |v: f32| v.signum() * v.abs().powf(2.0 / ROUND);
            let (sx, sy) = (shape(a.cos()), shape(a.sin()));
            pos.push(on_hull(slices, centre.0 + half.0 * s * sy, centre.1 + half.1 * s * sx, lift));
        }
    }
    let idx = |r: usize, j: usize| (r * AROUND_BADGE + j % AROUND_BADGE) as u32;
    for r in 0..RINGS {
        for j in 0..AROUND_BADGE {
            tris.push([idx(r, j), idx(r + 1, j), idx(r + 1, j + 1)]);
            tris.push([idx(r, j), idx(r + 1, j + 1), idx(r, j + 1)]);
        }
    }
    if inner <= 0.0 {
        let middle = pos.len() as u32;
        pos.push(on_hull(slices, centre.0, centre.1, lift));
        for j in 0..AROUND_BADGE {
            tris.push([middle, idx(0, j), idx(0, j + 1)]);
        }
    }
    let middle = slice_at(slices, centre.0);
    let toward = [middle.front.0 - middle.back.0, middle.front.1 - middle.back.1, 0.0];
    Part::facing(mat, pos, tris, toward)
}

/// A body built sideways: each slice is across z, for the handlebars' cowl. (z, x, y, half x, half y, roundness).
fn crosswise(mat: Mat, slices: &[(f32, f32, f32, f32, f32, f32)]) -> Part {
    let n = slices.len();
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    let ring = |i: usize, j: usize| (i * AROUND + j % AROUND) as u32;
    for &(z, x, y, rx, ry, p) in slices {
        for j in 0..AROUND {
            let a = j as f32 / AROUND as f32 * std::f32::consts::TAU;
            let shape = |v: f32| v.signum() * v.abs().powf(2.0 / p);
            pos.push([x + rx * shape(a.cos()), y + ry * shape(a.sin()), z]);
        }
    }
    for i in 0..n - 1 {
        for j in 0..AROUND {
            tris.push([ring(i, j), ring(i + 1, j), ring(i + 1, j + 1)]);
            tris.push([ring(i, j), ring(i + 1, j + 1), ring(i, j + 1)]);
        }
    }
    for (end, flip) in [(0usize, true), (n - 1, false)] {
        let centre = pos.len() as u32;
        pos.push([slices[end].1, slices[end].2, slices[end].0]);
        for j in 0..AROUND {
            let (a, b) = (ring(end, j), ring(end, j + 1));
            tris.push(if flip { [centre, b, a] } else { [centre, a, b] });
        }
    }
    Part::new(mat, pos, tris)
}

/// Spins a profile of (radius, sideways offset) around the z axis, for wheels. `closed` joins the ends.
fn lathe(mat: Mat, centre: V3, profile: &[(f32, f32)], closed: bool) -> Part {
    const AROUND_WHEEL: usize = 40;
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    for &(r, dz) in profile {
        for j in 0..AROUND_WHEEL {
            let a = j as f32 / AROUND_WHEEL as f32 * std::f32::consts::TAU;
            pos.push([centre[0] + r * a.cos(), centre[1] + r * a.sin(), centre[2] + dz]);
        }
    }
    let n = profile.len();
    let spans = if closed { n } else { n - 1 };
    let idx = |i: usize, j: usize| ((i % n) * AROUND_WHEEL + j % AROUND_WHEEL) as u32;
    for i in 0..spans {
        for j in 0..AROUND_WHEEL {
            tris.push([idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            tris.push([idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    Part::new(mat, pos, tris)
}

fn tube(mat: Mat, a: V3, b: V3, radius: f32) -> Part {
    const AROUND_TUBE: usize = 10;
    let axis = norm(sub(b, a));
    let side = norm(cross(axis, if axis[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }));
    let other = cross(axis, side);
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    for end in [a, b] {
        for j in 0..AROUND_TUBE {
            let t = j as f32 / AROUND_TUBE as f32 * std::f32::consts::TAU;
            pos.push(add(end, add(scale(side, radius * t.cos()), scale(other, radius * t.sin()))));
        }
    }
    let (ca, cb) = (pos.len() as u32, pos.len() as u32 + 1);
    pos.push(a);
    pos.push(b);
    for j in 0..AROUND_TUBE {
        let (j0, j1) = (j as u32, ((j + 1) % AROUND_TUBE) as u32);
        let k = AROUND_TUBE as u32;
        tris.push([j0, k + j0, k + j1]);
        tris.push([j0, k + j1, j1]);
        tris.push([ca, j1, j0]);
        tris.push([cb, k + j0, k + j1]);
    }
    Part::new(mat, pos, tris)
}

fn blob(mat: Mat, c: V3, r: V3) -> Part {
    const LAT: usize = 10;
    const LON: usize = 16;
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    for i in 0..=LAT {
        let v = i as f32 / LAT as f32 * std::f32::consts::PI;
        for j in 0..LON {
            let u = j as f32 / LON as f32 * std::f32::consts::TAU;
            pos.push([c[0] + r[0] * v.sin() * u.cos(), c[1] + r[1] * v.cos(), c[2] + r[2] * v.sin() * u.sin()]);
        }
    }
    let idx = |i: usize, j: usize| (i * LON + j % LON) as u32;
    for i in 0..LAT {
        for j in 0..LON {
            tris.push([idx(i, j), idx(i + 1, j), idx(i + 1, j + 1)]);
            tris.push([idx(i, j), idx(i + 1, j + 1), idx(i, j + 1)]);
        }
    }
    Part::new(mat, pos, tris)
}

fn solid(mat: Mat, min: V3, max: V3) -> Part {
    let p = |x: f32, y: f32, z: f32| [x, y, z];
    let (x0, x1) = (min[0].min(max[0]), min[0].max(max[0]));
    let (y0, y1) = (min[1].min(max[1]), min[1].max(max[1]));
    let (z0, z1) = (min[2].min(max[2]), min[2].max(max[2]));
    let pos = [p(x0, y0, z0), p(x1, y0, z0), p(x1, y1, z0), p(x0, y1, z0), p(x0, y0, z1), p(x1, y0, z1), p(x1, y1, z1), p(x0, y1, z1)];
    // Each face gets its own corners, so the edges stay crisp.
    let faces = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [3, 7, 6, 2], [0, 4, 7, 3], [1, 2, 6, 5]];
    let mut split = Vec::new();
    let mut tris = Vec::new();
    for face in faces {
        let base = split.len() as u32;
        split.extend(face.iter().map(|&i: &usize| pos[i]));
        tris.push([base, base + 1, base + 2]);
        tris.push([base, base + 2, base + 3]);
    }
    Part::new(mat, split, tris)
}

const RIM_RADIUS: f32 = 0.168;

fn wheel(parts: &mut Vec<Part>, x: f32, radius: f32, half_width: f32) {
    let c = [x, radius, 0.0];
    let tyre: Vec<(f32, f32)> = (0..16)
        .map(|i| {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            let middle = RIM_RADIUS + 0.012;
            (middle + (radius - middle) * a.cos(), half_width * a.sin())
        })
        .collect();
    parts.push(lathe(Mat::Tyre, c, &tyre, true));
    let w = half_width * 0.8;
    let rim = [(RIM_RADIUS + 0.008, -w), (RIM_RADIUS + 0.008, w), (RIM_RADIUS - 0.012, w * 0.9), (RIM_RADIUS - 0.02, 0.0), (RIM_RADIUS - 0.012, -w * 0.9)];
    parts.push(lathe(Mat::Rim, c, &rim, true));
    // Five Y-shaped spokes.
    for i in 0..5 {
        let base = i as f32 / 5.0 * std::f32::consts::TAU + 0.3;
        let point = |angle: f32, r: f32| [x + r * angle.cos(), radius + r * angle.sin(), 0.0];
        parts.push(tube(Mat::Rim, point(base, 0.03), point(base, 0.09), 0.0085));
        parts.push(tube(Mat::Rim, point(base, 0.09), point(base + 0.3, RIM_RADIUS - 0.012), 0.0065));
        parts.push(tube(Mat::Rim, point(base, 0.09), point(base - 0.3, RIM_RADIUS - 0.012), 0.0065));
    }
}

fn build() -> Vec<Part> {
    let mut parts = Vec::new();
    let (front_r, rear_r) = (0.238, 0.256);
    let (front_x, rear_x) = (px(210.0, 0.0).0, px(840.0, 0.0).0);
    wheel(&mut parts, front_x, front_r, 0.055);
    wheel(&mut parts, rear_x, rear_r, 0.066);

    // The front wheel's hub and brake disc.
    let front_c = [front_x, front_r, 0.0];
    parts.push(lathe(Mat::Metal, front_c, &[(0.0, -0.05), (0.032, -0.05), (0.036, 0.0), (0.032, 0.05), (0.0, 0.05)], false));
    parts.push(lathe(Mat::Silver, front_c, &[(0.07, 0.044), (0.118, 0.044), (0.118, 0.049), (0.07, 0.049)], true));

    // The rear wheel holds the motor. Its round cover is on the rider's right, and on the left is the silver arm.
    let rear_c = [rear_x, rear_r, 0.0];
    parts.push(lathe(Mat::Silver, rear_c, &[(0.0, 0.092), (0.075, 0.09), (0.12, 0.075), (0.14, 0.045), (0.14, 0.0)], false));
    parts.push(lathe(Mat::Dark, rear_c, &[(0.0, 0.094), (0.022, 0.094), (0.024, 0.088)], false));
    parts.push(lathe(Mat::Metal, rear_c, &[(0.0, -0.06), (0.09, -0.06), (0.125, -0.04), (0.125, 0.0)], false));
    parts.push(lathe(Mat::Silver, rear_c, &[(0.098, -0.066), (0.118, -0.066), (0.122, -0.05), (0.094, -0.05)], true));
    parts.push(loft(
        Mat::Silver,
        &shifted(
            &[
                slice((640.0, 520.0), (640.0, 548.0), 0.02, 3.0),
                slice((660.0, 513.0), (655.0, 556.0), 0.028, 3.0),
                slice((730.0, 506.0), (725.0, 566.0), 0.03, 3.0),
                slice((795.0, 502.0), (795.0, 575.0), 0.03, 3.0),
                slice((850.0, 515.0), (850.0, 572.0), 0.028, 3.0),
                slice((875.0, 528.0), (875.0, 552.0), 0.02, 3.0),
            ],
            -0.095,
        ),
    ));
    parts.push(tube(Mat::Dark, at(840.0, 536.0, 0.098), at(650.0, 536.0, 0.11), 0.016));

    // The single rear shock, on the left: a rod and the turns of its spring.
    let (shock_a, shock_b) = (at(815.0, 385.0, -0.092), at(855.0, 515.0, -0.092));
    parts.push(tube(Mat::Metal, shock_a, shock_b, 0.009));
    for i in 0..9 {
        let f = 0.08 + i as f32 / 9.0 * 0.62;
        let c = add(shock_a, scale(sub(shock_b, shock_a), f));
        let axis = scale(norm(sub(shock_b, shock_a)), 0.006);
        parts.push(tube(Mat::Dark, sub(c, axis), add(c, axis), 0.027));
    }

    // Under the feet.
    parts.push(loft(
        Mat::Dark,
        &[
            slice((395.0, 512.0), (398.0, 587.0), 0.17, 4.0),
            slice((470.0, 497.0), (470.0, 587.0), 0.185, 4.5),
            slice((530.0, 494.0), (530.0, 586.0), 0.185, 4.5),
            slice((590.0, 520.0), (590.0, 584.0), 0.17, 4.0),
            slice((630.0, 555.0), (630.0, 582.0), 0.12, 3.0),
        ],
    ));
    // The mat on top of the floor.
    parts.push(solid(Mat::Trim, at(400.0, 503.0, -0.14), at(525.0, 490.0, 0.14)));

    // The dark inside of the leg shield, up to the steering neck.
    parts.push(loft(
        Mat::Dark,
        &[
            slice((345.0, 212.0), (402.0, 232.0), 0.075, 2.8),
            slice((335.0, 290.0), (382.0, 285.0), 0.15, 3.2),
            slice((300.0, 345.0), (355.0, 340.0), 0.185, 3.6),
            slice((305.0, 395.0), (368.0, 400.0), 0.17, 3.6),
            slice((332.0, 445.0), (385.0, 455.0), 0.155, 3.6),
            slice((355.0, 500.0), (402.0, 500.0), 0.15, 3.6),
            slice((395.0, 548.0), (435.0, 535.0), 0.15, 3.6),
        ],
    ));

    // The head of the front, wide and rounded, and the top of it narrower.
    let head = [
        slice((326.0, 209.0), (356.0, 217.0), 0.07, 2.4),
        slice((300.0, 234.0), (356.0, 252.0), 0.112, 2.7),
        slice((266.0, 260.0), (350.0, 270.0), 0.15, 3.0),
        slice((245.0, 282.0), (340.0, 290.0), 0.178, 3.2),
        slice((233.0, 315.0), (320.0, 316.0), 0.196, 3.4),
        slice((226.0, 346.0), (294.0, 350.0), 0.2, 3.6),
        slice((233.0, 370.0), (297.0, 372.0), 0.195, 3.6),
        slice((262.0, 405.0), (301.0, 396.0), 0.17, 3.2),
    ];
    parts.push(loft(Mat::Body, &head));

    // Below the head the front splits into two legs, with the fork and the wheel between them.
    let leg = [
        slice((248.0, 368.0), (298.0, 372.0), 0.05, 3.2),
        slice((264.0, 405.0), (302.0, 395.0), 0.05, 3.2),
        slice((292.0, 436.0), (326.0, 441.0), 0.045, 3.2),
        slice((306.0, 470.0), (340.0, 478.0), 0.045, 3.2),
        slice((326.0, 510.0), (362.0, 522.0), 0.045, 3.2),
        slice((352.0, 560.0), (394.0, 553.0), 0.05, 3.2),
        slice((382.0, 582.0), (422.0, 585.0), 0.05, 3.2),
    ];
    for side in [-1.0f32, 1.0] {
        parts.push(loft(Mat::Body, &shifted(&leg, side * 0.15)));
    }
    // And the two legs meet again under the floor's front edge.
    parts.push(loft(
        Mat::Body,
        &[
            slice((360.0, 562.0), (400.0, 553.0), 0.198, 4.0),
            slice((382.0, 582.0), (424.0, 586.0), 0.198, 4.0),
            slice((410.0, 590.0), (440.0, 589.0), 0.17, 3.5),
        ],
    ));

    // The stripe down the middle, and the glass panel with its two lights.
    parts.push(hull_band(Mat::Trim, &head, (0.0, 1.0), (0.043, 0.036), 0.0015));
    let lamp_t = 0.635;
    parts.push(hull_badge(Mat::Glass, &head, (lamp_t, 0.0), (0.14, 0.128), 0.0, 0.003));
    for side in [-1.0f32, 1.0] {
        let z = side * 0.066;
        parts.push(hull_badge(Mat::Head, &head, (lamp_t, z), (0.122, 0.054), 0.68, 0.0055));
        parts.push(hull_badge(Mat::Lens, &head, (lamp_t, z), (0.086, 0.037), 0.0, 0.0045));
    }

    // Behind the feet: a silver wall, framed by a dark shell that wraps around it.
    parts.push(loft(
        Mat::Silver,
        &[
            slice((512.0, 385.0), (512.0, 500.0), 0.1, 4.5),
            slice((530.0, 385.0), (530.0, 498.0), 0.135, 5.0),
            slice((590.0, 385.0), (575.0, 492.0), 0.14, 5.0),
        ],
    ));
    for side in [-1.0f32, 1.0] {
        parts.push(solid(Mat::Trim, at(537.0, 388.0, side * 0.134), at(555.0, 494.0, side * 0.142)));
    }
    parts.push(loft(
        Mat::Trim,
        &[
            slice((588.0, 383.0), (568.0, 500.0), 0.168, 5.0),
            slice((620.0, 383.0), (598.0, 520.0), 0.17, 5.0),
            slice((664.0, 383.0), (632.0, 472.0), 0.165, 4.5),
        ],
    ));

    // The rear of the body, sweeping from the tail down to the footboard.
    parts.push(loft(
        Mat::Body,
        &[
            slice((882.0, 288.0), (880.0, 300.0), 0.06, 2.6),
            slice((862.0, 286.0), (862.0, 338.0), 0.112, 3.0),
            slice((800.0, 287.0), (800.0, 375.0), 0.14, 3.2),
            slice((740.0, 297.0), (745.0, 422.0), 0.158, 3.4),
            slice((690.0, 316.0), (705.0, 482.0), 0.168, 3.6),
            slice((667.0, 385.0), (680.0, 508.0), 0.172, 3.8),
            slice((635.0, 465.0), (650.0, 535.0), 0.176, 4.0),
            slice((605.0, 510.0), (625.0, 566.0), 0.178, 4.0),
            slice((575.0, 532.0), (590.0, 574.0), 0.175, 4.0),
            slice((555.0, 572.0), (558.0, 580.0), 0.16, 3.5),
        ],
    ));

    // Under the seat: the bay and the two batteries, each with five charge lights and a handle.
    parts.push(loft(
        Mat::Cavity,
        &[
            slice((496.0, 368.0), (500.0, 396.0), 0.12, 5.0),
            slice((512.0, 378.0), (512.0, 396.0), 0.135, 5.0),
            slice((600.0, 378.0), (600.0, 392.0), 0.14, 5.0),
            slice((680.0, 360.0), (680.0, 392.0), 0.14, 5.0),
        ],
    ));
    for (battery, z) in [(0usize, -0.064f32), (1, 0.064)] {
        parts.push(solid(Mat::BatteryCase, at(525.0, 362.0, z - 0.053), at(640.0, 395.0, z + 0.053)));
        for pip in 0..5u8 {
            let (x0, x1) = (532.0 + f32::from(pip) * 16.0, 545.0 + f32::from(pip) * 16.0);
            let (a, b) = (px(x0, 0.0).0, px(x1, 0.0).0);
            let y = px(0.0, 362.0).1;
            parts.push(solid(Mat::Pip(battery, pip), [b, y, z - 0.03], [a, y + 0.004, z + 0.03]));
        }
        parts.push(tube(Mat::Dark, at(620.0, 355.0, z - 0.025), at(620.0, 355.0, z + 0.025), 0.007));
    }

    // The seat: a wedge, deep at the front, with a step up to the thin passenger part. It hinges at its front edge.
    let mut seat = loft(
        Mat::Seat,
        &[
            slice((492.0, 342.0), (490.0, 372.0), 0.075, 2.6),
            slice((500.0, 300.0), (497.0, 381.0), 0.118, 3.0),
            slice((528.0, 277.0), (525.0, 382.0), 0.15, 3.4),
            slice((620.0, 275.0), (615.0, 381.0), 0.16, 3.8),
            slice((680.0, 268.0), (666.0, 336.0), 0.152, 3.6),
            slice((726.0, 253.0), (718.0, 316.0), 0.14, 3.4),
            slice((790.0, 248.0), (790.0, 288.0), 0.122, 3.0),
            slice((850.0, 247.0), (850.0, 264.0), 0.08, 2.6),
            slice((864.0, 252.0), (864.0, 259.0), 0.03, 2.2),
        ],
    );
    seat.on_seat = true;
    parts.push(seat);

    // The passenger's grab rail, ending in a silver tip behind the seat.
    for side in [-1.0f32, 1.0] {
        parts.push(tube(Mat::Dark, at(752.0, 304.0, side * 0.14), at(800.0, 287.0, side * 0.135), 0.012));
        parts.push(tube(Mat::Dark, at(800.0, 287.0, side * 0.135), at(860.0, 276.0, side * 0.105), 0.012));
        parts.push(tube(Mat::Dark, at(860.0, 276.0, side * 0.105), at(900.0, 262.0, side * 0.05), 0.012));
        // The folding footrest for a passenger.
        parts.push(tube(Mat::Silver, at(682.0, 477.0, side * 0.17), at(738.0, 464.0, side * 0.18), 0.012));
    }
    parts.push(tube(Mat::Silver, at(900.0, 262.0, -0.055), at(900.0, 262.0, 0.055), 0.0125));

    // The back end: a dark bracket with the tail light, the reflectors and the long number plate holder.
    parts.push(loft(
        Mat::Dark,
        &[
            slice((830.0, 345.0), (810.0, 372.0), 0.06, 3.0),
            slice((880.0, 340.0), (880.0, 378.0), 0.07, 3.0),
            slice((905.0, 375.0), (900.0, 415.0), 0.072, 3.0),
            slice((940.0, 405.0), (920.0, 440.0), 0.072, 3.0),
            slice((970.0, 465.0), (950.0, 478.0), 0.066, 3.0),
            slice((992.0, 516.0), (982.0, 512.0), 0.05, 2.6),
        ],
    ));
    parts.push(loft(
        Mat::Tail,
        &[
            slice((892.0, 342.0), (892.0, 374.0), 0.05, 3.4),
            slice((922.0, 344.0), (922.0, 372.0), 0.052, 3.4),
            slice((946.0, 350.0), (946.0, 368.0), 0.04, 3.0),
        ],
    ));
    for side in [-1.0f32, 1.0] {
        parts.push(blob(Mat::Signal, at(925.0, 423.0, side * 0.074), [0.028, 0.028, 0.005]));
    }

    // Front mudguard: a cap over the tyre, with a blade down each side of the fork, and a dark piece behind it.
    parts.push(loft(
        Mat::Body,
        &[
            slice((135.0, 450.0), (138.0, 455.0), 0.022, 2.4),
            slice((180.0, 428.0), (186.0, 466.0), 0.06, 3.0),
            slice((230.0, 419.0), (240.0, 448.0), 0.068, 3.2),
            slice((246.0, 428.0), (250.0, 442.0), 0.062, 3.0),
        ],
    ));
    let blade = [
        slice((194.0, 440.0), (246.0, 440.0), 0.014, 3.0),
        slice((186.0, 490.0), (222.0, 498.0), 0.013, 3.0),
        slice((176.0, 540.0), (200.0, 556.0), 0.011, 3.0),
    ];
    for side in [-1.0f32, 1.0] {
        parts.push(loft(Mat::Body, &shifted(&blade, side * 0.078)));
        // The fork: chrome above, dark below.
        parts.push(tube(Mat::Metal, at(272.0, 412.0, side * 0.058), at(252.0, 470.0, side * 0.066), 0.017));
        parts.push(tube(Mat::Dark, at(252.0, 470.0, side * 0.066), at(205.0, 548.0, side * 0.066), 0.021));
    }
    parts.push(loft(
        Mat::Dark,
        &[
            slice((243.0, 434.0), (248.0, 446.0), 0.055, 3.0),
            slice((282.0, 438.0), (276.0, 462.0), 0.06, 3.0),
            slice((312.0, 488.0), (300.0, 496.0), 0.05, 2.6),
        ],
    ));

    // The steering neck, and the handlebars' cowl with the dash on top.
    parts.push(loft(
        Mat::Dark,
        &[
            slice((352.0, 195.0), (418.0, 200.0), 0.055, 3.0),
            slice((348.0, 225.0), (402.0, 238.0), 0.07, 3.0),
        ],
    ));
    let (cowl_x, cowl_y) = px(366.0, 165.0);
    let mut sides = Vec::new();
    for &(z, dx, dy, rx, ry, p) in &[
        (-0.25f32, 0.0f32, 0.012f32, 0.028f32, 0.026f32, 2.4f32),
        (-0.2, -0.008, 0.018, 0.058, 0.04, 2.6),
        (-0.12, 0.012, 0.012, 0.095, 0.058, 3.0),
        (-0.05, 0.03, 0.002, 0.112, 0.066, 2.7),
        (0.0, 0.034, -0.002, 0.116, 0.068, 2.7),
    ] {
        sides.push((z, cowl_x + dx, cowl_y + dy, rx, ry, p));
    }
    let mut mirrored: Vec<_> = sides.iter().rev().skip(1).map(|s| (-s.0, s.1, s.2, s.3, s.4, s.5)).collect();
    sides.append(&mut mirrored);
    parts.push(crosswise(Mat::Dark, &sides));
    let top = cowl_y + 0.066;
    parts.push(blob(Mat::Glass, [cowl_x + 0.03, top, 0.0], [0.07, 0.012, 0.085]));
    parts.push(blob(Mat::Screen, [cowl_x + 0.03, top + 0.008, 0.0], [0.03, 0.006, 0.03]));

    let bar = px(385.0, 160.0);
    for side in [-1.0f32, 1.0] {
        parts.push(tube(Mat::Dark, [bar.0, bar.1, side * 0.235], [bar.0, bar.1, side * 0.335], 0.018));
        parts.push(tube(Mat::Silver, [bar.0, bar.1, side * 0.335], [bar.0, bar.1, side * 0.345], 0.015));
        // Brake lever.
        parts.push(tube(Mat::Silver, [bar.0 + 0.055, bar.1 + 0.002, side * 0.2], [bar.0 + 0.075, bar.1 - 0.006, side * 0.33], 0.0055));
        // Indicator.
        parts.push(blob(Mat::Signal, [bar.0 + 0.1, bar.1 - 0.012, side * 0.205], [0.018, 0.01, 0.028]));
        // Mirror on its thin stalk: a rounded plate with the glass facing the rider.
        let base = [bar.0 + 0.02, bar.1 + 0.04, side * 0.13];
        let head = [px(380.0, 0.0).0, px(0.0, 56.0).1, side * 0.29];
        parts.push(tube(Mat::Dark, base, [head[0], head[1] - 0.04, head[2] - side * 0.015], 0.0065));
        parts.push(blob(Mat::Dark, head, [0.014, 0.05, 0.062]));
        parts.push(blob(Mat::Glass, [head[0] - 0.008, head[1], head[2]], [0.009, 0.042, 0.054]));
    }

    // The centre stand, folded down.
    for side in [-1.0f32, 1.0] {
        parts.push(tube(Mat::Dark, at(642.0, 566.0, side * 0.1), at(622.0, 655.0, side * 0.115), 0.012));
        parts.push(tube(Mat::Dark, at(622.0, 655.0, side * 0.115), at(700.0, 658.0, side * 0.125), 0.008));
    }
    parts.push(tube(Mat::Dark, at(622.0, 655.0, -0.115), at(622.0, 655.0, 0.115), 0.011));
    parts
}

fn model() -> &'static [Part] {
    static MODEL: OnceLock<Vec<Part>> = OnceLock::new();
    MODEL.get_or_init(build)
}

const SEAT_OPEN_DEG: f32 = 56.0;
const TARGET: V3 = [-0.04, 0.63, 0.0];
const CAMERA_DISTANCE: f32 = 3.7;
const PIPS: u8 = 5;

/// Where a tappable part sits on the moped.
pub const ANCHOR_BATTERIES: V3 = [-0.1, 0.72, 0.0];
pub const ANCHOR_MOTOR: V3 = [-0.67, 0.256, 0.1];
pub const ANCHOR_DASH: V3 = [0.42, 1.11, 0.0];

#[derive(Clone, Copy, PartialEq)]
pub struct Scene {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    /// 0 is closed, 1 is fully open.
    pub seat: f32,
    pub body: [u8; 3],
    pub accent: [u8; 3],
    pub danger: [u8; 3],
    /// Charge of each battery, or `None` when it isn't there or isn't known.
    pub charge: [Option<u8>; 2],
    pub lights_on: bool,
    pub warning: bool,
}

fn rgb(c: [u8; 3]) -> [f32; 3] {
    [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0]
}

#[derive(Clone, Copy)]
struct Look {
    base: [f32; 3],
    spec: f32,
    gloss: i32,
    /// How much it mirrors its surroundings, as paint and chrome do.
    mirror: f32,
    glow: bool,
}

fn look(mat: Mat, scene: &Scene) -> Look {
    let surface = |base: [f32; 3], spec: f32, gloss: i32, mirror: f32| Look { base, spec, gloss, mirror, glow: false };
    let glow = |base: [f32; 3]| Look { base, spec: 0.0, gloss: 1, mirror: 0.0, glow: true };
    match mat {
        Mat::Body => surface(rgb(scene.body), 0.45, 40, 0.16),
        Mat::Trim => surface([0.055, 0.055, 0.06], 0.12, 14, 0.06),
        Mat::Seat => surface([0.125, 0.125, 0.13], 0.12, 12, 0.07),
        Mat::Tyre => surface([0.045, 0.045, 0.05], 0.08, 10, 0.03),
        Mat::Rim => surface([0.05, 0.05, 0.055], 0.5, 44, 0.2),
        Mat::Metal => surface([0.5, 0.5, 0.53], 0.5, 36, 0.3),
        Mat::Silver => surface([0.62, 0.62, 0.62], 0.3, 20, 0.14),
        Mat::Dark => surface([0.06, 0.06, 0.065], 0.2, 20, 0.08),
        Mat::Cavity => surface([0.025, 0.025, 0.03], 0.0, 1, 0.0),
        Mat::BatteryCase => surface([0.17, 0.18, 0.2], 0.3, 24, 0.08),
        Mat::Glass => surface([0.04, 0.05, 0.06], 0.7, 80, 0.45),
        Mat::Lens => surface([0.7, 0.72, 0.76], 0.7, 60, 0.4),
        Mat::Head if scene.lights_on => glow([1.0, 0.99, 0.94]),
        Mat::Head => surface([0.86, 0.88, 0.9], 0.7, 50, 0.3),
        Mat::Tail if scene.lights_on => glow([0.95, 0.12, 0.14]),
        Mat::Tail => surface([0.55, 0.52, 0.52], 0.7, 50, 0.35),
        Mat::Signal => surface([0.95, 0.52, 0.08], 0.6, 40, 0.15),
        Mat::Screen if scene.warning => glow(rgb(scene.danger)),
        Mat::Screen if scene.lights_on => glow(rgb(scene.accent)),
        Mat::Screen => surface([0.04, 0.05, 0.06], 0.4, 40, 0.3),
        Mat::Pip(battery, index) => match scene.charge[battery] {
            Some(percent) if u32::from(percent) * u32::from(PIPS) > u32::from(index) * 100 => {
                glow(rgb(if percent <= 20 { scene.danger } else { scene.accent }))
            }
            _ => surface([0.09, 0.09, 0.1], 0.1, 8, 0.0),
        },
    }
}

/// What a shiny surface sees: bright sky, a soft light overhead and dark ground.
fn surroundings(up: f32) -> f32 {
    let sky = 0.55 + 0.45 * up.max(0.0);
    let t = ((up + 0.2) / 0.5).clamp(0.0, 1.0);
    // A soft light overhead: it fades in and out, as a hard edge would look like a crack in the paint.
    let around = 1.0 - ((up - 0.5) / 0.22).abs().min(1.0);
    let band = 0.3 * around * around * (3.0 - 2.0 * around);
    0.1 + (sky + band - 0.1) * t * t * (3.0 - 2.0 * t)
}

fn shade(l: &Look, n: V3) -> [f32; 3] {
    if l.glow {
        return l.base;
    }
    const KEY: V3 = [-0.39, 0.74, 0.55];
    const FILL: V3 = [0.72, 0.2, 0.66];
    const HALF: V3 = [-0.21, 0.4, 0.89];
    let sky = 0.36 + 0.2 * (n[1] * 0.5 + 0.5);
    let light = sky + 0.62 * dot(n, KEY).max(0.0) + 0.2 * dot(n, FILL).max(0.0);
    let spec = l.spec * dot(n, HALF).max(0.0).powi(l.gloss);
    let facing = n[2].clamp(0.0, 1.0);
    let edge = 1.0 - facing;
    let fresnel = 0.05 + 0.95 * edge * edge * edge * edge * edge;
    // The reflection only needs its height, as the surroundings don't change around the moped.
    let mirrored = l.mirror * (0.25 + fresnel * 2.2) * surroundings(2.0 * facing * n[1]);
    [
        (l.base[0] * light + spec + mirrored).min(1.0),
        (l.base[1] * light + spec + mirrored).min(1.0),
        (l.base[2] * light + spec + mirrored).min(1.0),
    ]
}

struct View {
    sy: f32,
    cy: f32,
    sp: f32,
    cp: f32,
    focal: f32,
    w: f32,
    h: f32,
}

impl View {
    fn new(scene: &Scene, w: f32, h: f32) -> Self {
        // Fits the moped's length across the width, or its height down a tall view.
        let focal = (w * 1.46).min(h * 2.3) * scene.zoom;
        Self {
            sy: scene.yaw.sin(),
            cy: scene.yaw.cos(),
            sp: scene.pitch.sin(),
            cp: scene.pitch.cos(),
            focal,
            w,
            h,
        }
    }

    fn rotate(&self, v: V3) -> V3 {
        let x = v[0] * self.cy + v[2] * self.sy;
        let z = -v[0] * self.sy + v[2] * self.cy;
        [x, v[1] * self.cp - z * self.sp, v[1] * self.sp + z * self.cp]
    }

    /// Screen x and y, and the inverse of the depth.
    fn project(&self, p: V3) -> Option<[f32; 3]> {
        let v = self.rotate(sub(p, TARGET));
        let depth = CAMERA_DISTANCE - v[2];
        if depth < 0.2 {
            return None;
        }
        let inv = 1.0 / depth;
        Some([self.w / 2.0 + self.focal * v[0] * inv, self.h / 2.0 - self.focal * v[1] * inv, inv])
    }
}

/// Where a point of the moped lands in a view, from 0 to 1 each way.
pub fn project_point(scene: &Scene, point: V3, aspect: f32) -> Option<(f32, f32)> {
    let view = View::new(scene, aspect, 1.0);
    let spot = view.project(point)?;
    Some((spot[0] / aspect, spot[1]))
}

const NOTHING: u16 = u16::MAX;

/// What each pixel shows. The nearest surface wins first and is only lit afterwards, so nothing gets lit twice.
struct Target {
    w: usize,
    h: usize,
    shadow: Vec<f32>,
    depth: Vec<f32>,
    normal: Vec<V3>,
    part: Vec<u16>,
}

impl Target {
    fn triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], normals: [V3; 3], part: u16) {
        let Some((min_x, max_x, min_y, max_y, inv)) = self.bounds(a, b, c) else {
            return;
        };
        for y in min_y..=max_y {
            let py = y as f32 + 0.5;
            let row = y * self.w;
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) * inv;
                let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let i = row + x;
                let z = a[2] * w0 + b[2] * w1 + c[2] * w2;
                if z <= self.depth[i] {
                    continue;
                }
                self.depth[i] = z;
                self.part[i] = part;
                self.normal[i] = add(add(scale(normals[0], w0), scale(normals[1], w1)), scale(normals[2], w2));
            }
        }
    }

    fn shadow_triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], dark: [f32; 3]) {
        let Some((min_x, max_x, min_y, max_y, inv)) = self.bounds(a, b, c) else {
            return;
        };
        for y in min_y..=max_y {
            let py = y as f32 + 0.5;
            let row = y * self.w;
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) * inv;
                let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) * inv;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let amount = dark[0] * w0 + dark[1] * w1 + dark[2] * w2;
                if amount > self.shadow[row + x] {
                    self.shadow[row + x] = amount;
                }
            }
        }
    }

    fn bounds(&self, a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<(usize, usize, usize, usize, f32)> {
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-5 {
            return None;
        }
        let max_x = (a[0].max(b[0]).max(c[0]).ceil() as isize).min(self.w as isize - 1);
        let max_y = (a[1].max(b[1]).max(c[1]).ceil() as isize).min(self.h as isize - 1);
        if max_x < 0 || max_y < 0 {
            return None;
        }
        let min_x = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
        let min_y = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
        Some((min_x, max_x as usize, min_y, max_y as usize, 1.0 / area))
    }

    /// How much nearby surfaces crowd this pixel: it darkens the seams between parts, as real light does.
    fn crowding(&self, i: usize, reach: usize) -> f32 {
        const NEAR: f32 = 0.0007;
        const FAR: f32 = 0.011;
        let z = self.depth[i];
        let (x, y, r) = ((i % self.w) as isize, (i / self.w) as isize, reach as isize);
        let mut hits = 0u32;
        for (dx, dy) in [(r, 0), (-r, 0), (0, r), (0, -r), (r, r), (-r, r), (r, -r), (-r, -r)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx >= 0 && ny >= 0 && (nx as usize) < self.w && (ny as usize) < self.h {
                let gap = self.depth[ny as usize * self.w + nx as usize] - z;
                if gap > NEAR && gap < FAR {
                    hits += 1;
                }
            }
        }
        hits as f32 / 8.0
    }
}

fn seat_position(p: V3, hinge: (f32, f32), sin: f32, cos: f32) -> V3 {
    let (dx, dy) = (p[0] - hinge.0, p[1] - hinge.1);
    [hinge.0 + dx * cos + dy * sin, hinge.1 - dx * sin + dy * cos, p[2]]
}

/// Draws the scene. Returns premultiplied RGBA, `width * height * 4` bytes. `smooth` is the side of the
/// supersampling grid: 2 for a crisp still picture, 1 while something is moving.
pub fn render(scene: &Scene, width: usize, height: usize, smooth: usize) -> Vec<u8> {
    let (w, h) = (width * smooth, height * smooth);
    let view = View::new(scene, w as f32, h as f32);
    let count = w * h;
    let mut target = Target {
        w,
        h,
        shadow: vec![0.0; count],
        depth: vec![0.0; count],
        normal: vec![[0.0; 3]; count],
        part: vec![NOTHING; count],
    };

    // A soft shadow on the ground, under everything.
    const SHADOW_STEPS: usize = 28;
    if let Some(centre) = view.project([-0.04, 0.0, 0.0]) {
        for i in 0..SHADOW_STEPS {
            let spot = |k: usize| {
                let t = k as f32 / SHADOW_STEPS as f32 * std::f32::consts::TAU;
                view.project([-0.04 + 1.08 * t.cos(), 0.0, 0.3 * t.sin()])
            };
            if let (Some(a), Some(b)) = (spot(i), spot(i + 1)) {
                let dark = if view.cp > 0.0 { 0.34 } else { 0.0 };
                target.shadow_triangle(centre, a, b, [dark, 0.0, 0.0]);
            }
        }
    }

    let hinge = px(496.0, 372.0);
    let angle = (scene.seat.clamp(0.0, 1.0) * SEAT_OPEN_DEG).to_radians();
    let (seat_sin, seat_cos) = angle.sin_cos();
    let parts = model();
    let looks: Vec<Look> = parts.iter().map(|part| look(part.mat, scene)).collect();
    let mut screen = Vec::new();
    let mut turned = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        screen.clear();
        turned.clear();
        for (p, n) in part.pos.iter().zip(&part.nor) {
            let (p, n) = if part.on_seat {
                (seat_position(*p, hinge, seat_sin, seat_cos), [n[0] * seat_cos + n[1] * seat_sin, -n[0] * seat_sin + n[1] * seat_cos, n[2]])
            } else {
                (*p, *n)
            };
            screen.push(view.project(p));
            turned.push(view.rotate(n));
        }
        for t in &part.tris {
            let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let (Some(a), Some(b), Some(c)) = (screen[i0], screen[i1], screen[i2]) else {
                continue;
            };
            // Screen y points down, so a face turned toward the camera winds negative.
            if (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) >= 0.0 {
                continue;
            }
            target.triangle(a, b, c, [turned[i0], turned[i1], turned[i2]], index as u16);
        }
    }

    // Light each pixel once, now that it's known what it shows.
    let reach = (w.max(h) / 130).max(1);
    let mut lit = vec![[0.0f32; 4]; count];
    for i in 0..count {
        let part = target.part[i];
        if part == NOTHING {
            lit[i] = [0.0, 0.0, 0.0, target.shadow[i]];
            continue;
        }
        let material = &looks[part as usize];
        let colour = shade(material, norm(target.normal[i]));
        let dim = if material.glow { 1.0 } else { 1.0 - 0.5 * target.crowding(i, reach) };
        lit[i] = [colour[0] * dim, colour[1] * dim, colour[2] * dim, 1.0];
    }

    let mut out = vec![0u8; width * height * 4];
    let divide = (smooth * smooth) as f32;
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            for sy in 0..smooth {
                for sx in 0..smooth {
                    let c = lit[(y * smooth + sy) * w + x * smooth + sx];
                    for k in 0..4 {
                        sum[k] += c[k];
                    }
                }
            }
            let spot = (y * width + x) * 4;
            for k in 0..4 {
                out[spot + k] = (sum[k] / divide * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Scene {
        Scene {
            yaw: 0.0,
            pitch: 0.2,
            zoom: 1.0,
            seat: 0.0,
            body: [235, 235, 230],
            accent: [75, 75, 200],
            danger: [200, 40, 40],
            charge: [Some(60), Some(100)],
            lights_on: true,
            warning: false,
        }
    }

    fn solid_pixels(image: &[u8]) -> usize {
        image.chunks(4).filter(|p| p[3] == 255).count()
    }

    #[test]
    fn draws_a_moped_with_clear_corners() {
        let image = render(&scene(), 200, 150, 1);
        assert_eq!(image.len(), 200 * 150 * 4);
        let covered = solid_pixels(&image);
        assert!(covered > 2000, "only {covered} pixels");
        assert_eq!(image[3], 0);
        assert_eq!(image[image.len() - 1], 0);
    }

    #[test]
    fn it_fits_the_view_from_every_side_with_the_seat_shut_or_open() {
        for seat in [0.0, 1.0] {
            for step in 0..12 {
                let s = Scene { yaw: step as f32 * 0.52, seat, ..scene() };
                let image = render(&s, 200, 150, 1);
                assert!(solid_pixels(&image) > 800, "step {step}");
                for x in 0..200 {
                    assert!(image[x * 4 + 3] != 255, "touches the top at step {step}");
                }
            }
        }
    }

    #[test]
    fn opening_the_seat_changes_the_picture_and_shows_the_charge_lights() {
        let top = Scene { pitch: 1.0, yaw: 0.5, ..scene() };
        let closed = render(&top, 240, 180, 1);
        let open = render(&Scene { seat: 1.0, ..top }, 240, 180, 1);
        assert_ne!(closed, open);
        let accent = |image: &[u8]| image.chunks(4).filter(|p| p[0..3] == [75, 75, 200] && p[3] == 255).count();
        assert_eq!(accent(&closed) > accent(&open), false);
        assert!(accent(&open) > 10);
        let empty = render(&Scene { seat: 1.0, charge: [None, None], lights_on: false, ..top }, 240, 180, 1);
        assert!(accent(&empty) < accent(&open));
    }

    #[test]
    fn charge_decides_how_many_lights_are_on() {
        let on = |percent: u8| {
            (0..PIPS)
                .filter(|&i| {
                    let s = Scene { charge: [Some(percent), None], ..scene() };
                    look(Mat::Pip(0, i), &s).glow
                })
                .count()
        };
        assert_eq!(on(100), 5);
        assert_eq!(on(61), 4);
        assert_eq!(on(60), 3);
        assert_eq!(on(1), 1);
        assert_eq!(on(0), 0);
    }

    #[test]
    fn smoothing_softens_the_edges() {
        let image = render(&scene(), 120, 90, 2);
        assert!(image.chunks(4).any(|p| p[3] > 0 && p[3] < 255));
    }

    #[test]
    fn points_project_inside_the_view() {
        for anchor in [ANCHOR_BATTERIES, ANCHOR_MOTOR, ANCHOR_DASH] {
            let (x, y) = project_point(&scene(), anchor, 1.3).unwrap();
            assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y), "{x} {y}");
        }
    }
}
