//! The console's input line as pure state: keys go in, instructions come out. No Windows calls.

use crate::command::{self, Candidate, ServerName};

const HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    /// Esc or Ctrl+C.
    Cancel,
}

#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Redraw,
    Submit(String),
    ShowCandidates(Vec<Candidate>),
    /// `?` was pressed, with this line typed so far.
    Help(String),
    /// The console toggle key was pressed.
    Toggle,
}

#[derive(Default)]
pub struct Editor {
    line: String,
    /// A byte offset. Input is limited to ASCII, so it's also the column.
    cursor: usize,
    history: Vec<String>,
    /// The history entry being shown, and the unfinished line to return to.
    browsing: Option<(usize, String)>,
    tab_pending: bool,
}

impl Editor {
    pub fn line(&self) -> &str {
        &self.line
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Handles any key but Tab. `capacity` is how many characters fit on the console line.
    pub fn key(&mut self, key: Key, capacity: usize) -> Action {
        self.tab_pending = false;
        match key {
            Key::Char('`' | '~') => Action::Toggle,
            Key::Char('?') => Action::Help(self.line.clone()),
            Key::Char(c) if (' '..='~').contains(&c) && self.line.len() < capacity => {
                self.line.insert(self.cursor, c);
                self.cursor += 1;
                self.browsing = None;
                Action::Redraw
            }
            Key::Char(_) => Action::None,
            Key::Enter => self.submit(),
            Key::Backspace if self.cursor > 0 => {
                self.cursor -= 1;
                self.line.remove(self.cursor);
                Action::Redraw
            }
            Key::Delete if self.cursor < self.line.len() => {
                self.line.remove(self.cursor);
                Action::Redraw
            }
            Key::Backspace | Key::Delete => Action::None,
            Key::Left => self.move_to(self.cursor.saturating_sub(1)),
            Key::Right => self.move_to((self.cursor + 1).min(self.line.len())),
            Key::Home => self.move_to(0),
            Key::End => self.move_to(self.line.len()),
            Key::Up => self.browse_older(),
            Key::Down => self.browse_newer(),
            Key::Cancel => {
                self.set_line(String::new());
                self.browsing = None;
                Action::Redraw
            }
        }
    }

    /// Completes at the cursor: as far as is certain, then a second Tab lists what's possible.
    pub fn tab(&mut self, servers: &[ServerName]) -> Action {
        let wants_list = std::mem::take(&mut self.tab_pending);
        let completion = command::complete(&self.line, self.cursor, servers);
        match completion.candidates.as_slice() {
            [] => Action::None,
            [only] => {
                let tail_has_space = self.line[self.cursor..].starts_with(' ');
                let spacer = if tail_has_space { "" } else { " " };
                self.replace(completion.start, &format!("{}{spacer}", only.text));
                Action::Redraw
            }
            many => {
                let word = self.line[completion.start..self.cursor].to_lowercase();
                let common = common_prefix(many);
                if common.len() > word.len() && common.starts_with(&word) {
                    self.replace(completion.start, &common);
                    self.tab_pending = true;
                    Action::Redraw
                } else if wants_list {
                    Action::ShowCandidates(completion.candidates)
                } else {
                    self.tab_pending = true;
                    Action::None
                }
            }
        }
    }

    fn submit(&mut self) -> Action {
        let line = self.line.trim().to_string();
        self.set_line(String::new());
        self.browsing = None;
        if line.is_empty() {
            return Action::None;
        }
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > HISTORY_LIMIT {
                self.history.remove(0);
            }
        }
        Action::Submit(line)
    }

    fn browse_older(&mut self) -> Action {
        let index = match &self.browsing {
            Some((0, _)) => return Action::None,
            Some((index, _)) => index - 1,
            None if self.history.is_empty() => return Action::None,
            None => self.history.len() - 1,
        };
        let draft = self
            .browsing
            .take()
            .map_or_else(|| self.line.clone(), |(_, draft)| draft);
        self.set_line(self.history[index].clone());
        self.browsing = Some((index, draft));
        Action::Redraw
    }

    fn browse_newer(&mut self) -> Action {
        let Some((index, draft)) = self.browsing.take() else {
            return Action::None;
        };
        if index + 1 < self.history.len() {
            self.set_line(self.history[index + 1].clone());
            self.browsing = Some((index + 1, draft));
        } else {
            self.set_line(draft);
        }
        Action::Redraw
    }

    fn move_to(&mut self, cursor: usize) -> Action {
        if cursor == self.cursor {
            return Action::None;
        }
        self.cursor = cursor;
        Action::Redraw
    }

    fn set_line(&mut self, line: String) {
        self.cursor = line.len();
        self.line = line;
    }

    /// Replaces the text from `start` up to the cursor.
    fn replace(&mut self, start: usize, text: &str) {
        self.line.replace_range(start..self.cursor, text);
        self.cursor = start + text.len();
    }
}

fn common_prefix(candidates: &[Candidate]) -> String {
    let first = &candidates[0].text;
    let length = candidates
        .iter()
        .map(|candidate| {
            first
                .bytes()
                .zip(candidate.text.bytes())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .min()
        .unwrap_or(0);
    first[..length].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPACITY: usize = 40;

    fn servers() -> Vec<ServerName> {
        [
            ("sto", "Stockholm"),
            ("sto2", "Stockholm 2"),
            ("jnb", "Johannesburg"),
        ]
        .iter()
        .map(|(code, desc)| ((*code).to_string(), (*desc).to_string()))
        .collect()
    }

    fn typed(text: &str) -> Editor {
        let mut editor = Editor::default();
        for c in text.chars() {
            editor.key(Key::Char(c), CAPACITY);
        }
        editor
    }

    #[test]
    fn typing_and_editing() {
        let mut editor = typed("set pck");
        editor.key(Key::Left, CAPACITY);
        editor.key(Key::Left, CAPACITY);
        editor.key(Key::Char('i'), CAPACITY);
        assert_eq!(editor.line(), "set pick");
        assert_eq!(editor.cursor(), 6);
        editor.key(Key::Backspace, CAPACITY);
        assert_eq!(editor.line(), "set pck");
        editor.key(Key::Delete, CAPACITY);
        assert_eq!(editor.line(), "set pk");
        editor.key(Key::Home, CAPACITY);
        assert_eq!(editor.cursor(), 0);
        assert_eq!(editor.key(Key::Backspace, CAPACITY), Action::None);
        assert_eq!(editor.key(Key::Left, CAPACITY), Action::None);
        editor.key(Key::End, CAPACITY);
        assert_eq!(editor.cursor(), 6);
        assert_eq!(editor.key(Key::Right, CAPACITY), Action::None);
        assert_eq!(editor.key(Key::Delete, CAPACITY), Action::None);
    }

    #[test]
    fn input_is_limited_to_capacity_and_printable_ascii() {
        let mut editor = Editor::default();
        for _ in 0..5 {
            editor.key(Key::Char('a'), 3);
        }
        assert_eq!(editor.line(), "aaa");
        assert_eq!(editor.key(Key::Char('\u{e9}'), CAPACITY), Action::None);
        assert_eq!(editor.key(Key::Char('\u{7}'), CAPACITY), Action::None);
    }

    #[test]
    fn enter_submits_trimmed_and_clears() {
        let mut editor = typed("  clear pick ");
        assert_eq!(
            editor.key(Key::Enter, CAPACITY),
            Action::Submit("clear pick".to_string())
        );
        assert_eq!(editor.line(), "");
        assert_eq!(editor.cursor(), 0);
        assert_eq!(editor.key(Key::Enter, CAPACITY), Action::None);
    }

    #[test]
    fn special_keys_leave_the_line_alone() {
        let mut editor = typed("set");
        assert_eq!(
            editor.key(Key::Char('?'), CAPACITY),
            Action::Help("set".to_string())
        );
        assert_eq!(editor.key(Key::Char('`'), CAPACITY), Action::Toggle);
        assert_eq!(editor.key(Key::Char('~'), CAPACITY), Action::Toggle);
        assert_eq!(editor.line(), "set");
    }

    #[test]
    fn cancel_clears_the_line() {
        let mut editor = typed("set pick");
        editor.key(Key::Cancel, CAPACITY);
        assert_eq!(editor.line(), "");
        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn history_browses_and_restores_the_draft() {
        let mut editor = typed("show status");
        editor.key(Key::Enter, CAPACITY);
        for c in "clear pick".chars() {
            editor.key(Key::Char(c), CAPACITY);
        }
        editor.key(Key::Enter, CAPACITY);
        for c in "draft".chars() {
            editor.key(Key::Char(c), CAPACITY);
        }
        editor.key(Key::Up, CAPACITY);
        assert_eq!(editor.line(), "clear pick");
        editor.key(Key::Up, CAPACITY);
        assert_eq!(editor.line(), "show status");
        assert_eq!(editor.key(Key::Up, CAPACITY), Action::None);
        editor.key(Key::Down, CAPACITY);
        assert_eq!(editor.line(), "clear pick");
        editor.key(Key::Down, CAPACITY);
        assert_eq!(editor.line(), "draft");
        assert_eq!(editor.key(Key::Down, CAPACITY), Action::None);
    }

    #[test]
    fn history_skips_immediate_repeats() {
        let mut editor = Editor::default();
        for _ in 0..2 {
            for c in "quit".chars() {
                editor.key(Key::Char(c), CAPACITY);
            }
            editor.key(Key::Enter, CAPACITY);
        }
        editor.key(Key::Up, CAPACITY);
        assert_eq!(editor.key(Key::Up, CAPACITY), Action::None);
    }

    #[test]
    fn tab_finishes_a_certain_word_and_adds_a_space() {
        let mut editor = typed("list se");
        assert_eq!(editor.tab(&servers()), Action::Redraw);
        assert_eq!(editor.line(), "list servers ");
        assert_eq!(editor.cursor(), 13);
    }

    #[test]
    fn tab_replaces_a_name_match_with_the_code() {
        let mut editor = typed("set pick johan");
        editor.tab(&servers());
        assert_eq!(editor.line(), "set pick jnb ");
    }

    #[test]
    fn first_tab_goes_as_far_as_certain_then_second_lists() {
        let mut editor = typed("set pick s");
        assert_eq!(editor.tab(&servers()), Action::Redraw);
        assert_eq!(editor.line(), "set pick sto");
        let Action::ShowCandidates(listed) = editor.tab(&servers()) else {
            panic!("second tab should list");
        };
        let codes: Vec<&str> = listed.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(codes, ["sto", "sto2"]);
    }

    #[test]
    fn with_nothing_to_extend_it_takes_two_tabs_to_list() {
        let mut editor = typed("s");
        assert_eq!(editor.tab(&servers()), Action::None);
        assert!(matches!(editor.tab(&servers()), Action::ShowCandidates(_)));
        assert_eq!(editor.line(), "s");
    }

    #[test]
    fn typing_between_tabs_resets_the_list_request() {
        let mut editor = typed("s");
        editor.tab(&servers());
        editor.key(Key::Left, CAPACITY);
        editor.key(Key::End, CAPACITY);
        assert_eq!(editor.tab(&servers()), Action::None);
    }

    #[test]
    fn tab_with_no_matches_does_nothing() {
        let mut editor = typed("wibble");
        assert_eq!(editor.tab(&servers()), Action::None);
        assert_eq!(editor.line(), "wibble");
    }

    #[test]
    fn a_name_match_without_a_common_code_prefix_is_not_clobbered() {
        let servers = vec![
            ("abc".to_string(), "Same City".to_string()),
            ("xyz".to_string(), "Same City 2".to_string()),
        ];
        let mut editor = typed("set pick city");
        assert_eq!(editor.tab(&servers), Action::None);
        assert_eq!(editor.line(), "set pick city");
        assert!(matches!(editor.tab(&servers), Action::ShowCandidates(_)));
    }

    #[test]
    fn tab_mid_line_keeps_the_rest() {
        let mut editor = typed("li servers");
        for _ in 0.."servers".len() + 1 {
            editor.key(Key::Left, CAPACITY);
        }
        editor.tab(&servers());
        assert_eq!(editor.line(), "list servers");
        assert_eq!(editor.cursor(), 4);
    }
}
