use crate::editor::annotation::{Annotation, AnnotationId, Document};

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
        self.undo.push(cmd);
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

fn apply_forward(doc: &mut Document, cmd: &Command) {
    match cmd {
        Command::Insert { index, ann } => doc.annotations.insert(*index, ann.clone()),
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
    }
}

fn apply_reverse(doc: &mut Document, cmd: &Command) {
    match cmd {
        Command::Insert { index, .. } => {
            if *index < doc.annotations.len() {
                doc.annotations.remove(*index);
            }
        }
        Command::Remove { index, ann } => doc.annotations.insert(*index, ann.clone()),
        Command::Mutate { id, before, .. } => {
            if let Some(i) = doc.find_index(*id) {
                doc.annotations[i] = before.clone();
            }
        }
    }
}
