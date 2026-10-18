// Frame Zero - a tiny Windows screenshot and annotation tool
// Copyright (C) 2026 Claude Henchoz
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use crate::gfx::bitmap::Bitmap;
use std::collections::HashMap;
use std::rc::Rc;

pub use crate::gfx::color::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn dist(self, o: Point) -> f32 {
        ((self.x - o.x) * (self.x - o.x) + (self.y - o.y) * (self.y - o.y)).sqrt()
    }
    /// Rotate around `c` by `deg` degrees (clockwise in screen space, y down).
    pub fn rotated(self, c: Point, deg: f32) -> Point {
        if deg == 0.0 {
            return self;
        }
        let (s, co) = deg.to_radians().sin_cos();
        let dx = self.x - c.x;
        let dy = self.y - c.y;
        Point::new(c.x + dx * co - dy * s, c.y + dx * s + dy * co)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn from_points(a: Point, b: Point) -> Self {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        let w = (a.x - b.x).abs();
        let h = (a.y - b.y).abs();
        Self { x, y, w, h }
    }
    pub fn bounding(pts: &[Point]) -> Self {
        let mut l = f32::MAX;
        let mut t = f32::MAX;
        let mut r = f32::MIN;
        let mut b = f32::MIN;
        for p in pts {
            l = l.min(p.x);
            t = t.min(p.y);
            r = r.max(p.x);
            b = b.max(p.y);
        }
        if pts.is_empty() {
            return Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        }
        Rect { x: l, y: t, w: r - l, h: b - t }
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
    }
    pub fn inflated(&self, by: f32) -> Self {
        Self {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2.0 * by,
            h: self.h + 2.0 * by,
        }
    }
    pub fn center(&self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let l = self.x.min(o.x);
        let t = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        Rect { x: l, y: t, w: r - l, h: b - t }
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }
    pub fn translated(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..*self }
    }
    /// Map a point expressed relative to `self` into `to` (used by resize).
    pub fn map_point(&self, to: &Rect, p: Point) -> Point {
        let sx = if self.w.abs() > 1e-3 { to.w / self.w } else { 1.0 };
        let sy = if self.h.abs() > 1e-3 { to.h / self.h } else { 1.0 };
        Point::new(to.x + (p.x - self.x) * sx, to.y + (p.y - self.y) * sy)
    }
    pub fn map_rect(&self, to: &Rect, r: Rect) -> Rect {
        let a = self.map_point(to, Point::new(r.x, r.y));
        let b = self.map_point(to, Point::new(r.right(), r.bottom()));
        Rect::from_points(a, b)
    }
    /// The four corners in TL, TR, BR, BL order.
    pub fn corners(&self) -> [Point; 4] {
        [
            Point::new(self.x, self.y),
            Point::new(self.right(), self.y),
            Point::new(self.right(), self.bottom()),
            Point::new(self.x, self.bottom()),
        ]
    }
}

pub type AnnotationId = u32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
    Polygon,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlurMode {
    Solid,
    Blur,
    Pixelate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EndStyle {
    Arrow,
    Circle,
    Square,
    None,
}

impl EndStyle {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => EndStyle::Circle,
            2 => EndStyle::Square,
            3 => EndStyle::None,
            _ => EndStyle::Arrow,
        }
    }
    pub fn to_u8(self) -> u8 {
        match self {
            EndStyle::Arrow => 0,
            EndStyle::Circle => 1,
            EndStyle::Square => 2,
            EndStyle::None => 3,
        }
    }
}

/// A callout tail: `anchor` is relative to the body (0..1 in both axes, on or
/// inside the body), `tip` is the absolute point the tail points at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tail {
    pub anchor: Point,
    pub tip: Point,
}

#[derive(Clone, Debug)]
pub enum Annotation {
    Arrow {
        id: AnnotationId,
        start: Point,
        end: Point,
        color: Color,
        stroke: f32,
        /// Optional cubic-bezier control points. `None` draws a straight
        /// arrow; `Some((c1, c2))` draws a curved shaft.
        bezier: Option<(Point, Point)>,
        head: EndStyle,
        tail: EndStyle,
        shadow: bool,
    },
    Shape {
        id: AnnotationId,
        kind: ShapeKind,
        bounds: Rect,
        /// Polygon vertices (absolute document coordinates). Empty for
        /// rectangles/ellipses; `bounds` is kept in sync for polygons.
        points: Vec<Point>,
        outline: Color,
        fill: Color, // alpha=0 means transparent
        stroke: f32,
        rotation: f32,
        shadow: bool,
    },
    Callout {
        id: AnnotationId,
        bounds: Rect,
        text: String,
        fill: Color,
        outline: Color,
        text_color: Color,
        size: f32,
        stroke: f32,
        tails: Vec<Tail>,
        rotation: f32,
        radius: f32,
        shadow: bool,
    },
    Text {
        id: AnnotationId,
        bounds: Rect,
        text: String,
        color: Color,
        size: f32,
        rotation: f32,
    },
    Step {
        id: AnnotationId,
        center: Point,
        radius: f32,
        value: String,
        fill: Color,
        text: Color,
    },
    Highlighter {
        id: AnnotationId,
        bounds: Rect,
        color: Color,
    },
    Blur {
        id: AnnotationId,
        bounds: Rect,
        mode: BlurMode,
        intensity: f32,
        fill: Color, // used when mode == Solid
    },
    /// Magnifier: shows a scaled-up copy of `source` at `display`, with a
    /// connecting line + rectangle outlines.
    Magnify {
        id: AnnotationId,
        source: Rect,
        display: Rect,
        line_width: f32,
        color: Color,
    },
    /// Stamp: a bundled vector stamp (`"b:<name>"`) or a user PNG from the
    /// stamps folder (`"f:<category>/<file>"`), placed at `bounds`.
    Stamp {
        id: AnnotationId,
        asset: String,
        bounds: Rect,
        rotation: f32,
    },
    /// Raster layer: pasted images and pixels lifted by the Selection tool.
    Image {
        id: AnnotationId,
        bounds: Rect,
        bitmap: Rc<Bitmap>,
        rotation: f32,
    },
}

/// Which part of an annotation a pointer is over (for single-object drags).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Whole,
    MagnifySource,
    MagnifyDisplay,
}

impl Annotation {
    pub fn id(&self) -> AnnotationId {
        match self {
            Annotation::Arrow { id, .. }
            | Annotation::Shape { id, .. }
            | Annotation::Callout { id, .. }
            | Annotation::Text { id, .. }
            | Annotation::Step { id, .. }
            | Annotation::Highlighter { id, .. }
            | Annotation::Blur { id, .. }
            | Annotation::Magnify { id, .. }
            | Annotation::Stamp { id, .. }
            | Annotation::Image { id, .. } => *id,
        }
    }

    pub fn set_id(&mut self, new: AnnotationId) {
        match self {
            Annotation::Arrow { id, .. }
            | Annotation::Shape { id, .. }
            | Annotation::Callout { id, .. }
            | Annotation::Text { id, .. }
            | Annotation::Step { id, .. }
            | Annotation::Highlighter { id, .. }
            | Annotation::Blur { id, .. }
            | Annotation::Magnify { id, .. }
            | Annotation::Stamp { id, .. }
            | Annotation::Image { id, .. } => *id = new,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Annotation::Arrow { bezier: Some(_), .. } => "Curved arrow",
            Annotation::Arrow { .. } => "Arrow",
            Annotation::Shape { kind: ShapeKind::Rectangle, .. } => "Rectangle",
            Annotation::Shape { kind: ShapeKind::Ellipse, .. } => "Ellipse",
            Annotation::Shape { kind: ShapeKind::Polygon, .. } => "Polygon",
            Annotation::Callout { .. } => "Callout",
            Annotation::Text { .. } => "Text",
            Annotation::Step { .. } => "Step",
            Annotation::Highlighter { .. } => "Highlight",
            Annotation::Blur { mode: BlurMode::Solid, .. } => "Redaction",
            Annotation::Blur { mode: BlurMode::Pixelate, .. } => "Pixelate",
            Annotation::Blur { .. } => "Blur",
            Annotation::Magnify { .. } => "Magnifier",
            Annotation::Stamp { .. } => "Stamp",
            Annotation::Image { .. } => "Image",
        }
    }

    /// Rotation in degrees for rotatable kinds, else 0.
    pub fn rotation(&self) -> f32 {
        match self {
            Annotation::Shape { rotation, .. }
            | Annotation::Callout { rotation, .. }
            | Annotation::Text { rotation, .. }
            | Annotation::Stamp { rotation, .. }
            | Annotation::Image { rotation, .. } => *rotation,
            _ => 0.0,
        }
    }

    pub fn rotatable(&self) -> bool {
        matches!(
            self,
            Annotation::Shape { .. }
                | Annotation::Callout { .. }
                | Annotation::Text { .. }
                | Annotation::Stamp { .. }
                | Annotation::Image { .. }
        )
    }

    pub fn set_rotation(&mut self, deg: f32) {
        let mut d = deg % 360.0;
        if d < 0.0 {
            d += 360.0;
        }
        match self {
            Annotation::Shape { rotation, .. }
            | Annotation::Callout { rotation, .. }
            | Annotation::Text { rotation, .. }
            | Annotation::Stamp { rotation, .. }
            | Annotation::Image { rotation, .. } => *rotation = d,
            _ => {}
        }
    }

    /// The (unrotated) frame the 8 resize handles sit on. Arrows have no frame
    /// (they're edited via endpoint handles) — see `has_frame`.
    pub fn frame(&self) -> Rect {
        match self {
            Annotation::Arrow { start, end, bezier, .. } => {
                let mut pts = vec![*start, *end];
                if let Some((a, b)) = bezier {
                    pts.push(*a);
                    pts.push(*b);
                }
                Rect::bounding(&pts)
            }
            Annotation::Shape { bounds, .. }
            | Annotation::Callout { bounds, .. }
            | Annotation::Text { bounds, .. }
            | Annotation::Highlighter { bounds, .. }
            | Annotation::Blur { bounds, .. }
            | Annotation::Stamp { bounds, .. }
            | Annotation::Image { bounds, .. } => *bounds,
            Annotation::Step { center, radius, .. } => Rect {
                x: center.x - radius,
                y: center.y - radius,
                w: radius * 2.0,
                h: radius * 2.0,
            },
            Annotation::Magnify { display, .. } => *display,
        }
    }

    pub fn has_frame(&self) -> bool {
        !matches!(self, Annotation::Arrow { .. })
    }

    /// Axis-aligned box covering everything drawn (rotation, tails, stroke).
    pub fn bounding_box(&self) -> Rect {
        let rot = self.rotation();
        let f = self.frame();
        let mut r = if rot != 0.0 {
            let c = f.center();
            let pts: Vec<Point> = f.corners().iter().map(|p| p.rotated(c, rot)).collect();
            Rect::bounding(&pts)
        } else {
            f
        };
        match self {
            Annotation::Arrow { stroke, .. } => r = r.inflated(stroke * 3.0 + 6.0),
            Annotation::Shape { stroke, .. } => r = r.inflated(stroke * 0.5),
            Annotation::Callout { tails, bounds, rotation, .. } => {
                let c = bounds.center();
                for t in tails {
                    let tip = t.tip.rotated(c, *rotation);
                    r = r.union(&Rect { x: tip.x, y: tip.y, w: 0.0, h: 0.0 });
                }
            }
            Annotation::Magnify { source, .. } => r = r.union(source),
            _ => {}
        }
        r
    }

    /// Precise-ish hit test in document space.
    pub fn hit_test(&self, p: Point, tol: f32) -> bool {
        let f = self.frame();
        let local = p.rotated(f.center(), -self.rotation());
        match self {
            Annotation::Arrow { start, end, bezier, stroke, .. } => {
                let pts = arrow_polyline(*start, *end, *bezier);
                let lim = stroke * 0.5 + tol + 2.0;
                pts.windows(2).any(|w| dist_to_segment(p, w[0], w[1]) <= lim)
                    || p.dist(*end) <= stroke * 2.0 + 6.0 + tol
            }
            Annotation::Shape { kind, bounds, points, fill, stroke, .. } => {
                let lim = stroke * 0.5 + tol + 2.0;
                match kind {
                    ShapeKind::Rectangle => {
                        if fill.a > 0 {
                            bounds.inflated(lim).contains(local)
                        } else {
                            bounds.inflated(lim).contains(local)
                                && !bounds.inflated(-lim).contains(local)
                        }
                    }
                    ShapeKind::Ellipse => {
                        let c = bounds.center();
                        let rx = (bounds.w * 0.5).max(1.0);
                        let ry = (bounds.h * 0.5).max(1.0);
                        let dx = (local.x - c.x) / (rx + lim);
                        let dy = (local.y - c.y) / (ry + lim);
                        let outer = dx * dx + dy * dy <= 1.0;
                        if fill.a > 0 || rx <= lim || ry <= lim {
                            outer
                        } else {
                            let ix = (local.x - c.x) / (rx - lim);
                            let iy = (local.y - c.y) / (ry - lim);
                            outer && ix * ix + iy * iy >= 1.0
                        }
                    }
                    ShapeKind::Polygon => {
                        let edge = (0..points.len()).any(|i| {
                            dist_to_segment(local, points[i], points[(i + 1) % points.len()]) <= lim
                        });
                        edge || (fill.a > 0 && point_in_polygon(local, points))
                    }
                }
            }
            Annotation::Callout { bounds, tails, .. } => {
                if bounds.inflated(tol).contains(local) {
                    return true;
                }
                tails.iter().any(|t| {
                    let a = Point::new(bounds.x + t.anchor.x * bounds.w, bounds.y + t.anchor.y * bounds.h);
                    dist_to_segment(local, a, t.tip) <= tol + 6.0
                })
            }
            Annotation::Step { center, radius, .. } => p.dist(*center) <= radius + tol,
            Annotation::Magnify { .. } => self.part_at(p, tol).is_some(),
            _ => f.inflated(tol).contains(local),
        }
    }

    /// For annotations with independently draggable parts.
    pub fn part_at(&self, p: Point, tol: f32) -> Option<Part> {
        match self {
            Annotation::Magnify { source, display, .. } => {
                if display.inflated(tol).contains(p) {
                    Some(Part::MagnifyDisplay)
                } else if source.inflated(tol).contains(p) {
                    Some(Part::MagnifySource)
                } else {
                    None
                }
            }
            _ => {
                if self.hit_test(p, tol) {
                    Some(Part::Whole)
                } else {
                    None
                }
            }
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.translate_part(Part::Whole, dx, dy);
    }

    pub fn translate_part(&mut self, part: Part, dx: f32, dy: f32) {
        let mv = |p: &mut Point| {
            p.x += dx;
            p.y += dy;
        };
        match self {
            Annotation::Arrow { start, end, bezier, .. } => {
                mv(start);
                mv(end);
                if let Some((a, b)) = bezier.as_mut() {
                    mv(a);
                    mv(b);
                }
            }
            Annotation::Shape { bounds, points, .. } => {
                *bounds = bounds.translated(dx, dy);
                points.iter_mut().for_each(mv);
            }
            Annotation::Callout { bounds, tails, .. } => {
                *bounds = bounds.translated(dx, dy);
                tails.iter_mut().for_each(|t| mv(&mut t.tip));
            }
            Annotation::Text { bounds, .. }
            | Annotation::Highlighter { bounds, .. }
            | Annotation::Blur { bounds, .. }
            | Annotation::Stamp { bounds, .. }
            | Annotation::Image { bounds, .. } => *bounds = bounds.translated(dx, dy),
            Annotation::Step { center, .. } => mv(center),
            Annotation::Magnify { source, display, .. } => match part {
                Part::MagnifySource => *source = source.translated(dx, dy),
                Part::MagnifyDisplay => *display = display.translated(dx, dy),
                Part::Whole => {
                    *source = source.translated(dx, dy);
                    *display = display.translated(dx, dy);
                }
            },
        }
    }

    /// Resize: map the frame from `old` to `new` (both unrotated frames).
    pub fn set_frame(&mut self, old: Rect, new: Rect) {
        match self {
            Annotation::Arrow { start, end, bezier, .. } => {
                *start = old.map_point(&new, *start);
                *end = old.map_point(&new, *end);
                if let Some((a, b)) = bezier.as_mut() {
                    *a = old.map_point(&new, *a);
                    *b = old.map_point(&new, *b);
                }
            }
            Annotation::Shape { bounds, points, .. } => {
                *bounds = new;
                for p in points.iter_mut() {
                    *p = old.map_point(&new, *p);
                }
            }
            Annotation::Callout { bounds, tails, .. } => {
                // Tips stay where they point unless the whole thing moves; the
                // anchors are relative so they follow the body automatically.
                let _ = tails;
                *bounds = new;
            }
            Annotation::Text { bounds, .. }
            | Annotation::Highlighter { bounds, .. }
            | Annotation::Blur { bounds, .. }
            | Annotation::Stamp { bounds, .. }
            | Annotation::Image { bounds, .. } => *bounds = new,
            Annotation::Step { center, radius, .. } => {
                *center = new.center();
                *radius = (new.w.min(new.h) * 0.5).max(6.0);
            }
            Annotation::Magnify { display, .. } => *display = new,
        }
    }

    /// Kind-specific draggable points (beyond the 8 frame handles), in
    /// document coordinates.
    pub fn edit_points(&self) -> Vec<Point> {
        match self {
            Annotation::Arrow { start, end, bezier, .. } => {
                let mut v = vec![*start, *end];
                if let Some((a, b)) = bezier {
                    v.push(*a);
                    v.push(*b);
                }
                v
            }
            Annotation::Shape { kind: ShapeKind::Polygon, points, bounds, rotation, .. } => {
                let c = bounds.center();
                points.iter().map(|p| p.rotated(c, *rotation)).collect()
            }
            Annotation::Callout { bounds, tails, rotation, .. } => {
                let c = bounds.center();
                tails.iter().map(|t| t.tip.rotated(c, *rotation)).collect()
            }
            Annotation::Magnify { source, .. } => source.corners().to_vec(),
            _ => Vec::new(),
        }
    }

    pub fn set_edit_point(&mut self, idx: usize, p: Point) {
        match self {
            Annotation::Arrow { start, end, bezier, .. } => match idx {
                0 => *start = p,
                1 => *end = p,
                2 | 3 => {
                    if let Some((a, b)) = bezier.as_mut() {
                        if idx == 2 {
                            *a = p
                        } else {
                            *b = p
                        }
                    }
                }
                _ => {}
            },
            Annotation::Shape { kind: ShapeKind::Polygon, points, bounds, rotation, .. } => {
                if idx < points.len() {
                    // Bake any rotation into the vertices first so moving one
                    // vertex doesn't shift the others when the frame changes.
                    if *rotation != 0.0 {
                        let c = bounds.center();
                        for q in points.iter_mut() {
                            *q = q.rotated(c, *rotation);
                        }
                        *rotation = 0.0;
                    }
                    points[idx] = p;
                    *bounds = Rect::bounding(points);
                }
            }
            Annotation::Callout { bounds, tails, rotation, .. } => {
                if let Some(t) = tails.get_mut(idx) {
                    t.tip = p.rotated(bounds.center(), -*rotation);
                }
            }
            Annotation::Magnify { source, .. } => {
                // Drag a corner with the opposite corner fixed.
                let opp = source.corners()[(idx + 2) % 4];
                let r = Rect::from_points(opp, p);
                if r.w >= 4.0 && r.h >= 4.0 {
                    *source = r;
                }
            }
            _ => {}
        }
    }
}

pub fn dist_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-6 {
        return p.dist(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.dist(Point::new(a.x + dx * t, a.y + dy * t))
}

pub fn point_in_polygon(p: Point, poly: &[Point]) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Flatten an arrow shaft into a polyline (straight or cubic bezier).
pub fn arrow_polyline(start: Point, end: Point, bezier: Option<(Point, Point)>) -> Vec<Point> {
    match bezier {
        None => vec![start, end],
        Some((c1, c2)) => (0..=24)
            .map(|i| {
                let t = i as f32 / 24.0;
                let u = 1.0 - t;
                let a = u * u * u;
                let b = 3.0 * u * u * t;
                let c = 3.0 * u * t * t;
                let d = t * t * t;
                Point::new(
                    a * start.x + b * c1.x + c * c2.x + d * end.x,
                    a * start.y + b * c1.y + c * c2.y + d * end.y,
                )
            })
            .collect(),
    }
}

/// Default control points for a curved arrow: a gentle S-free arc.
pub fn default_bezier(start: Point, end: Point) -> (Point, Point) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let nx = -dy / len;
    let ny = dx / len;
    let bend = (len * 0.3).min(120.0);
    (
        Point::new(start.x + dx / 3.0 + nx * bend, start.y + dy / 3.0 + ny * bend),
        Point::new(start.x + dx * 2.0 / 3.0 + nx * bend, start.y + dy * 2.0 / 3.0 + ny * bend),
    )
}

/// Step label for the n-th marker (1-based).
pub fn step_label(n: u32, letters: bool) -> String {
    if !letters {
        return n.to_string();
    }
    // 1 → A … 26 → Z, 27 → AA …
    let mut n = n.max(1);
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        s.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

#[derive(Clone)]
pub struct Document {
    pub base: Rc<Bitmap>,
    /// Bumped whenever `base` is replaced so renderers know to re-upload.
    pub base_gen: u32,
    pub annotations: Vec<Annotation>,
    pub next_id: AnnotationId,
    pub next_step_value: u32,
    /// Group membership: annotation id → group id. Absent = ungrouped.
    pub groups: HashMap<AnnotationId, u32>,
}

impl Document {
    pub fn new(base: Bitmap) -> Self {
        Self {
            base: Rc::new(base),
            base_gen: 0,
            annotations: Vec::new(),
            next_id: 1,
            next_step_value: 1,
            groups: HashMap::new(),
        }
    }

    pub fn alloc_id(&mut self) -> AnnotationId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn set_base(&mut self, base: Rc<Bitmap>) {
        self.base = base;
        self.base_gen = self.base_gen.wrapping_add(1);
    }

    pub fn push(&mut self, a: Annotation) {
        self.annotations.push(a);
    }

    pub fn find_index(&self, id: AnnotationId) -> Option<usize> {
        self.annotations.iter().position(|a| a.id() == id)
    }

    pub fn get(&self, id: AnnotationId) -> Option<&Annotation> {
        self.annotations.iter().find(|a| a.id() == id)
    }

    pub fn topmost_hit(&self, p: Point, tol: f32) -> Option<AnnotationId> {
        for a in self.annotations.iter().rev() {
            if a.hit_test(p, tol) {
                return Some(a.id());
            }
        }
        None
    }

    /// Expand a set of ids to include every member of their groups.
    pub fn expand_groups(&self, ids: &[AnnotationId]) -> Vec<AnnotationId> {
        let mut out: Vec<AnnotationId> = Vec::new();
        for id in ids {
            match self.groups.get(id) {
                Some(g) => {
                    for a in &self.annotations {
                        if self.groups.get(&a.id()) == Some(g) && !out.contains(&a.id()) {
                            out.push(a.id());
                        }
                    }
                }
                None => {
                    if !out.contains(id) {
                        out.push(*id);
                    }
                }
            }
        }
        out
    }

    pub fn canvas_rect(&self) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.base.width as f32, h: self.base.height as f32 }
    }
}
