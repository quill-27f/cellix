//! Which commands have something to act on now. The palette lists only those, as VS Code's
//! does: a list that begins with Abort Merge when nothing is being merged is one to read
//! past. A command is left out only when what it needs is known to be missing. Where that
//! can't be told without reading the disk again, as for a project's tasks or the folders
//! opened lately, it stays, and says what's missing when it's run.

use super::*;

/// What git said at the last look at the disk that a command's place in the palette turns on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct GitNow {
    /// The folder is in a repository.
    pub repo: bool,
    /// A merge, rebase, cherry-pick or revert waits to be finished.
    pub underway: bool,
    /// Something is stashed.
    pub stashes: bool,
}

impl GitNow {
    /// Two quick questions to git, asked away from the screen with the rest of the look.
    pub(super) fn of(root: &Path) -> Self {
        let underway = cellix_review::conflict::is_underway(root);
        Self {
            repo: underway.is_some(),
            underway: underway == Some(true),
            stashes: underway.is_some() && cellix_review::any_stashed(root),
        }
    }
}

impl App {
    /// Whether `command` has something to act on as things stand. Each arm is the test its
    /// handler makes before it does anything, or what the menus grey it by. The keys of a
    /// command this turns down still run it, and it says what's missing as it always has.
    pub(crate) fn can_run(&self, command: Command) -> bool {
        use Command as C;
        let editor = self.editors.get(self.active());
        let path = editor.and_then(|e| e.buffer.path());
        // What kind of page the file opens as, if it does: Markdown, a notebook, a paper, a
        // table. Code has none.
        let page = path.and_then(Dialect::of);
        let latex = page == Some(Dialect::Latex);
        let writing = page.is_some_and(Dialect::is_writing);
        // An agent's transcript is read, not written.
        let edits = editor.is_some_and(|e| !e.buffer.read_only);
        let as_code = editor.is_some_and(|e| !e.rendered);
        let notebook = editor.is_some_and(|e| e.notebook.is_some());
        let server = || {
            let language = path.and_then(Registry::language_name);
            language.is_some_and(|l| self.servers.contains_key(l))
        };
        let suggestions = || {
            editor.is_some_and(|e| {
                !cellix_doc::changes::find(&e.buffer.text(), e.change_syntax()).is_empty()
            })
        };
        // What git said last. Before the first look nothing is known, and a command stays.
        let git = |known: fn(&GitNow) -> bool| self.git_now.as_ref().is_none_or(known);
        let repo = git(|now| now.repo);
        match command {
            // The file in front, whatever it is.
            C::Save | C::SelectAll | C::GoToLine | C::GoToSymbol => editor.is_some(),
            C::Undo | C::Redo | C::DuplicateLine | C::MoveLineUp | C::Complete => edits,
            C::ToggleComment => edits && path.and_then(Registry::line_comment).is_some(),
            // A page lays itself out: wrapping and folding are for code.
            C::ToggleWordWrap => as_code,
            C::Fold | C::FoldAll => {
                editor.is_some_and(|e| !e.rendered && !e.fold_ranges().is_empty())
            }
            C::Unfold | C::UnfoldAll => editor.is_some_and(|e| !e.rendered && e.any_folded()),

            // Tabs, panes and places.
            C::CloseTab => editor.is_some() || self.settings_page.is_some() || self.task_shown(),
            C::NextTab | C::PreviousTab => {
                self.editors.len() + usize::from(self.task_tab().is_some()) > 1
            }
            C::SplitPane => !self.compact(),
            C::FocusNextPane => self.panes.len() > 1,
            C::MaximizeEditor => {
                !self.editors.is_empty() || self.maximized == Some(Maximized::Editor)
            }
            C::CloseDock => self.dock.open,
            C::GoBack => !self.back.is_empty(),
            C::GoForward => !self.forward.is_empty(),
            C::NextWorkspace | C::PreviousWorkspace => self.workspaces > 1,

            // What a language server answers, as the editor's menu greys them.
            C::GoToDefinition
            | C::GoToReferences
            | C::QuickFix
            | C::ShowHover
            | C::RenameSymbol => server(),
            C::FormatDocument => path.is_some_and(|path| {
                let language = Registry::language_name(path).unwrap_or("Plain Text");
                self.servers.contains_key(language) || self.settings.formatter(language).is_some()
            }),

            // Writing: Markdown, notebooks and papers.
            C::Format(_) | C::ToggleTracking => edits && writing,
            C::MarkdownPreview => editor.is_some_and(|e| e.rendered || e.has_page()),
            C::BuildPdf => latex,
            C::ExportPdf => writing,
            C::ShowInPdf => latex && self.dock.pdf.is_some(),
            C::OpenInBrowser => self.has_browser_target(),
            C::AcceptChange | C::RejectChange => {
                editor.is_some_and(|e| e.change_at_caret().is_some())
            }
            C::AcceptAllChanges | C::RejectAllChanges | C::NextChange | C::PreviousChange => {
                suggestions()
            }
            C::RunCell | C::RunAllCells | C::ClearOutputs | C::RestartKernel => notebook,
            C::InterruptKernel => notebook && path.is_some_and(|p| self.kernels.contains_key(p)),

            // Comments and agents.
            C::AddComment | C::AskAgent => path.is_some(),
            C::SendToAgent => self.comments.lock().is_ok_and(|c| !c.is_empty()),
            C::MentionSelection => {
                path.is_some() && self.ide.as_ref().is_some_and(|ide| ide.connected())
            }
            C::AcceptProposal => self.review.as_ref().is_some_and(|r| r.is_proposal()),
            C::RejectProposal => self.proposal.is_some(),
            C::ApprovePlan | C::SendBackPlan => self.plan.is_some(),
            C::ComparePlanVersions => {
                let plan = self.plan.as_ref().map(|plan| plan.path.as_path());
                plan.or(path).is_some_and(|p| !plan_versions(p).is_empty())
            }
            // Review knows the turns while it's open. Closed, only git does.
            C::UndoTurn => self
                .review
                .as_ref()
                .is_none_or(|r| r.undo_target().is_some()),
            C::OpenTranscript => self.dock.sessions.iter().any(|s| !s.shell && !s.browser),
            C::RenamePaneTab => self.dock.active.is_some(),

            // Git.
            C::SwitchBranch
            | C::Stash
            | C::StashUntracked
            | C::StashStaged
            | C::ShowHistory
            | C::NewWorktree => repo,
            C::BlameLine => repo && path.is_some(),
            C::FileHistory => repo && self.in_workspace(path),
            // With the clipboard too, which needs no repository.
            C::CompareWith => self.in_workspace(path),
            // Review does the committing.
            C::Commit => repo && self.review.is_some(),
            C::PopStash | C::ApplyStash | C::PickStashToPop | C::DropStash => {
                git(|now| now.stashes)
            }
            C::AbortMerge => git(|now| now.underway),
            C::AcceptCurrent
            | C::AcceptIncoming
            | C::AcceptBoth
            | C::CompareConflict
            | C::NextConflict
            | C::PreviousConflict => editor.is_some_and(|e| !crate::conflicts::found(e).is_empty()),

            // The rest open, show, toggle or start something whatever is in view.
            _ => true,
        }
    }

    /// Whether a file is this workspace's own, as comparing it and its history need.
    fn in_workspace(&self, path: Option<&Path>) -> bool {
        path.is_some_and(|path| path.starts_with(self.workspace.root()))
    }

    /// Whether Open in Browser has something to open: a link at the caret, or a file a
    /// browser shows. It's `browser_target` without writing the page that would open.
    fn has_browser_target(&self) -> bool {
        let Some(editor) = self.editors.get(self.active()) else {
            return false;
        };
        let rope = editor.buffer.rope();
        let head = editor.selections.primary().head;
        let line = rope.char_to_line(head);
        let text = rope.line(line).to_string();
        let column = head - rope.line_to_char(line);
        let shown = |path: &Path| {
            let kind = path.extension().and_then(|kind| kind.to_str());
            matches!(kind, Some("html" | "htm" | "md" | "markdown" | "mdx"))
        };
        rendered::link_at(text.trim_end(), column).is_some()
            || editor.buffer.path().is_some_and(shown)
    }
}
