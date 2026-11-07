//! Keyboard shortcuts: every command's keys, the ones the person set in place of the
//! defaults, and keys two commands share.

use std::collections::HashMap;
use std::sync::OnceLock;

use cellix_input::{Command, Key, KeyEvent, Mods, simple_keymap};

use crate::palette::{COMMANDS, keys_of};

/// Commands that have keys but no row in the palette, which the page lists too.
const KEYS_ONLY: [(&str, Command); 17] = [
    ("Show All Commands", Command::CommandPalette),
    ("Go to File", Command::QuickOpen),
    ("Copy", Command::Copy),
    ("Cut", Command::Cut),
    ("Paste", Command::Paste),
    ("Add Selection to Next Find Match", Command::SelectNextMatch),
    ("Select All Occurrences", Command::SelectAllMatches),
    ("Add Cursor Above", Command::AddCursorAbove),
    ("Add Cursor Below", Command::AddCursorBelow),
    ("Move Line Down", Command::MoveLineDown),
    ("Delete Line", Command::DeleteLine),
    ("Insert Line Below", Command::InsertLineBelow),
    ("Insert Line Above", Command::InsertLineAbove),
    ("Next Problem", Command::NextProblem),
    ("Previous Problem", Command::PreviousProblem),
    ("Indent Line", Command::Indent),
    ("Outdent Line", Command::Outdent),
];

/// Every command the page lists, by name.
fn named() -> impl Iterator<Item = &'static (&'static str, Command)> {
    COMMANDS.iter().chain(KEYS_ONLY.iter())
}

/// A key as settings and the page write it: `Ctrl+Shift+B`, `F12`, `Alt+Up`.
pub fn name(event: KeyEvent) -> String {
    let mut parts: Vec<String> = Vec::new();
    if event.mods.ctrl {
        parts.push("Ctrl".into());
    }
    if event.mods.alt {
        parts.push("Alt".into());
    }
    if event.mods.shift {
        parts.push("Shift".into());
    }
    parts.push(match event.key {
        Key::Char(' ') => "Space".into(),
        Key::Char(c) => c.to_uppercase().to_string(),
        Key::Enter => "Enter".into(),
        Key::Tab | Key::BackTab => "Tab".into(),
        Key::Backspace => "Backspace".into(),
        Key::Delete => "Delete".into(),
        Key::Esc => "Escape".into(),
        Key::Left => "Left".into(),
        Key::Right => "Right".into(),
        Key::Up => "Up".into(),
        Key::Down => "Down".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::F(n) => format!("F{n}"),
    });
    parts.join("+")
}

/// A key from its name, in any case: `ctrl+shift+b` as VS Code writes it works too.
pub fn parse(text: &str) -> Option<KeyEvent> {
    let mut mods = Mods::default();
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    // `Ctrl++` names the plus key.
    let (last, rest): (&str, &[&str]) = if text.ends_with("++") {
        ("+", &parts[..parts.len().saturating_sub(2)])
    } else {
        let (last, rest) = parts.split_last()?;
        (last, rest)
    };
    for part in rest {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "meta" => mods.ctrl = true,
            "alt" | "option" | "opt" => mods.alt = true,
            "shift" => mods.shift = true,
            _ => return None,
        }
    }
    let lower = last.to_ascii_lowercase();
    let key = match lower.as_str() {
        "space" => Key::Char(' '),
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "escape" | "esc" => Key::Esc,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        f if f.starts_with('f') && f.len() > 1 => Key::F(f[1..].parse().ok()?),
        _ => {
            let mut chars = lower.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            Key::Char(c)
        }
    };
    Some(KeyEvent { key, mods })
}

/// How a key arrives, made one form, so `Ctrl+Shift+P` matches however the terminal
/// sent it: shifted letters come as capitals, Shift+Tab as BackTab.
pub fn normal(event: KeyEvent) -> KeyEvent {
    let mut event = event;
    match event.key {
        Key::Char(c) if c.is_ascii_uppercase() => {
            event.key = Key::Char(c.to_ascii_lowercase());
            event.mods.shift = true;
        }
        Key::BackTab => {
            event.key = Key::Tab;
            event.mods.shift = true;
        }
        _ => {}
    }
    event
}

/// Keys terminals send for others, which aren't worth listing as a command's own.
const STAND_INS: [&str; 5] = ["Ctrl+4", "Ctrl+7", "Ctrl+_", "Ctrl+Shift+_", "Ctrl+~"];

/// Every command's default keys, found by asking the key map about every key, so this can't
/// drift from what the keys really do. The key the palette shows comes first.
pub fn defaults() -> &'static HashMap<Command, Vec<String>> {
    static FOUND: OnceLock<HashMap<Command, Vec<String>>> = OnceLock::new();
    FOUND.get_or_init(|| {
        let mut found: HashMap<Command, Vec<String>> = HashMap::new();
        let mut keys: Vec<Key> = vec![
            Key::Enter,
            Key::Tab,
            Key::Backspace,
            Key::Delete,
            Key::Esc,
            Key::Left,
            Key::Right,
            Key::Up,
            Key::Down,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
        ];
        keys.extend((1..=12).map(Key::F));
        keys.extend(
            (' '..='~')
                .filter(|c| !c.is_ascii_uppercase())
                .map(Key::Char),
        );
        for bits in 0..8u8 {
            let mods = Mods {
                ctrl: bits & 1 != 0,
                alt: bits & 2 != 0,
                shift: bits & 4 != 0,
            };
            for key in &keys {
                let event = KeyEvent { key: *key, mods };
                let Some(command) = simple_keymap(event) else {
                    continue;
                };
                let written = name(event);
                if STAND_INS.contains(&written.as_str()) {
                    continue;
                }
                // Ctrl+Shift+S saves only because Ctrl+S does: only the plainer one is listed.
                let fewer = [
                    Mods {
                        ctrl: false,
                        ..mods
                    },
                    Mods { alt: false, ..mods },
                    Mods {
                        shift: false,
                        ..mods
                    },
                ];
                let plainer = fewer.iter().filter(|m| **m != mods).any(|m| {
                    let event = KeyEvent {
                        key: *key,
                        mods: *m,
                    };
                    simple_keymap(event) == Some(command)
                });
                if plainer {
                    continue;
                }
                let list = found.entry(command).or_default();
                if !list.contains(&written) {
                    list.push(written);
                }
            }
        }
        // The key the palette shows comes first.
        for (command, keys) in &mut found {
            if let Some(main) = keys_of(*command)
                && let Some(at) = keys.iter().position(|k| k == main)
            {
                let main = keys.remove(at);
                keys.insert(0, main);
            }
        }
        found
    })
}

/// A command as the shortcuts page lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub name: &'static str,
    pub command: Command,
    pub keys: Vec<String>,
    /// Set by the person, rather than Cellix's own.
    pub user: bool,
}

/// The person's keys, by the palette's name for each command, in place of its defaults. An
/// empty list takes a command's keys away.
#[derive(Default)]
pub struct Bindings {
    /// Each key the person set, and its command.
    bound: HashMap<KeyEvent, Command>,
    /// Commands whose defaults the person replaced.
    replaced: Vec<Command>,
}

impl Bindings {
    pub fn new(set: &HashMap<String, Vec<String>>) -> Self {
        let mut bindings = Self::default();
        for (name, keys) in set {
            let Some((_, command)) = named().find(|(n, _)| n == name) else {
                continue;
            };
            bindings.replaced.push(*command);
            for key in keys.iter().filter_map(|k| parse(k)) {
                bindings.bound.insert(normal(key), *command);
            }
        }
        bindings
    }

    /// What a key does: the person's binding first, then the default unless replaced.
    pub fn command(&self, event: KeyEvent) -> Option<Command> {
        if let Some(command) = self.bound.get(&normal(event)) {
            return Some(*command);
        }
        let command = simple_keymap(event)?;
        // Keys that edit text aren't on the page, so they can't have been replaced.
        (!self.replaced.contains(&command)).then_some(command)
    }
}

/// Every command, with the keys it has now.
pub fn shortcuts(set: &HashMap<String, Vec<String>>) -> Vec<Shortcut> {
    let defaults = defaults();
    let mut list: Vec<Shortcut> = named()
        .map(|(name, command)| match set.get(*name) {
            Some(keys) => Shortcut {
                name,
                command: *command,
                keys: keys.iter().filter_map(|k| parse(k)).map(name_of).collect(),
                user: true,
            },
            None => Shortcut {
                name,
                command: *command,
                keys: defaults.get(command).cloned().unwrap_or_default(),
                user: false,
            },
        })
        .collect();
    list.sort_by_key(|s| s.name.to_lowercase());
    list
}

fn name_of(event: KeyEvent) -> String {
    name(normal(event))
}

/// How many commands each key is on, by its name.
pub fn sharing(list: &[Shortcut]) -> HashMap<String, usize> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for shortcut in list {
        for key in &shortcut.keys {
            *count.entry(key.clone()).or_default() += 1;
        }
    }
    count
}

/// Whether a row fits what's typed: by name, or by keys as `ctrl+r` or `Ctrl+R`.
pub fn matches(shortcut: &Shortcut, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    shortcut.name.to_lowercase().contains(&query)
        || shortcut
            .keys
            .iter()
            .any(|k| k.to_lowercase().contains(&query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_go_both_ways() {
        for text in [
            "Ctrl+Shift+B",
            "F12",
            "Alt+Up",
            "Ctrl+\\",
            "Ctrl+Space",
            "Shift+F8",
        ] {
            assert_eq!(name(parse(text).unwrap()), text);
        }
        assert_eq!(parse("ctrl+shift+b"), parse("Ctrl+Shift+B"));
        assert!(parse("Hyper+X").is_none());
    }

    #[test]
    fn defaults_come_from_the_key_map_itself() {
        let found = defaults();
        assert_eq!(found[&Command::CommandPalette][0], "Ctrl+Shift+P");
        assert!(found[&Command::CommandPalette].contains(&"F1".to_string()));
        assert!(found[&Command::RunBuildTask].contains(&"Ctrl+Shift+B".to_string()));
        assert_eq!(found[&Command::SplitPane], vec!["Ctrl+\\".to_string()]);
    }

    #[test]
    fn the_persons_keys_win_and_replace_the_defaults() {
        let mut set = HashMap::new();
        set.insert(
            "Rerun Last Task".to_string(),
            vec!["Ctrl+Alt+R".to_string()],
        );
        set.insert("Toggle Sidebar".to_string(), vec![]);
        set.insert("Open Recent…".to_string(), vec!["ctrl+r".to_string()]);
        let bindings = Bindings::new(&set);
        let ctrl_alt_r = parse("Ctrl+Alt+R").unwrap();
        assert_eq!(bindings.command(ctrl_alt_r), Some(Command::RerunLastTask));
        assert_eq!(bindings.command(KeyEvent::ctrl('b')), None, "taken away");
        assert_eq!(
            bindings.command(KeyEvent::ctrl('r')),
            Some(Command::OpenRecent)
        );
        assert_eq!(bindings.command(KeyEvent::ctrl('s')), Some(Command::Save));
        let list = shortcuts(&set);
        let shared = sharing(&list);
        assert_eq!(shared["Ctrl+R"], 2, "Replace in File keeps it too");
    }
}
