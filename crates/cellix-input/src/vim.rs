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

/// A built-in subset of Vim: motions, counts, operators with a motion, Visual and Insert.
#[derive(Debug)]
pub struct Vim {
    pub mode: VimMode,
    count: Option<usize>,
    operator: Option<char>,
    g_pending: bool,
}

impl Default for Vim {
    fn default() -> Self {
        Self {
            mode: VimMode::Normal,
            count: None,
            operator: None,
            g_pending: false,
        }
    }
}

fn motion(key: char) -> Option<Direction> {
    Some(match key {
        'h' => Direction::Left,
        'l' => Direction::Right,
        'k' => Direction::Up,
        'j' => Direction::Down,
        'w' | 'e' => Direction::WordRight,
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
            return VimAction::Passthrough;
        };
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
            'v' => {
                self.reset();
                self.mode = VimMode::Visual;
                VimAction::Pending
            }
            _ => {
                self.reset();
                VimAction::Pending
            }
        }
    }

    /// The key after `d`, `c` or `y`: its own letter means the whole line.
    fn operate(&mut self, operator: char, c: char) -> VimAction {
        use Command::*;
        let times = self.count.unwrap_or(1);
        self.reset();
        let body: Vec<Command> = if c == operator {
            vec![SelectLines(times)]
        } else if let Some(direction) = motion(c) {
            vec![Move(direction, true); times]
        } else {
            return VimAction::Pending;
        };
        let finish = match operator {
            'y' => [Copy, Cancel],
            _ => [Cut, Cancel],
        };
        let mut commands = body;
        commands.extend(finish);
        if operator == 'c' {
            self.mode = VimMode::Insert;
        }
        VimAction::Commands(commands)
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
