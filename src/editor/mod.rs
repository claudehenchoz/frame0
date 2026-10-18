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

pub mod annotation;
pub mod canvas;
pub mod history;
pub mod layout;
pub mod pixels;
pub mod property;
pub mod render;
pub mod stamps;
pub mod text_edit;
pub mod toolbar;
pub mod tools;
pub mod window;

pub use annotation::{Annotation, AnnotationId, BlurMode, Color, Document, Point, Rect, ShapeKind};
pub use window::EditorWindow;
