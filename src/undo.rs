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
        Change {
            at: self.at,
            inserted: self.removed.clone(),
            removed: self.inserted.clone(),
        }
    }
}

/// Undo steps are batches: a single edit is a one-element batch, replace-all
/// records many changes as one step. Batches are stored in the order they were
/// applied (replace-all: descending offsets); undo applies inverses in reverse.
pub struct History {
    undo: Vec<Vec<Change>>,
    redo: Vec<Vec<Change>>,
}

impl History {
    pub fn new() -> History {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn push(&mut self, c: Change) {
        self.undo.push(vec![c]);
        self.redo.clear();
    }

    pub fn push_batch(&mut self, batch: Vec<Change>) {
        if batch.is_empty() {
            return;
        }
        self.undo.push(batch);
        self.redo.clear();
    }

    /// Undo one step; returns a suggested cursor offset (start of the earliest
    /// undone change).
    pub fn undo(&mut self, rope: &mut Rope) -> Option<usize> {
        let batch = self.undo.pop()?;
        let mut earliest = usize::MAX;
        for c in batch.iter().rev() {
            let inv = c.invert();
            inv.apply(rope);
            earliest = earliest.min(c.at);
        }
        self.redo.push(batch);
        Some(earliest)
    }

    /// Pop the top undo batch without applying it, for callers that need to
    /// apply changes manually (e.g. to capture byte-offset PendingEdits).
    /// Caller must call `push_redo_batch` to keep redo working.
    pub fn pop_undo_batch(&mut self) -> Option<Vec<Change>> {
        self.undo.pop()
    }

    /// Push a batch onto the redo stack (companion to `pop_undo_batch`).
    pub fn push_redo_batch(&mut self, batch: Vec<Change>) {
        self.redo.push(batch);
    }

    /// Pop the top redo batch without applying it.
    /// Caller must call `push_undo_batch` to keep undo working.
    pub fn pop_redo_batch(&mut self) -> Option<Vec<Change>> {
        self.redo.pop()
    }

    /// Push a batch onto the undo stack (companion to `pop_redo_batch`).
    pub fn push_undo_batch(&mut self, batch: Vec<Change>) {
        self.undo.push(batch);
    }

    /// Redo one step; returns a suggested cursor offset (end of the last change).
    pub fn redo(&mut self, rope: &mut Rope) -> Option<usize> {
        let batch = self.redo.pop()?;
        let mut end = 0;
        for c in &batch {
            c.apply(rope);
            end = end.max(c.at + c.inserted.chars().count());
        }
        self.undo.push(batch);
        Some(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_apply_invert_round_trip() {
        let mut rope = Rope::from_str("hello");
        let c = Change {
            at: 5,
            inserted: " world".into(),
            removed: String::new(),
        };
        c.apply(&mut rope);
        assert_eq!(rope.to_string(), "hello world");
        c.invert().apply(&mut rope);
        assert_eq!(rope.to_string(), "hello");
    }

    #[test]
    fn change_replacement_applies_and_inverts() {
        let mut rope = Rope::from_str("foo(bar)");
        let c = Change {
            at: 4,
            inserted: "baz".into(),
            removed: "bar".into(),
        };
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
        h.push(Change {
            at: 1,
            inserted: "b".into(),
            removed: String::new(),
        });
        rope.insert(2, "c");
        h.push(Change {
            at: 2,
            inserted: "c".into(),
            removed: String::new(),
        });
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
    fn push_batch_is_a_single_history_step() {
        // replace-all stores changes in DESCENDING offset order (pre-state offsets:
        // editing high offsets first keeps lower ones valid)
        let mut rope = Rope::from_str("aa aa");
        let mut h = History::new();
        // applied high-to-low, then recorded in that same order
        rope.remove(3..5);
        rope.insert(3, "b");
        rope.remove(0..2);
        rope.insert(0, "b");
        h.push_batch(vec![
            Change {
                at: 3,
                inserted: "b".into(),
                removed: "aa".into(),
            },
            Change {
                at: 0,
                inserted: "b".into(),
                removed: "aa".into(),
            },
        ]);
        assert_eq!(rope.to_string(), "b b");
        assert!(h.undo(&mut rope).is_some());
        assert_eq!(
            rope.to_string(),
            "aa aa",
            "one Ctrl-Z must undo the whole batch"
        );
        assert!(h.undo(&mut rope).is_none(), "the batch was one step");
        // redo re-applies stored (descending) order: offsets stay valid
        assert!(h.redo(&mut rope).is_some());
        assert_eq!(rope.to_string(), "b b");
    }

    #[test]
    fn push_clears_redo_stack() {
        let mut rope = Rope::from_str("abc");
        let mut h = History::new();
        // deletion of "c" already applied to the rope by the editor:
        rope.remove(2..3);
        h.push(Change {
            at: 2,
            inserted: String::new(),
            removed: "c".into(),
        });
        h.undo(&mut rope);
        assert_eq!(rope.to_string(), "abc");
        // new edit branch:
        rope.remove(0..1);
        h.push(Change {
            at: 0,
            inserted: String::new(),
            removed: "a".into(),
        });
        assert!(
            h.redo(&mut rope).is_none(),
            "push must clear the redo stack"
        );
    }
}
