#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PromptKind {
    SaveAs,
    Find,
    Command,
    ConfirmQuit,
    ConfirmReload,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Choice {
    Save,
    Discard,
    Cancel,
}

impl Choice {
    pub fn cycle(self) -> Choice {
        match self {
            Choice::Save => Choice::Discard,
            Choice::Discard => Choice::Cancel,
            Choice::Cancel => Choice::Save,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Choice::Save => "save",
            Choice::Discard => "discard",
            Choice::Cancel => "cancel",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PromptField {
    Pattern,
    /// Find prompt only: the replacement text (Tab toggles).
    Replace,
}

pub struct Prompt {
    pub kind: PromptKind,
    pub line: String,
    /// Find prompt only: replacement text, edited in the Replace field.
    pub replace: String,
    pub field: PromptField,
    pub history: Vec<String>,
    pub choice: Choice,
}

impl Prompt {
    pub fn new(kind: PromptKind, prefill: String) -> Prompt {
        Prompt { kind, line: prefill, replace: String::new(), field: PromptField::Pattern, history: Vec::new(), choice: Choice::Cancel }
    }

    pub fn push_history(&mut self) {
        if !self.line.is_empty() {
            self.history.push(self.line.clone());
        }
    }

    pub fn history_up(&mut self) {
        if let Some(last) = self.history.pop() {
            self.line = last;
        }
    }

    pub fn history_down(&mut self) {
        // phase 0: history is up-only stack; down re-commits previous position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_cycles() {
        assert_eq!(Choice::Cancel.cycle(), Choice::Save);
        assert_eq!(Choice::Save.cycle(), Choice::Discard);
        assert_eq!(Choice::Discard.cycle(), Choice::Cancel);
    }

    #[test]
    fn history_stack_pops_into_line() {
        let mut p = Prompt::new(PromptKind::Find, String::new());
        p.line = "one".into();
        p.push_history();
        p.line = "two".into();
        p.push_history();
        p.line = String::new();
        p.history_up();
        assert_eq!(p.line, "two");
        p.history_up();
        assert_eq!(p.line, "one");
    }
}
