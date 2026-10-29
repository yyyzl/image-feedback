//! Shared image-coordinate geometry for direct manipulation and raster output.
//! Positive angles rotate clockwise in the image's downward-positive Y axis.
use crate::document::{Point, Rect};

pub const MIN_SCALE: f32 = 0.05;
pub const MAX_SCALE: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Resize(usize),
    Rotate,
}

pub fn center(r: Rect) -> Point {
    Point {
        x: r.x + r.width / 2.,
        y: r.y + r.height / 2.,
    }
}
pub fn rotate(p: Point, degrees: f32) -> Point {
    let (s, c) = degrees.to_radians().sin_cos();
    Point {
        x: p.x * c - p.y * s,
        y: p.x * s + p.y * c,
    }
}
pub fn local_to_image(r: Rect, angle: f32, p: Point) -> Point {
    let c = center(r);
    let v = rotate(p, angle);
    Point {
        x: c.x + v.x,
        y: c.y + v.y,
    }
}
pub fn image_to_local(r: Rect, angle: f32, p: Point) -> Point {
    let c = center(r);
    rotate(
        Point {
            x: p.x - c.x,
            y: p.y - c.y,
        },
        -angle,
    )
}
pub fn corners(r: Rect, angle: f32) -> [Point; 4] {
    [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)].map(|(x, y)| {
        local_to_image(
            r,
            angle,
            Point {
                x: x * r.width / 2.,
                y: y * r.height / 2.,
            },
        )
    })
}
pub fn bounds(r: Rect, angle: f32) -> Rect {
    let points = corners(r, angle);
    let (mut l, mut t, mut right, mut bottom) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for p in points {
        l = l.min(p.x);
        t = t.min(p.y);
        right = right.max(p.x);
        bottom = bottom.max(p.y);
    }
    Rect {
        x: l,
        y: t,
        width: right - l,
        height: bottom - t,
    }
}
pub fn contains(r: Rect, angle: f32, p: Point) -> bool {
    let q = image_to_local(r, angle, p);
    q.x.abs() <= r.width / 2. && q.y.abs() <= r.height / 2.
}
pub fn rotation_handle(r: Rect, angle: f32, view_scale: f32) -> Point {
    local_to_image(
        r,
        angle,
        Point {
            x: 0.,
            y: -r.height / 2. - 30. / view_scale,
        },
    )
}
/// Screen-sized hit areas, with resize handles taking precedence over rotation.
pub fn hit_handle(
    r: Rect,
    angle: f32,
    p: Point,
    view_scale: f32,
    rotation_point: Point,
) -> Option<Handle> {
    if !view_scale.is_finite() || view_scale <= 0. {
        return None;
    }
    let q = image_to_local(r, angle, p);
    let hx = r.width / 2.;
    let hy = r.height / 2.;
    let directions = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)];
    for (i, (sx, sy)) in directions.iter().enumerate() {
        if (q.x - sx * hx).abs() * view_scale <= 8. && (q.y - sy * hy).abs() * view_scale <= 8. {
            return Some(Handle::Resize(i));
        }
    }
    if (p.x - rotation_point.x).hypot(p.y - rotation_point.y) * view_scale <= 11. {
        return Some(Handle::Rotate);
    }
    for (sx, sy) in directions {
        let dx = (q.x - sx * hx) * view_scale;
        let dy = (q.y - sy * hy) * view_scale;
        if dx.abs() <= 28. && dy.abs() <= 28. && (dx * sx > 0. || dy * sy > 0.) {
            return Some(Handle::Rotate);
        }
    }
    None
}
/// Resize uniformly about the opposite corner; no axis flipping.
pub fn resize(source: Rect, original: Rect, angle: f32, corner: usize, pointer: Point) -> Rect {
    let directions = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)];
    let (sx, sy) = directions[corner.min(3)];
    let anchor = corners(original, angle)[(corner + 2) % 4];
    let v = rotate(
        Point {
            x: pointer.x - anchor.x,
            y: pointer.y - anchor.y,
        },
        -angle,
    );
    let denom = source.width * source.width + source.height * source.height;
    let scale =
        ((v.x * sx * source.width + v.y * sy * source.height) / denom).clamp(MIN_SCALE, MAX_SCALE);
    let width = source.width * scale;
    let height = source.height * scale;
    let half = rotate(
        Point {
            x: sx * width / 2.,
            y: sy * height / 2.,
        },
        angle,
    );
    Rect {
        x: anchor.x + half.x - width / 2.,
        y: anchor.y + half.y - height / 2.,
        width,
        height,
    }
}
pub fn normalize_angle(angle: f32) -> f32 {
    (angle + 180.).rem_euclid(360.) - 180.
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect() -> Rect {
        Rect {
            x: 100.,
            y: 100.,
            width: 120.,
            height: 60.,
        }
    }
    #[test]
    fn rotated_contains_and_bounds_agree() {
        let r = rect();
        assert!(contains(r, 45., center(r)));
        let c = corners(r, 45.);
        let b = bounds(r, 45.);
        for p in c {
            assert!(
                p.x >= b.x - 0.01
                    && p.x <= b.x + b.width + 0.01
                    && p.y >= b.y - 0.01
                    && p.y <= b.y + b.height + 0.01
            );
        }
        assert!(!contains(r, 45., Point { x: b.x, y: b.y }));
    }
    #[test]
    fn resize_keeps_opposite_corner_at_any_angle() {
        let r = rect();
        for angle in [0., 45., -120.] {
            for i in 0..4 {
                let old = corners(r, angle);
                let center = center(r);
                let p = Point {
                    x: center.x + (old[i].x - center.x) * 1.6,
                    y: center.y + (old[i].y - center.y) * 1.6,
                };
                let out = resize(r, r, angle, i, p);
                let anchor = corners(out, angle)[(i + 2) % 4];
                assert!(
                    (anchor.x - old[(i + 2) % 4].x).hypot(anchor.y - old[(i + 2) % 4].y) < 0.001
                );
                assert!((out.width / out.height - 2.).abs() < 0.001);
            }
        }
    }
    #[test]
    fn corners_and_rotation_regions_are_zoom_independent() {
        let r = rect();
        for z in [0.25, 1., 4.] {
            for angle in [0., 37.] {
                for (i, p) in corners(r, angle).into_iter().enumerate() {
                    assert_eq!(
                        hit_handle(r, angle, p, z, rotation_handle(r, angle, z)),
                        Some(Handle::Resize(i))
                    );
                    let signs = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)][i];
                    let offset = rotate(
                        Point {
                            x: signs.0 * 18. / z,
                            y: signs.1 * 18. / z,
                        },
                        angle,
                    );
                    assert_eq!(
                        hit_handle(
                            r,
                            angle,
                            Point {
                                x: p.x + offset.x,
                                y: p.y + offset.y
                            },
                            z,
                            rotation_handle(r, angle, z)
                        ),
                        Some(Handle::Rotate)
                    );
                }
            }
        }
    }
}
