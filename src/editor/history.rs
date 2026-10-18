use crate::editor::annotation::{Annotation, AnnotationId, Document};
use crate::gfx::bitmap::Bitmap;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone)]
pub enum Command {
    Insert {
        index: usize,
        ann: Annotation,
    },
    Remove {
        index: usize,
        ann: Annotation,
    },
    Mutate {
        id: AnnotationId,
        before: Annotation,
        after: Annotation,
    },
    /// Replace the base raster (selection ops, cut-out, flatten).
    Base {
        before: Rc<Bitmap>,
        after: Rc<Bitmap>,
    },
    /// Replace the whole group table (group / ungroup).
    Groups {
        before: HashMap<AnnotationId, u32>,
        after: HashMap<AnnotationId, u32>,
    },
    /// Several commands applied (and undone) as one step.
    Batch(Vec<Command>),
}

pub struct History {
    pub undo: Vec<Command>,
    pub redo: Vec<Command>,
}

impl History {
    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn push(&mut self, cmd: Command) {
        if let Command::Batch(v) = &cmd {
            if v.is_empty() {
                return;
            }
        }
        self.undo.push(cmd);
        self.redo.clear();
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.undo.pop() {
            apply_reverse(doc, &cmd);
            self.redo.push(cmd);
            true
        } else {
            false
        }
    }

    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.redo.pop() {
            apply_forward(doc, &cmd);
            self.undo.push(cmd);
            true
        } else {
            false
        }
    }
}

/// Apply a command that hasn't been applied to `doc` yet, then record it.
pub fn execute(doc: &mut Document, history: &mut History, cmd: Command) {
    apply_forward(doc, &cmd);
    history.push(cmd);
}

pub fn apply_forward(doc: &mut Document, cmd: &Command) {
    match cmd {
        Command::Insert { index, ann } => {
            let i = (*index).min(doc.annotations.len());
            doc.annotations.insert(i, ann.clone())
        }
        Command::Remove { index, .. } => {
            if *index < doc.annotations.len() {
                doc.annotations.remove(*index);
            }
        }
        Command::Mutate { id, after, .. } => {
            if let Some(i) = doc.find_index(*id) {
                doc.annotations[i] = after.clone();
            }
        }
        Command::Base { after, .. } => doc.set_base(after.clone()),
        Command::Groups { after, .. } => doc.groups = after.clone(),
        Command::Batch(v) => v.iter().for_each(|c| apply_forward(doc, c)),
    }
}

pub fn apply_reverse(doc: &mut Document, cmd: &Command) {
    match cmd {
        Command::Insert { index, .. } => {
            if *index < doc.annotations.len() {
                doc.annotations.remove(*index);
            }
        }
        Command::Remove { index, ann } => {
            let i = (*index).min(doc.annotations.len());
            doc.annotations.insert(i, ann.clone())
        }
        Command::Mutate { id, before, .. } => {
            if let Some(i) = doc.find_index(*id) {
                doc.annotations[i] = before.clone();
            }
        }
        Command::Base { before, .. } => doc.set_base(before.clone()),
        Command::Groups { before, .. } => doc.groups = before.clone(),
        Command::Batch(v) => v.iter().rev().for_each(|c| apply_reverse(doc, c)),
    }
}

/// Build a Remove batch for `ids` (highest index first so indices stay valid
/// when replayed forward, and reinsert in ascending order on undo).
pub fn remove_many(doc: &Document, ids: &[AnnotationId]) -> Command {
    let mut idx: Vec<usize> = ids.iter().filter_map(|id| doc.find_index(*id)).collect();
    idx.sort_unstable();
    idx.dedup();
    Command::Batch(
        idx.iter()
            .rev()
            .map(|&i| Command::Remove { index: i, ann: doc.annotations[i].clone() })
            .collect(),
    )
}
