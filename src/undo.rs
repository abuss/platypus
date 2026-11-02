use ropey::Rope;

/// One undoable edit: rope[at..at+removed.len_chars()] replaced by `inserted`.
/// Offsets are pre-state char offsets.
#[derive(Debug, Clone)]
pub struct Change {
    pub at: usize,
    pub inserted: String,
    pub removed: String,
}

impl Change {
    pub fn apply(&self, rope: &mut Rope) {
        // offsets are pre-state: remove first, then insert
        if !self.removed.is_empty() {
            rope.remove(self.at..self.at + self.removed.chars().count());
        }
        if !self.inserted.is_empty() {
            rope.insert(self.at, &self.inserted);
        }
    }

    pub fn invert(&self) -> Change {
        Change { at: self.at, inserted: self.removed.clone(), removed: self.inserted.clone() }
    }
}

pub struct History {
    undo: Vec<Change>,
    redo: Vec<Change>,
}

impl History {
    pub fn new() -> History {
        History { undo: Vec::new(), redo: Vec::new() }
    }

    pub fn push(&mut self, c: Change) {
        self.undo.push(c);
        self.redo.clear();
    }

    pub fn undo(&mut self, rope: &mut Rope) -> Option<Change> {
        let c = self.undo.pop()?;
        let inv = c.invert();
        inv.apply(rope);
        self.redo.push(c);
        Some(inv)
    }

    pub fn redo(&mut self, rope: &mut Rope) -> Option<Change> {
        let c = self.redo.pop()?;
        c.apply(rope);
        self.undo.push(c.clone());
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_apply_invert_round_trip() {
        let mut rope = Rope::from_str("hello");
        let c = Change { at: 5, inserted: " world".into(), removed: String::new() };
        c.apply(&mut rope);
        assert_eq!(rope.to_string(), "hello world");
        c.invert().apply(&mut rope);
        assert_eq!(rope.to_string(), "hello");
    }

    #[test]
    fn change_replacement_applies_and_inverts() {
        let mut rope = Rope::from_str("foo(bar)");
        let c = Change { at: 4, inserted: "baz".into(), removed: "bar".into() };
        c.apply(&mut rope);
        assert_eq!(rope.to_string(), "foo(baz)");
        c.invert().apply(&mut rope);
        assert_eq!(rope.to_string(), "foo(bar)");
    }

    #[test]
    fn history_undo_redo_sequence() {
        let mut rope = Rope::from_str("a");
        let mut h = History::new();
        // the editor applies to the rope, then records:
        rope.insert(1, "b");
        h.push(Change { at: 1, inserted: "b".into(), removed: String::new() });
        rope.insert(2, "c");
        h.push(Change { at: 2, inserted: "c".into(), removed: String::new() });
        assert_eq!(rope.to_string(), "abc");
        h.undo(&mut rope);
        assert_eq!(rope.to_string(), "ab");
        h.undo(&mut rope);
        assert_eq!(rope.to_string(), "a");
        assert!(h.undo(&mut rope).is_none());
        h.redo(&mut rope);
        assert_eq!(rope.to_string(), "ab");
    }

    #[test]
    fn push_clears_redo_stack() {
        let mut rope = Rope::from_str("abc");
        let mut h = History::new();
        // deletion of "c" already applied to the rope by the editor:
        rope.remove(2..3);
        h.push(Change { at: 2, inserted: String::new(), removed: "c".into() });
        h.undo(&mut rope);
        assert_eq!(rope.to_string(), "abc");
        // new edit branch:
        rope.remove(0..1);
        h.push(Change { at: 0, inserted: String::new(), removed: "a".into() });
        assert!(h.redo(&mut rope).is_none(), "push must clear the redo stack");
    }
}
