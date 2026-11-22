//! Which program in the side pane has the person's keys, and what each is told of it. A
//! program that asks (focus reporting, DECSET 1004) is told as its keys come and go, as a
//! terminal tells it of its window's focus: Claude Code draws its prompt's cursor only while
//! told it has them, and showed it with the keys in the file, which read as a place to type that
//! wasn't one (the user, 22 November). It's worked out from where things stand each time the app
//! settles, after an event or a poll, rather than at each place the keyboard moves, and a
//! program is sent only what changed. A workspace that comes forward for the first time with
//! an agent's tab in front of an open pane starts with the keyboard there.

use super::*;

impl App {
    /// The session the person's keys go to now, if one does: the one in front of the side pane
    /// while it has the keyboard, or a helper looked at in peek, which has it while it's open,
    /// as its pastes did. None while a box, a menu, a dialog or a field has them, while another
    /// workspace is in front, or while Cellix's window is behind another. It follows the order
    /// `App::key` hands a key on in.
    pub(crate) fn session_with_keys(&self) -> Option<usize> {
        if !self.window_focused() || !self.in_front {
            return None;
        }
        let taken = !matches!(self.overlay, Overlay::None)
            || self.side_field_open()
            || self.dock.draft.is_some()
            || self.source.focused;
        if taken {
            return None;
        }
        if self.peek.is_some() {
            return self.peeked().filter(|_| self.settings_page.is_none());
        }
        let front = self.dock.open && self.dock.front == Front::Session && !self.dock.listing();
        let index = self.dock.active.filter(|_| self.dock.focused && front)?;
        self.dock.sessions.get(index).filter(|s| !s.exited)?;
        Some(index)
    }

    /// Whether the keys go to the side pane rather than the file, wherever Cellix's window is:
    /// to what it shows while it has the keyboard, or to a helper in peek. The file's caret
    /// goes while they do, so only one place says it's where typing goes.
    pub(crate) fn pane_has_keyboard(&self) -> bool {
        (self.dock.open && self.dock.focused) || self.peek.is_some()
    }

    /// Whether session `index` has the keys: its cursor shows only then.
    pub(crate) fn has_keys(&self, index: Option<usize>) -> bool {
        index.is_some() && self.session_with_keys() == index
    }

    /// Tells each session's program where the keyboard is, where it asked and it changed.
    pub(crate) fn tell_keys(&mut self) {
        let keys = self.session_with_keys();
        for (index, session) in self.dock.sessions.iter_mut().enumerate() {
            session.tell_keys(Some(index) == keys);
        }
    }

    /// The first time this workspace is in front in this run: with the side pane open and an
    /// agent's tab in front, the keyboard starts there, so the cursor its prompt shows is where
    /// typing goes. Not over a dialog or Settings, and not where a file named at the command
    /// line has it (`keep_keys_in_file`). Once settled, it's never moved from here again: a
    /// workspace used in this run keeps the keyboard where the person left it.
    pub fn place_keys(&mut self) {
        if std::mem::replace(&mut self.keys_placed, true) {
            return;
        }
        let free = matches!(self.overlay, Overlay::None) && self.settings_page.is_none();
        let front = self.dock.open && self.dock.front == Front::Session && !self.dock.listing();
        let agent = self.dock.session().is_some_and(|session| {
            let own = session.agent() && !session.task();
            own || session.agent_in_front().is_some()
        });
        if free && front && agent && self.peek.is_none() {
            self.dock.focused = true;
        }
    }

    /// A file the person named at the command line keeps the keyboard, whatever the pane has:
    /// it isn't moved to the agent in front.
    pub fn keep_keys_in_file(&mut self) {
        self.keys_placed = true;
    }

    /// The side pane, for a test of the workspaces around this one.
    #[cfg(test)]
    pub(crate) fn dock_for_test(&mut self) -> &mut crate::dock::Dock {
        &mut self.dock
    }
}
