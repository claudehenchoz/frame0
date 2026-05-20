use crate::gfx::bitmap::Bitmap;

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
}

pub type AnnotationId = u32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlurMode {
    Solid,
    Blur,
}

#[derive(Clone, Debug)]
pub enum Annotation {
    Arrow {
        id: AnnotationId,
        start: Point,
        end: Point,
        color: Color,
        stroke: f32,
    },
    Shape {
        id: AnnotationId,
        kind: ShapeKind,
        bounds: Rect,
        outline: Color,
        fill: Color, // alpha=0 means transparent
        stroke: f32,
        rotation: f32,
    },
    Text {
        id: AnnotationId,
        bounds: Rect,
        text: String,
        color: Color,
        size: f32,
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
}

impl Annotation {
    pub fn id(&self) -> AnnotationId {
        match self {
            Annotation::Arrow { id, .. }
            | Annotation::Shape { id, .. }
            | Annotation::Text { id, .. }
            | Annotation::Step { id, .. }
            | Annotation::Highlighter { id, .. }
            | Annotation::Blur { id, .. } => *id,
        }
    }

    pub fn bounding_box(&self) -> Rect {
        match self {
            Annotation::Arrow { start, end, stroke, .. } => {
                let r = Rect::from_points(*start, *end);
                r.inflated(stroke.max(8.0))
            }
            Annotation::Shape { bounds, .. } => *bounds,
            Annotation::Text { bounds, .. } => *bounds,
            Annotation::Step { center, radius, .. } => Rect {
                x: center.x - radius,
                y: center.y - radius,
                w: radius * 2.0,
                h: radius * 2.0,
            },
            Annotation::Highlighter { bounds, .. } => *bounds,
            Annotation::Blur { bounds, .. } => *bounds,
        }
    }

    pub fn hit_test(&self, p: Point) -> bool {
        self.bounding_box().inflated(4.0).contains(p)
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Annotation::Arrow { start, end, .. } => {
                start.x += dx;
                start.y += dy;
                end.x += dx;
                end.y += dy;
            }
            Annotation::Shape { bounds, .. }
            | Annotation::Text { bounds, .. }
            | Annotation::Highlighter { bounds, .. }
            | Annotation::Blur { bounds, .. } => {
                bounds.x += dx;
                bounds.y += dy;
            }
            Annotation::Step { center, .. } => {
                center.x += dx;
                center.y += dy;
            }
        }
    }
}

#[derive(Clone)]
pub struct Document {
    pub base: Bitmap,
    pub annotations: Vec<Annotation>,
    pub next_id: AnnotationId,
    pub next_step_value: u32,
}

impl Document {
    pub fn new(base: Bitmap) -> Self {
        Self {
            base,
            annotations: Vec::new(),
            next_id: 1,
            next_step_value: 1,
        }
    }

    pub fn alloc_id(&mut self) -> AnnotationId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn push(&mut self, a: Annotation) {
        self.annotations.push(a);
    }

    pub fn find_index(&self, id: AnnotationId) -> Option<usize> {
        self.annotations.iter().position(|a| a.id() == id)
    }

    pub fn topmost_hit(&self, p: Point) -> Option<AnnotationId> {
        for a in self.annotations.iter().rev() {
            if a.hit_test(p) {
                return Some(a.id());
            }
        }
        None
    }
}
