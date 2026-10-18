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

//! `.frame0` project format. Little-endian binary layout:
//!
//! ```text
//! 0..4    magic "FRM0"
//! 4..6    u16 version (1)
//! 6..10   u32 base_png_len
//! 10..    base PNG bytes (encoded via GDI+; full base image)
//! ...     u32 next_id
//! ...     u32 next_step_value
//! ...     u32 annotation_count
//! ...     annotation records, each: u8 tag + variant-specific payload
//! ...     u32 group_count, then (u32 annotation id, u32 group id) pairs
//! ```

use crate::editor::annotation::{Annotation, BlurMode, Document, EndStyle, Point, Rect, ShapeKind, Tail};
use crate::error::{Error, Result};
use crate::gfx::color::Color;
use std::fs::File;
use std::io::{Read, Write};
use std::rc::Rc;

const MAGIC: &[u8; 4] = b"FRM0";
const VERSION: u16 = 1;

const TAG_ARROW: u8 = 1;
const TAG_SHAPE: u8 = 2;
const TAG_TEXT: u8 = 3;
const TAG_STEP: u8 = 4;
const TAG_HIGHLIGHT: u8 = 5;
const TAG_BLUR: u8 = 6;
const TAG_MAGNIFY: u8 = 7;
const TAG_STAMP: u8 = 8;
const TAG_CALLOUT: u8 = 9;
const TAG_IMAGE: u8 = 10;

pub fn save(doc: &Document, path: &str) -> Result<()> {
    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());

    let png = crate::gfx::gdiplus::encode_png_bytes(&doc.base)?;
    write_bytes(&mut buf, &png);

    buf.extend_from_slice(&doc.next_id.to_le_bytes());
    buf.extend_from_slice(&doc.next_step_value.to_le_bytes());
    buf.extend_from_slice(&(doc.annotations.len() as u32).to_le_bytes());
    for a in &doc.annotations {
        write_ann(&mut buf, a)?;
    }
    buf.extend_from_slice(&(doc.groups.len() as u32).to_le_bytes());
    for (id, g) in &doc.groups {
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&g.to_le_bytes());
    }

    let mut f = File::create(path).map_err(|_| Error::msg("project: create failed"))?;
    f.write_all(&buf)
        .map_err(|_| Error::msg("project: write failed"))?;
    Ok(())
}

pub fn load(path: &str) -> Result<Document> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| Error::msg("project: open failed"))?
        .read_to_end(&mut bytes)
        .map_err(|_| Error::msg("project: read failed"))?;
    let mut r = Reader::new(&bytes);
    if r.take(4)? != MAGIC {
        return Err(Error::msg("not a .frame0 file"));
    }
    if r.u16()? != VERSION {
        return Err(Error::msg("unsupported project version"));
    }
    let png_len = r.u32()? as usize;
    let png_bytes = r.take(png_len)?.to_vec();
    let base = crate::gfx::gdiplus::decode_bytes(&png_bytes)?;
    let next_id = r.u32()?;
    let next_step_value = r.u32()?;
    let count = r.u32()? as usize;
    let mut doc = Document::new(base);
    doc.next_id = next_id;
    doc.next_step_value = next_step_value;
    for _ in 0..count {
        doc.annotations.push(read_ann(&mut r)?);
    }
    let n = r.u32()?;
    for _ in 0..n {
        let id = r.u32()?;
        let g = r.u32()?;
        doc.groups.insert(id, g);
    }
    // Guard against ids colliding with ones already in the file.
    let max_id = doc.annotations.iter().map(|a| a.id()).max().unwrap_or(0);
    doc.next_id = doc.next_id.max(max_id + 1);
    Ok(doc)
}

fn write_ann(buf: &mut Vec<u8>, a: &Annotation) -> Result<()> {
    match a {
        Annotation::Arrow { id, start, end, color, stroke, bezier, head, tail, shadow } => {
            buf.push(TAG_ARROW);
            buf.extend_from_slice(&id.to_le_bytes());
            write_point(buf, *start);
            write_point(buf, *end);
            write_color(buf, *color);
            buf.extend_from_slice(&stroke.to_le_bytes());
            match bezier {
                Some((c1, c2)) => {
                    buf.push(1);
                    write_point(buf, *c1);
                    write_point(buf, *c2);
                }
                None => buf.push(0),
            }
            buf.push(head.to_u8());
            buf.push(tail.to_u8());
            buf.push(*shadow as u8);
        }
        Annotation::Shape { id, kind, bounds, points, outline, fill, stroke, rotation, shadow } => {
            buf.push(TAG_SHAPE);
            buf.extend_from_slice(&id.to_le_bytes());
            buf.push(match kind {
                ShapeKind::Rectangle => 0,
                ShapeKind::Ellipse => 1,
                ShapeKind::Polygon => 2,
            });
            write_rect(buf, *bounds);
            write_color(buf, *outline);
            write_color(buf, *fill);
            buf.extend_from_slice(&stroke.to_le_bytes());
            buf.extend_from_slice(&rotation.to_le_bytes());
            buf.push(*shadow as u8);
            buf.extend_from_slice(&(points.len() as u32).to_le_bytes());
            for p in points {
                write_point(buf, *p);
            }
        }
        Annotation::Callout { id, bounds, text, fill, outline, text_color, size, stroke, tails, rotation, radius, shadow } => {
            buf.push(TAG_CALLOUT);
            buf.extend_from_slice(&id.to_le_bytes());
            write_rect(buf, *bounds);
            write_str(buf, text);
            write_color(buf, *fill);
            write_color(buf, *outline);
            write_color(buf, *text_color);
            buf.extend_from_slice(&size.to_le_bytes());
            buf.extend_from_slice(&stroke.to_le_bytes());
            buf.extend_from_slice(&(tails.len() as u32).to_le_bytes());
            for t in tails {
                write_point(buf, t.anchor);
                write_point(buf, t.tip);
            }
            buf.extend_from_slice(&rotation.to_le_bytes());
            buf.extend_from_slice(&radius.to_le_bytes());
            buf.push(*shadow as u8);
        }
        Annotation::Text { id, bounds, text, color, size, rotation } => {
            buf.push(TAG_TEXT);
            buf.extend_from_slice(&id.to_le_bytes());
            write_rect(buf, *bounds);
            write_str(buf, text);
            write_color(buf, *color);
            buf.extend_from_slice(&size.to_le_bytes());
            buf.extend_from_slice(&rotation.to_le_bytes());
        }
        Annotation::Step { id, center, radius, value, fill, text } => {
            buf.push(TAG_STEP);
            buf.extend_from_slice(&id.to_le_bytes());
            write_point(buf, *center);
            buf.extend_from_slice(&radius.to_le_bytes());
            write_str(buf, value);
            write_color(buf, *fill);
            write_color(buf, *text);
        }
        Annotation::Highlighter { id, bounds, color } => {
            buf.push(TAG_HIGHLIGHT);
            buf.extend_from_slice(&id.to_le_bytes());
            write_rect(buf, *bounds);
            write_color(buf, *color);
        }
        Annotation::Blur { id, bounds, mode, intensity, fill } => {
            buf.push(TAG_BLUR);
            buf.extend_from_slice(&id.to_le_bytes());
            write_rect(buf, *bounds);
            buf.push(match mode {
                BlurMode::Solid => 0,
                BlurMode::Blur => 1,
                BlurMode::Pixelate => 2,
            });
            buf.extend_from_slice(&intensity.to_le_bytes());
            write_color(buf, *fill);
        }
        Annotation::Magnify { id, source, display, line_width, color } => {
            buf.push(TAG_MAGNIFY);
            buf.extend_from_slice(&id.to_le_bytes());
            write_rect(buf, *source);
            write_rect(buf, *display);
            buf.extend_from_slice(&line_width.to_le_bytes());
            write_color(buf, *color);
        }
        Annotation::Stamp { id, asset, bounds, rotation } => {
            buf.push(TAG_STAMP);
            buf.extend_from_slice(&id.to_le_bytes());
            write_str(buf, asset);
            write_rect(buf, *bounds);
            buf.extend_from_slice(&rotation.to_le_bytes());
            // Embed user stamp pixels so the project opens anywhere.
            let png = match crate::editor::stamps::file_bitmap(asset) {
                Some(b) if asset.starts_with("f:") => crate::gfx::gdiplus::encode_png_bytes(&b)?,
                _ => Vec::new(),
            };
            write_bytes(buf, &png);
        }
        Annotation::Image { id, bounds, bitmap, rotation } => {
            buf.push(TAG_IMAGE);
            buf.extend_from_slice(&id.to_le_bytes());
            write_rect(buf, *bounds);
            buf.extend_from_slice(&rotation.to_le_bytes());
            write_bytes(buf, &crate::gfx::gdiplus::encode_png_bytes(bitmap)?);
        }
    }
    Ok(())
}

fn read_ann(r: &mut Reader) -> Result<Annotation> {
    let tag = r.u8()?;
    let id = r.u32()?;
    Ok(match tag {
        TAG_ARROW => {
            let start = read_point(r)?;
            let end = read_point(r)?;
            let color = read_color(r)?;
            let stroke = r.f32()?;
            let bezier = if r.u8()? == 1 { Some((read_point(r)?, read_point(r)?)) } else { None };
            let head = EndStyle::from_u8(r.u8()?);
            let tail = EndStyle::from_u8(r.u8()?);
            let shadow = r.u8()? != 0;
            Annotation::Arrow { id, start, end, color, stroke, bezier, head, tail, shadow }
        }
        TAG_SHAPE => {
            let kind = match r.u8()? {
                0 => ShapeKind::Rectangle,
                1 => ShapeKind::Ellipse,
                _ => ShapeKind::Polygon,
            };
            let bounds = read_rect(r)?;
            let outline = read_color(r)?;
            let fill = read_color(r)?;
            let stroke = r.f32()?;
            let rotation = r.f32()?;
            let shadow = r.u8()? != 0;
            let n = r.u32()? as usize;
            let mut points = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                points.push(read_point(r)?);
            }
            Annotation::Shape { id, kind, bounds, points, outline, fill, stroke, rotation, shadow }
        }
        TAG_CALLOUT => {
            let bounds = read_rect(r)?;
            let text = read_str(r)?;
            let fill = read_color(r)?;
            let outline = read_color(r)?;
            let text_color = read_color(r)?;
            let size = r.f32()?;
            let stroke = r.f32()?;
            let n = r.u32()? as usize;
            let mut tails = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                tails.push(Tail { anchor: read_point(r)?, tip: read_point(r)? });
            }
            let rotation = r.f32()?;
            let radius = r.f32()?;
            let shadow = r.u8()? != 0;
            Annotation::Callout { id, bounds, text, fill, outline, text_color, size, stroke, tails, rotation, radius, shadow }
        }
        TAG_TEXT => Annotation::Text {
            id,
            bounds: read_rect(r)?,
            text: read_str(r)?,
            color: read_color(r)?,
            size: r.f32()?,
            rotation: r.f32()?,
        },
        TAG_STEP => Annotation::Step {
            id,
            center: read_point(r)?,
            radius: r.f32()?,
            value: read_str(r)?,
            fill: read_color(r)?,
            text: read_color(r)?,
        },
        TAG_HIGHLIGHT => Annotation::Highlighter {
            id,
            bounds: read_rect(r)?,
            color: read_color(r)?,
        },
        TAG_BLUR => {
            let bounds = read_rect(r)?;
            let mode = match r.u8()? {
                0 => BlurMode::Solid,
                2 => BlurMode::Pixelate,
                _ => BlurMode::Blur,
            };
            let intensity = r.f32()?;
            let fill = read_color(r)?;
            Annotation::Blur { id, bounds, mode, intensity, fill }
        }
        TAG_MAGNIFY => Annotation::Magnify {
            id,
            source: read_rect(r)?,
            display: read_rect(r)?,
            line_width: r.f32()?,
            color: read_color(r)?,
        },
        TAG_STAMP => {
            let asset = read_str(r)?;
            let bounds = read_rect(r)?;
            let rotation = r.f32()?;
            let n = r.u32()? as usize;
            let png = r.take(n)?;
            if !png.is_empty() {
                if let Ok(b) = crate::gfx::gdiplus::decode_bytes(png) {
                    crate::editor::stamps::register(&asset, Rc::new(b));
                }
            }
            Annotation::Stamp { id, asset, bounds, rotation }
        }
        TAG_IMAGE => {
            let bounds = read_rect(r)?;
            let rotation = r.f32()?;
            let n = r.u32()? as usize;
            let bitmap = Rc::new(crate::gfx::gdiplus::decode_bytes(r.take(n)?)?);
            Annotation::Image { id, bounds, bitmap, rotation }
        }
        _ => return Err(Error::msg("unknown annotation tag")),
    })
}

fn write_bytes(buf: &mut Vec<u8>, b: &[u8]) {
    buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
    buf.extend_from_slice(b);
}

fn write_point(buf: &mut Vec<u8>, p: Point) {
    buf.extend_from_slice(&p.x.to_le_bytes());
    buf.extend_from_slice(&p.y.to_le_bytes());
}
fn read_point(r: &mut Reader) -> Result<Point> {
    Ok(Point::new(r.f32()?, r.f32()?))
}
fn write_rect(buf: &mut Vec<u8>, r: Rect) {
    buf.extend_from_slice(&r.x.to_le_bytes());
    buf.extend_from_slice(&r.y.to_le_bytes());
    buf.extend_from_slice(&r.w.to_le_bytes());
    buf.extend_from_slice(&r.h.to_le_bytes());
}
fn read_rect(r: &mut Reader) -> Result<Rect> {
    Ok(Rect {
        x: r.f32()?,
        y: r.f32()?,
        w: r.f32()?,
        h: r.f32()?,
    })
}
fn write_color(buf: &mut Vec<u8>, c: Color) {
    buf.extend_from_slice(&[c.r, c.g, c.b, c.a]);
}
fn read_color(r: &mut Reader) -> Result<Color> {
    Ok(Color {
        r: r.u8()?,
        g: r.u8()?,
        b: r.u8()?,
        a: r.u8()?,
    })
}
fn write_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}
fn read_str(r: &mut Reader) -> Result<String> {
    let n = r.u32()? as usize;
    Ok(String::from_utf8_lossy(r.take(n)?).into_owned())
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.bytes.len() {
            return Err(Error::msg("project: unexpected eof"));
        }
        let r = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(r)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Result<f32> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}
