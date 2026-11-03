use crate::{Command, Direction, Key, KeyEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
}

/// What a key does in Vim mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VimAction {
    Commands(Vec<Command>),
    /// Handle the key as Simple mode would: typing, in Insert mode.
    Passthrough,
    /// Part of a longer sequence, such as the first `d` of `dd`.
    Pending,
}

/// A built-in subset of Vim: motions, counts, operators with a motion or text object,
/// Visual and Insert, and the `:` commands that save and close. Anything it doesn't know is
/// swallowed, never typed: an unknown key in Normal mode must not reach the file. With Neovim
/// installed, the `vim` keymap runs that instead.
#[derive(Debug)]
pub struct Vim {
    pub mode: VimMode,
    count: Option<usize>,
    operator: Option<char>,
    g_pending: bool,
    /// `i` or `a` after an operator, waiting for the object's letter.
    object: Option<char>,
    /// `f`, `t`, `F` or `T`, or `r`, waiting for the character.
    awaiting: Option<char>,
    /// What's being typed after `:`.
    line: Option<String>,
}

impl Default for Vim {
    fn default() -> Self {
        Self {
            mode: VimMode::Normal,
            count: None,
            operator: None,
            g_pending: false,
            object: None,
            awaiting: None,
            line: None,
        }
    }
}

fn motion(key: char) -> Option<Direction> {
    Some(match key {
        'h' => Direction::Left,
        'l' => Direction::Right,
        'k' => Direction::Up,
        'j' => Direction::Down,
        'w' => Direction::NextWordStart,
        'e' => Direction::WordRight,
        'b' => Direction::WordLeft,
        '$' => Direction::LineEnd,
        '0' => Direction::LineColumnZero,
        'G' => Direction::Bottom,
        _ => return None,
    })
}

impl Vim {
    fn reset(&mut self) {
        self.count = None;
        self.operator = None;
        self.g_pending = false;
        self.object = None;
        self.awaiting = None;
    }

    /// The `:` line being typed, for the status bar.
    pub fn command_line(&self) -> Option<&str> {
        self.line.as_deref()
    }

    /// A key on the `:` line.
    fn line_key(&mut self, event: KeyEvent) -> VimAction {
        use Command::*;
        let line = self.line.as_mut().expect("only called on a line");
        match event.key {
            Key::Esc => self.line = None,
            Key::Backspace if line.is_empty() => self.line = None,
            Key::Backspace => {
                line.pop();
            }
            Key::Char(c) => line.push(c),
            Key::Enter => {
                let typed = self.line.take().unwrap_or_default();
                let commands = match typed.trim() {
                    "w" => vec![Save],
                    "q" | "q!" => vec![CloseTab],
                    "wq" | "x" => vec![Save, CloseTab],
                    "qa" | "qa!" => vec![Quit],
                    number => match number.parse::<usize>() {
                        Ok(line) => {
                            let down = line.saturating_sub(1);
                            let mut moves = vec![Move(Direction::Top, false)];
                            moves.extend(vec![Move(Direction::Down, false); down]);
                            moves
                        }
                        Err(_) => Vec::new(),
                    },
                };
                return VimAction::Commands(commands);
            }
            _ => {}
        }
        VimAction::Pending
    }

    /// The character after `f`, `t`, `F`, `T` or `r`.
    fn awaited(&mut self, pending: char, c: char) -> VimAction {
        use Command::*;
        if pending == 'r' {
            self.reset();
            return VimAction::Commands(vec![DeleteForward, Type(c), Move(Direction::Left, false)]);
        }
        let direction =
            Direction::FindChar(c, pending.is_lowercase(), matches!(pending, 't' | 'T'));
        let operator = self.operator;
        let times = self.count.unwrap_or(1);
        self.reset();
        let Some(operator) = operator else {
            return VimAction::Commands(vec![Move(direction, false); times]);
        };
        // An operator reaches over the character it found, as Vim's `df` does.
        let mut commands = vec![Move(direction, true); times];
        if pending.is_lowercase() {
            commands.push(Move(Direction::Right, true));
        }
        self.finish(operator, commands)
    }

    /// The operator's own commands after its motion or object has selected what it acts on.
    fn finish(&mut self, operator: char, mut commands: Vec<Command>) -> VimAction {
        use Command::*;
        match operator {
            'y' => commands.extend([Copy, Cancel]),
            _ => commands.extend([Cut, Cancel]),
        }
        if operator == 'c' {
            self.mode = VimMode::Insert;
        }
        VimAction::Commands(commands)
    }

    fn repeat(&mut self, commands: Vec<Command>) -> VimAction {
        let times = self.count.unwrap_or(1);
        self.reset();
        VimAction::Commands(
            commands
                .iter()
                .copied()
                .cycle()
                .take(commands.len() * times)
                .collect(),
        )
    }

    fn enter_insert(&mut self, before: Vec<Command>) -> VimAction {
        self.reset();
        self.mode = VimMode::Insert;
        VimAction::Commands(before)
    }

    pub fn key(&mut self, event: KeyEvent) -> VimAction {
        if self.line.is_some() {
            return self.line_key(event);
        }
        match self.mode {
            VimMode::Insert => {
                if event.key == Key::Esc {
                    self.mode = VimMode::Normal;
                    return VimAction::Commands(vec![Command::Move(Direction::Left, false)]);
                }
                VimAction::Passthrough
            }
            VimMode::Normal | VimMode::Visual => self.command_key(event),
        }
    }

    fn visual_key(&mut self, c: char) -> VimAction {
        let operate = |vim: &mut Self, command| {
            vim.mode = VimMode::Normal;
            vim.reset();
            // Visual mode includes the character under the cursor.
            VimAction::Commands(vec![Command::Move(Direction::Right, true), command])
        };
        match c {
            'd' | 'x' => operate(self, Command::Cut),
            'y' => operate(self, Command::Copy),
            'v' => {
                self.mode = VimMode::Normal;
                self.reset();
                VimAction::Commands(vec![Command::Cancel])
            }
            _ => match motion(c) {
                Some(direction) => self.repeat(vec![Command::Move(direction, true)]),
                None => VimAction::Pending,
            },
        }
    }

    fn command_key(&mut self, event: KeyEvent) -> VimAction {
        use Command::*;
        if event.key == Key::Esc {
            self.reset();
            self.mode = VimMode::Normal;
            return VimAction::Commands(vec![Cancel]);
        }
        if event.mods.ctrl {
            return match event.key {
                Key::Char('r') => self.repeat(vec![Redo]),
                _ => VimAction::Passthrough,
            };
        }
        let Key::Char(c) = event.key else {
            // Arrows and the like still move, but nothing else may reach the file.
            return match event.key {
                Key::Enter | Key::Backspace | Key::Delete | Key::Tab => VimAction::Pending,
                _ => VimAction::Passthrough,
            };
        };
        if let Some(pending) = self.awaiting.take() {
            return self.awaited(pending, c);
        }
        if let Some(object) = self.object.take() {
            return self.text_object(object, c);
        }
        if c.is_ascii_digit() && (c != '0' || self.count.is_some()) {
            let digit = c as usize - '0' as usize;
            self.count = Some(self.count.unwrap_or(0) * 10 + digit);
            return VimAction::Pending;
        }
        if self.mode == VimMode::Visual {
            return self.visual_key(c);
        }
        if std::mem::take(&mut self.g_pending) {
            self.reset();
            return if c == 'g' {
                VimAction::Commands(vec![Move(Direction::Top, false)])
            } else {
                VimAction::Pending
            };
        }
        if matches!(c, 'f' | 't' | 'F' | 'T') {
            self.awaiting = Some(c);
            return VimAction::Pending;
        }
        if let Some(operator) = self.operator {
            return self.operate(operator, c);
        }
        if let Some(direction) = motion(c) {
            return self.repeat(vec![Move(direction, false)]);
        }
        match c {
            'g' => {
                self.g_pending = true;
                VimAction::Pending
            }
            'd' | 'y' | 'c' => {
                self.operator = Some(c);
                VimAction::Pending
            }
            'i' => self.enter_insert(Vec::new()),
            'a' => self.enter_insert(vec![Move(Direction::Right, false)]),
            'I' => self.enter_insert(vec![
                Move(Direction::LineColumnZero, false),
                Move(Direction::LineStart, false),
            ]),
            'A' => self.enter_insert(vec![Move(Direction::LineEnd, false)]),
            'o' => self.enter_insert(vec![Move(Direction::LineEnd, false), Newline]),
            'O' => self.enter_insert(vec![
                Move(Direction::LineColumnZero, false),
                Newline,
                Move(Direction::Up, false),
            ]),
            'x' => self.repeat(vec![DeleteForward]),
            'u' => self.repeat(vec![Undo]),
            'p' => self.repeat(vec![PasteLine]),
            'D' => self.repeat(vec![DeleteToLineEnd]),
            'C' => self.enter_insert(vec![DeleteToLineEnd]),
            'J' => self.repeat(vec![
                Move(Direction::LineEnd, false),
                DeleteForward,
                Type(' '),
            ]),
            'r' => {
                self.awaiting = Some('r');
                VimAction::Pending
            }
            ':' => {
                self.reset();
                self.line = Some(String::new());
                VimAction::Pending
            }
            '/' => {
                self.reset();
                VimAction::Commands(vec![FindInFile])
            }
            'v' => {
                self.reset();
                self.mode = VimMode::Visual;
                VimAction::Pending
            }
            'V' => {
                self.reset();
                self.mode = VimMode::Visual;
                VimAction::Commands(vec![
                    Move(Direction::LineColumnZero, false),
                    Move(Direction::LineEnd, true),
                ])
            }
            _ => {
                self.reset();
                VimAction::Pending
            }
        }
    }

    /// The key after `d`, `c` or `y`: its own letter means the whole line, and `i` or `a`
    /// starts a text object.
    fn operate(&mut self, operator: char, c: char) -> VimAction {
        use Command::*;
        if matches!(c, 'i' | 'a') {
            self.object = Some(c);
            return VimAction::Pending;
        }
        let times = self.count.unwrap_or(1);
        self.reset();
        // Vim's `cw` changes to the end of the word, not to the next one.
        let direction = match (operator, motion(c)) {
            ('c', Some(Direction::NextWordStart)) => Some(Direction::WordRight),
            (_, direction) => direction,
        };
        let body: Vec<Command> = if c == operator {
            vec![SelectLines(times)]
        } else if let Some(direction) = direction {
            vec![Move(direction, true); times]
        } else {
            return VimAction::Pending;
        };
        self.finish(operator, body)
    }

    /// The letter after `i` or `a` in `diw` or `caw`. Only words are known; the rest are
    /// swallowed whole, so they can't leave stray letters behind.
    fn text_object(&mut self, around: char, c: char) -> VimAction {
        use Command::*;
        let operator = self.operator;
        self.reset();
        let Some(operator) = operator.filter(|_| c == 'w') else {
            return VimAction::Pending;
        };
        // Into the word, back to its start, then over it.
        let mut commands = vec![
            Move(Direction::Right, false),
            Move(Direction::WordLeft, false),
            Move(Direction::WordRight, true),
        ];
        if around == 'a' {
            commands.push(Move(Direction::NextWordStart, true));
        }
        self.finish(operator, commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(vim: &mut Vim, keys: &str) -> Vec<VimAction> {
        keys.chars()
            .map(|c| vim.key(KeyEvent::plain(Key::Char(c))))
            .collect()
    }

    /// Runs keys through Vim, as the editor would, returning the commands they made.
    fn commands(vim: &mut Vim, keys: &str) -> Vec<Command> {
        let mut out = Vec::new();
        for c in keys.chars() {
            let event = match c {
                '\n' => KeyEvent::plain(Key::Enter),
                '\x1b' => KeyEvent::plain(Key::Esc),
                c => KeyEvent::plain(Key::Char(c)),
            };
            match vim.key(event) {
                VimAction::Commands(more) => out.extend(more),
                VimAction::Passthrough => out.push(Command::Type(c)),
                VimAction::Pending => {}
            }
        }
        out
    }

    #[test]
    fn change_inner_word_replaces_just_the_word() {
        let mut vim = Vim::default();
        let made = commands(&mut vim, "ciwHI\x1b");
        assert!(made.contains(&Command::Cut), "{made:?}");
        let typed: String = made
            .iter()
            .filter_map(|c| match c {
                Command::Type(c) => Some(*c),
                _ => None,
            })
            .collect();
        assert_eq!(typed, "HI", "only what's typed in Insert reaches the file");
        assert_eq!(vim.mode, VimMode::Normal);
    }

    #[test]
    fn unknown_keys_in_normal_mode_never_type() {
        let mut vim = Vim::default();
        let made = commands(&mut vim, "dif.QZ!\n");
        assert!(
            !made.iter().any(|c| matches!(c, Command::Type(_))),
            "{made:?}"
        );
        assert_eq!(vim.mode, VimMode::Normal);
    }

    #[test]
    fn colon_commands_save_close_and_go_to_a_line() {
        let mut vim = Vim::default();
        assert_eq!(commands(&mut vim, ":w\n"), vec![Command::Save]);
        assert_eq!(
            commands(&mut vim, ":wq\n"),
            vec![Command::Save, Command::CloseTab]
        );
        let to_three = commands(&mut vim, ":3\n");
        assert_eq!(to_three.len(), 3, "top, then down twice: {to_three:?}");
        assert!(vim.command_line().is_none());
    }

    #[test]
    fn find_motions_reach_the_character() {
        let mut vim = Vim::default();
        let made = commands(&mut vim, "fx");
        assert_eq!(
            made,
            vec![Command::Move(Direction::FindChar('x', true, false), false)]
        );
        let made = commands(&mut vim, "dtx");
        assert!(made.contains(&Command::Cut), "{made:?}");
    }

    #[test]
    fn w_goes_to_the_next_word_and_e_to_the_end() {
        assert_eq!(motion('w'), Some(Direction::NextWordStart));
        assert_eq!(motion('e'), Some(Direction::WordRight));
    }

    #[test]
    fn counts_repeat_motions() {
        let mut vim = Vim::default();
        let actions = press(&mut vim, "3j");
        let want = vec![Command::Move(Direction::Down, false); 3];
        assert_eq!(actions[1], VimAction::Commands(want));
    }

    #[test]
    fn insert_passes_keys_through_until_escape() {
        let mut vim = Vim::default();
        press(&mut vim, "i");
        assert_eq!(press(&mut vim, "q")[0], VimAction::Passthrough);
        vim.key(KeyEvent::plain(Key::Esc));
        assert_eq!(vim.mode, VimMode::Normal);
    }

    #[test]
    fn zero_is_a_motion_until_a_count_starts() {
        let mut vim = Vim::default();
        let want = vec![Command::Move(Direction::LineColumnZero, false)];
        assert_eq!(press(&mut vim, "0")[0], VimAction::Commands(want));
        assert_eq!(press(&mut vim, "10")[1], VimAction::Pending);
    }
}
