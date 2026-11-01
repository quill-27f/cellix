use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::Duration;

use cellix_input::{Key, KeyEvent};
use rmpv::Value;

/// A request that takes longer means Neovim is stuck, and a stuck editor must not freeze
/// the window.
const TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST: i64 = 0;
const RESPONSE: i64 = 1;

/// Everything worth knowing after a key. Lines only come back when they've changed.
const STATE: &str = r#"
local tick = ...
local changed = vim.b.changedtick ~= tick
return {
  mode = vim.fn.mode(),
  tick = vim.b.changedtick,
  cursor = vim.api.nvim_win_get_cursor(0),
  anchor = vim.fn.getpos('v'),
  cmdtype = vim.fn.getcmdtype(),
  cmdline = vim.fn.getcmdline(),
  lines = changed and vim.api.nvim_buf_get_lines(0, 0, -1, false) or false,
}
"#;

/// Where the editor stands after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub mode: String,
    pub cursor: (usize, usize),
    pub anchor: (usize, usize),
    pub cmdtype: String,
    pub cmdline: String,
    pub lines: Option<Vec<String>>,
}

/// Which keys Neovim gets. Cellix keeps a few, so getting around the app works the same in
/// every keymap, and anything an agent's review needs stays one chord away.
pub fn notation(key: KeyEvent) -> Option<String> {
    let mods = key.mods;
    let named = |name: &str| Some(format!("<{name}>"));
    match key.key {
        Key::Char(c) if mods.ctrl => {
            let kept = matches!(c.to_ascii_lowercase(), 'p' | 'q' | 's' | 'g' | 'b');
            (!kept && !mods.alt && !mods.shift).then(|| format!("<C-{c}>"))
        }
        Key::Char(_) if mods.alt => None,
        Key::Char('<') => named("lt"),
        Key::Char(c) => Some(c.to_string()),
        Key::Enter => named("CR"),
        Key::Esc => named("Esc"),
        Key::Backspace => named("BS"),
        Key::Delete => named("Del"),
        Key::Tab => named("Tab"),
        Key::BackTab => named("S-Tab"),
        Key::Left => named("Left"),
        Key::Right => named("Right"),
        Key::Up => named("Up"),
        Key::Down => named("Down"),
        Key::Home => named("Home"),
        Key::End => named("End"),
        Key::PageUp => named("PageUp"),
        Key::PageDown => named("PageDown"),
        _ => None,
    }
}

fn int(value: &Value) -> usize {
    value.as_u64().unwrap_or(0) as usize
}

fn field<'a>(map: &'a Value, name: &str) -> Option<&'a Value> {
    let entries = map.as_map()?;
    let found = entries.iter().find(|(key, _)| key.as_str() == Some(name));
    found.map(|(_, value)| value)
}

fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A headless Neovim holding one scratch buffer per open file. It's the user's own, with
/// their config, so motions, registers and macros all work. It never touches the files.
pub struct Neovim {
    child: Child,
    input: BufWriter<ChildStdin>,
    replies: Receiver<Value>,
    next_id: i64,
    buffers: HashMap<PathBuf, i64>,
    tick: i64,
}

impl Drop for Neovim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Neovim {
    pub fn start(args: &[&str]) -> io::Result<Self> {
        let mut child = Command::new("nvim")
            .args(["--embed", "--headless"])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("no stdin"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no stdout"))?;
        let (sender, replies) = channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(output);
            while let Ok(message) = rmpv::decode::read_value(&mut reader) {
                let is_reply = message
                    .as_array()
                    .is_some_and(|m| m.first().and_then(Value::as_i64) == Some(RESPONSE));
                if is_reply && sender.send(message).is_err() {
                    return;
                }
            }
        });
        let mut neovim = Self {
            child,
            input: BufWriter::new(input),
            replies,
            next_id: 1,
            buffers: HashMap::new(),
            tick: -1,
        };
        neovim
            .call("nvim_command", vec!["set noswapfile".into()])
            .ok_or_else(|| io::Error::other("Neovim didn't answer"))?;
        Ok(neovim)
    }

    /// Makes a file's buffer current, creating it if it's new.
    pub fn show(&mut self, path: &Path) -> Option<()> {
        let existing = self.buffers.get(path).copied();
        let buffer = match existing {
            Some(buffer) => buffer,
            None => {
                let made = self.call("nvim_create_buf", vec![true.into(), true.into()])?;
                let buffer = made.as_ext().map_or(0, |(_, data)| i64::from(data[0]));
                self.buffers.insert(path.to_path_buf(), buffer);
                buffer
            }
        };
        self.call(
            "nvim_exec2",
            vec![format!("buffer {buffer}").into(), Value::Map(Vec::new())],
        )?;
        Some(())
    }

    /// `request` with the result, for calls whose answer matters.
    fn call(&mut self, method: &str, params: Vec<Value>) -> Option<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = Value::Array(vec![
            REQUEST.into(),
            id.into(),
            method.into(),
            Value::Array(params),
        ]);
        rmpv::encode::write_value(&mut self.input, &message).ok()?;
        self.input.flush().ok()?;
        loop {
            let reply = self.replies.recv_timeout(TIMEOUT).ok()?;
            let parts = reply.as_array()?;
            if parts.get(1).and_then(Value::as_i64) == Some(id) {
                return parts.get(2)?.is_nil().then(|| parts[3].clone());
            }
        }
    }

    /// Replaces the current buffer, for when the file changed outside Neovim.
    pub fn set_text(&mut self, text: &str) -> Option<()> {
        let body = text.strip_suffix('\n').unwrap_or(text);
        let lines: Vec<Value> = body.split('\n').map(Value::from).collect();
        let params = vec![
            0.into(),
            0.into(),
            (-1).into(),
            false.into(),
            Value::Array(lines),
        ];
        self.call("nvim_buf_set_lines", params)?;
        self.tick = -1;
        Some(())
    }

    /// Moves the cursor, given a zero-based line and a byte column.
    pub fn set_cursor(&mut self, line: usize, column: usize) -> Option<()> {
        let at = Value::Array(vec![(line as i64 + 1).into(), (column as i64).into()]);
        self.call("nvim_win_set_cursor", vec![0.into(), at])?;
        Some(())
    }

    pub fn input(&mut self, keys: &str) -> Option<()> {
        self.call("nvim_input", vec![keys.into()])?;
        Some(())
    }

    /// Reads the state, or None while Neovim is waiting for more of a command such as
    /// `d`. It holds requests back then, so asking would only hang.
    pub fn state(&mut self) -> Option<State> {
        let mode = self.call("nvim_get_mode", Vec::new())?;
        if field(&mode, "blocking").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let reply = self.call(
            "nvim_exec_lua",
            vec![STATE.into(), vec![Value::from(self.tick)].into()],
        )?;
        self.tick = field(&reply, "tick")?.as_i64()?;
        let cursor = field(&reply, "cursor")?.as_array()?;
        let anchor = field(&reply, "anchor")?.as_array()?;
        let lines = field(&reply, "lines")?.as_array().map(|lines| {
            lines
                .iter()
                .map(|line| line.as_str().unwrap_or_default().to_string())
                .collect()
        });
        Some(State {
            mode: text(field(&reply, "mode")),
            cursor: (int(&cursor[0]).saturating_sub(1), int(&cursor[1])),
            anchor: (
                int(&anchor[1]).saturating_sub(1),
                int(&anchor[2]).saturating_sub(1),
            ),
            cmdtype: text(field(&reply, "cmdtype")),
            cmdline: text(field(&reply, "cmdline")),
            lines,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellix_input::Mods;

    #[test]
    fn keys_turn_into_neovim_notation() {
        let plain = |key| notation(KeyEvent::plain(key));
        assert_eq!(plain(Key::Char('d')).as_deref(), Some("d"));
        assert_eq!(plain(Key::Char('<')).as_deref(), Some("<lt>"));
        assert_eq!(plain(Key::Esc).as_deref(), Some("<Esc>"));
        assert_eq!(notation(KeyEvent::ctrl('r')).as_deref(), Some("<C-r>"));
    }

    #[test]
    fn cellix_keeps_its_own_chords() {
        for c in ['p', 'q', 's', 'g', 'b'] {
            assert_eq!(notation(KeyEvent::ctrl(c)), None, "{c}");
        }
        let alt = KeyEvent {
            key: Key::Char('c'),
            mods: Mods {
                alt: true,
                ..Mods::default()
            },
        };
        assert_eq!(notation(alt), None);
    }
}
