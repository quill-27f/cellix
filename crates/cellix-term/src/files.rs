//! Tiles handed over as files, where the terminal is on this machine and reads them.
//!
//! Inline, each tile is compressed, base64-encoded and sent in 4096-byte pieces, so a frame
//! with many new tiles (the first, a resize, a drag of the side pane's edge, a theme change)
//! is hundreds of kilobytes through the terminal. The Kitty protocol lets the terminal read a
//! temporary file instead (`t=t`) and delete it once read: then only its name crosses.
//!
//! Only for Ghostty with nothing between (see `allowed`), only once a probe at start has shown
//! the terminal takes a file, and only until anything goes wrong, when tiles go inline for the
//! rest of the run. The files are raw pixels in a folder only this user can open, each written
//! whole and closed before the terminal is told its name. `docs/DESIGN.md` has the reasoning.

use std::path::PathBuf;
#[cfg(unix)]
use std::sync::{Arc, Mutex};

/// Turns tile files off when set to 0.
pub const SWITCH: &str = "CELLIX_TILE_FILES";

/// What says Cellix's output doesn't go straight to a terminal on this machine: a connection
/// from elsewhere, a multiplexer between, or Claude Code, whose pane shows its own pictures.
const ELSEWHERE: [&str; 8] = [
    "SSH_CONNECTION",
    "SSH_CLIENT",
    "SSH_TTY",
    "TMUX",
    "STY",
    "ZELLIJ",
    "CLAUDECODE",
    "CELLIX_MOD",
];

/// Whether tiles may go as files here: Ghostty is the terminal, on this machine, with nothing
/// between, and the switch isn't off. Set at all, even empty, a variable of `ELSEWHERE` says
/// no: guessing wrong here costs nothing but bytes, and the other way costs every picture.
pub fn allowed(var: impl Fn(&str) -> Option<String>) -> bool {
    let off = var(SWITCH).is_some_and(|value| value.trim() == "0");
    let ghostty = var("TERM_PROGRAM").is_some_and(|name| name.eq_ignore_ascii_case("ghostty"));
    cfg!(unix) && !off && ghostty && ELSEWHERE.iter().all(|name| var(name).is_none())
}

/// Tiles waiting for the terminal to take them, at most. A first paint at 4K is a few hundred.
pub const MOST_FILES: usize = 1024;
/// Their bytes, at most: a first paint at 4K is about 25 MB of raw pixels.
pub const MOST_BYTES: usize = 64 * 1024 * 1024;
/// One tile's bytes, at most.
pub const MOST_TILE: usize = 16 * 1024 * 1024;

/// The ledger of tile files for one run, shared by every backend the run makes. Clones are
/// the same ledger; the last one dropped clears what the terminal left.
#[derive(Clone)]
pub struct TileFiles {
    #[cfg(unix)]
    ledger: Arc<Mutex<unix::Ledger>>,
}

impl TileFiles {
    /// A ledger where `allowed` says files may go, in the user's own temp folder. Nothing is
    /// written until the probe.
    pub fn for_terminal(var: impl Fn(&str) -> Option<String>) -> Option<Self> {
        if !allowed(&var) {
            return None;
        }
        #[cfg(unix)]
        {
            let root = unix::temp_root(var("TMPDIR"));
            Some(Self::with_ledger(unix::Ledger::new(root)))
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    #[cfg(unix)]
    fn with_ledger(ledger: unix::Ledger) -> Self {
        Self {
            ledger: Arc::new(Mutex::new(ledger)),
        }
    }

    /// A ledger in `root` whose probe was taken, for tests.
    #[cfg(all(test, unix))]
    pub(crate) fn for_test(root: PathBuf) -> Self {
        Self::with_ledger(unix::Ledger::taken(root))
    }

    /// The question that asks whether the terminal takes a file, with `id` for its answer:
    /// a 1 by 1 picture in a file of its own, queried rather than kept. `None` if the file
    /// couldn't be written, and then files are off.
    pub fn question(&self, id: u32) -> Option<String> {
        #[cfg(unix)]
        {
            self.ledger.lock().ok()?.question(id)
        }
        #[cfg(not(unix))]
        {
            let _ = id;
            None
        }
    }

    /// What the terminal answered: whether it said OK. Files are on from now if it did and the
    /// file is gone, as Ghostty deletes one it has read; off for the run otherwise.
    pub fn answered(&self, ok: bool) -> bool {
        #[cfg(unix)]
        {
            self.ledger
                .lock()
                .is_ok_and(|mut ledger| ledger.answered(ok))
        }
        #[cfg(not(unix))]
        {
            let _ = ok;
            false
        }
    }

    /// Whether tiles go as files.
    pub fn on(&self) -> bool {
        #[cfg(unix)]
        {
            self.ledger.lock().is_ok_and(|ledger| ledger.on())
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// Writes a tile's pixels to a file of their own, whole and closed, and gives its path for
    /// the terminal to read. `None` sends the tile inline: files are off, a cap is reached, or
    /// the write failed, which turns files off for good.
    pub fn write(&self, pixels: &[u8]) -> Option<PathBuf> {
        #[cfg(unix)]
        {
            let now = std::time::Instant::now();
            self.ledger.lock().ok()?.write(pixels, now)
        }
        #[cfg(not(unix))]
        {
            let _ = pixels;
            None
        }
    }

    /// The folder this run's files are in, once there is one, for a stop to clear.
    pub fn folder(&self) -> Option<PathBuf> {
        #[cfg(unix)]
        {
            self.ledger.lock().ok()?.folder()
        }
        #[cfg(not(unix))]
        {
            None
        }
    }

    /// Turns files off and removes what the terminal hasn't taken, and the folder. For the end
    /// of a run: the terminal may still be reading the last frame.
    pub fn clear(&self) {
        #[cfg(unix)]
        if let Ok(mut ledger) = self.ledger.lock() {
            ledger.clear();
        }
    }
}

/// What a file is named to the terminal: its path, in base64.
pub fn payload(path: &std::path::Path) -> String {
    use base64::Engine;
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).to_vec();
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().into_owned().into_bytes();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(unix)]
mod unix {
    use std::collections::VecDeque;
    use std::fs::{self, DirBuilder, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime};

    use super::{MOST_BYTES, MOST_FILES, MOST_TILE, payload};

    /// How much may wait for the terminal at once.
    #[derive(Clone, Copy)]
    pub(super) struct Caps {
        pub files: usize,
        pub bytes: usize,
        pub tile: usize,
    }

    const CAPS: Caps = Caps {
        files: MOST_FILES,
        bytes: MOST_BYTES,
        tile: MOST_TILE,
    };

    /// Every file's name has this in it: Ghostty takes no temporary file without it.
    pub(super) const NAMED: &str = "tty-graphics-protocol-";
    /// Each run's folder, inside the user's.
    const SESSION: &str = "session-";
    /// How long the terminal has to take a file before files are given up for the run. It
    /// reads them as it reads the frame, so one still there after this never will be.
    pub(super) const TAKEN_WITHIN: Duration = Duration::from_secs(10);
    /// How long after its answer the probe's file has to be gone.
    const PROBE_TAKEN_WITHIN: Duration = Duration::from_millis(100);
    /// A run's folder that nobody holds is cleared at the next start once it's this old: a
    /// terminal may still be reading what a Cellix that just ended left.
    pub(super) const STALE: Duration = Duration::from_secs(60);
    /// Runs' folders there can be at once, live or left. Past this tiles go inline, so runs
    /// that crash one after another can't fill the disk.
    pub(super) const MOST_SESSIONS: usize = 16;

    pub(super) fn effective_uid() -> u32 {
        // SAFETY: geteuid takes nothing and can't fail.
        unsafe { libc::geteuid() }
    }

    /// The user's own temp folder: `TMPDIR` where it's a folder only they can write to, as
    /// macOS gives each user, or else `/tmp`. Ghostty takes a temporary file from either, and
    /// from its own `TMPDIR`, which is this one when Cellix was started inside it.
    pub(super) fn temp_root(tmpdir: Option<String>) -> PathBuf {
        let own = tmpdir.map(PathBuf::from).filter(|dir| {
            dir.is_absolute()
                && fs::metadata(dir).is_ok_and(|meta| {
                    meta.is_dir() && meta.uid() == effective_uid() && meta.mode() & 0o022 == 0
                })
        });
        own.unwrap_or_else(|| PathBuf::from("/tmp"))
    }

    struct Waiting {
        path: PathBuf,
        bytes: usize,
        since: Instant,
    }

    enum Probe {
        NotAsked,
        Asked(PathBuf),
        Taken,
    }

    /// This run's folder, locked for as long as the run holds it, so another Cellix's start
    /// can tell it from one left behind.
    struct Session {
        path: PathBuf,
        _lock: File,
    }

    pub(super) struct Ledger {
        root: PathBuf,
        session: Option<Session>,
        /// In the order the terminal was told them, which is the order it reads them.
        waiting: VecDeque<Waiting>,
        bytes: usize,
        /// Given up for the rest of the run.
        off: bool,
        probe: Probe,
        random: Option<File>,
        pub(super) caps: Caps,
    }

    impl Ledger {
        pub(super) fn new(root: PathBuf) -> Self {
            Self {
                root,
                session: None,
                waiting: VecDeque::new(),
                bytes: 0,
                off: false,
                probe: Probe::NotAsked,
                random: None,
                caps: CAPS,
            }
        }

        #[cfg(test)]
        pub(super) fn taken(root: PathBuf) -> Self {
            let mut ledger = Self::new(root);
            ledger.probe = Probe::Taken;
            ledger
        }

        pub(super) fn on(&self) -> bool {
            !self.off && matches!(self.probe, Probe::Taken)
        }

        pub(super) fn folder(&self) -> Option<PathBuf> {
            self.session.as_ref().map(|session| session.path.clone())
        }

        pub(super) fn question(&mut self, id: u32) -> Option<String> {
            if self.off || !matches!(self.probe, Probe::NotAsked) {
                return None;
            }
            // One pixel of RGB, as the question says.
            match self.write_file(&[0, 0, 0], SystemTime::now()) {
                Ok(path) => {
                    let name = payload(&path);
                    self.probe = Probe::Asked(path);
                    Some(format!("\x1b_Gi={id},s=1,v=1,a=q,t=t,f=24;{name}\x1b\\"))
                }
                Err(_) => {
                    self.off = true;
                    None
                }
            }
        }

        pub(super) fn answered(&mut self, ok: bool) -> bool {
            let Probe::Asked(path) = std::mem::replace(&mut self.probe, Probe::NotAsked) else {
                self.off = true;
                return false;
            };
            let deadline = Instant::now() + PROBE_TAKEN_WITHIN;
            let mut taken = gone(&path);
            while ok && !taken && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
                taken = gone(&path);
            }
            if ok && taken {
                self.probe = Probe::Taken;
                return true;
            }
            // Answered and done with, or never coming: either way nothing will read it now.
            let _ = fs::remove_file(&path);
            self.off = true;
            false
        }

        pub(super) fn write(&mut self, pixels: &[u8], now: Instant) -> Option<PathBuf> {
            if !self.on() || pixels.is_empty() || pixels.len() > self.caps.tile {
                return None;
            }
            self.forget_taken(now);
            let full = self.waiting.len() >= self.caps.files
                || self.bytes + pixels.len() > self.caps.bytes;
            if self.off || full {
                return None;
            }
            match self.write_file(pixels, SystemTime::now()) {
                Ok(path) => {
                    self.bytes += pixels.len();
                    self.waiting.push_back(Waiting {
                        path: path.clone(),
                        bytes: pixels.len(),
                        since: now,
                    });
                    Some(path)
                }
                Err(_) => {
                    // Not again every frame: whatever it was, inline works.
                    self.off = true;
                    None
                }
            }
        }

        /// Lets go of the files the terminal has taken, from the oldest, and gives files up
        /// if the oldest it hasn't is too old. What it's left stays until the run ends: the
        /// terminal could still open it.
        fn forget_taken(&mut self, now: Instant) {
            while let Some(oldest) = self.waiting.front() {
                if !gone(&oldest.path) {
                    if now.saturating_duration_since(oldest.since) >= TAKEN_WITHIN {
                        self.off = true;
                    }
                    return;
                }
                self.bytes -= oldest.bytes;
                self.waiting.pop_front();
            }
        }

        /// A new file holding `bytes`, closed. Nothing is overwritten, and nothing outside
        /// this run's folder is written.
        fn write_file(&mut self, bytes: &[u8], now: SystemTime) -> io::Result<PathBuf> {
            if self.session.is_none() {
                self.session = Some(self.make_session(now)?);
            }
            let name = format!("{NAMED}{}", hex(&self.random_bytes::<8>()?));
            let path = self.session.as_ref().expect("made above").path.join(name);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)?;
            let written = file.write_all(bytes);
            // Closed before the terminal hears of it. No fsync: it reads the same page cache.
            drop(file);
            if let Err(error) = written {
                let _ = fs::remove_file(&path);
                return Err(error);
            }
            Ok(path)
        }

        fn random_bytes<const N: usize>(&mut self) -> io::Result<[u8; N]> {
            if self.random.is_none() {
                self.random = Some(File::open("/dev/urandom")?);
            }
            let mut bytes = [0; N];
            self.random
                .as_mut()
                .expect("opened above")
                .read_exact(&mut bytes)?;
            Ok(bytes)
        }

        /// The user's folder in the temp folder, made if it isn't there, then this run's in
        /// it: after clearing what runs that ended without clearing up left there.
        fn make_session(&mut self, now: SystemTime) -> io::Result<Session> {
            let parent = self.root.join(format!("cellix-tiles-{}", effective_uid()));
            match DirBuilder::new().mode(0o700).create(&parent) {
                Ok(()) => fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            // Ours and private, and not a link to somewhere else, or nothing goes in it.
            open_private(&parent)?;
            if sweep(&parent, now)? >= MOST_SESSIONS {
                return Err(io::Error::other("too many runs' tile folders"));
            }
            for _ in 0..4 {
                let path = parent.join(format!("{SESSION}{}", hex(&self.random_bytes::<16>()?)));
                match DirBuilder::new().mode(0o700).create(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
                let held = fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                    .and_then(|()| open_private(&path))
                    .and_then(|lock| lock_now(&lock).map(|()| lock));
                return match held {
                    Ok(lock) => Ok(Session { path, _lock: lock }),
                    Err(error) => {
                        let _ = fs::remove_dir(&path);
                        Err(error)
                    }
                };
            }
            Err(io::Error::other("no free name for a tile folder"))
        }

        pub(super) fn clear(&mut self) {
            self.off = true;
            for waiting in self.waiting.drain(..) {
                let _ = fs::remove_file(&waiting.path);
            }
            self.bytes = 0;
            if let Probe::Asked(path) = std::mem::replace(&mut self.probe, Probe::NotAsked) {
                let _ = fs::remove_file(path);
            }
            if let Some(session) = self.session.take() {
                // Never recursive: anything else in it stays, for the next start to judge.
                let _ = fs::remove_dir(&session.path);
            }
        }

        #[cfg(test)]
        pub(super) fn crash(&mut self) {
            // As a run killed outright leaves it: files and folder there, and the lock let go.
            self.waiting.clear();
            self.session = None;
            self.off = true;
        }
    }

    impl Drop for Ledger {
        fn drop(&mut self) {
            self.clear();
        }
    }

    fn gone(path: &Path) -> bool {
        // Only "not there" is taken: a permission or I/O error says nothing of the terminal.
        matches!(fs::symlink_metadata(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Opens a folder without following a link, and only if it's the user's and private.
    pub(super) fn open_private(path: &Path) -> io::Result<File> {
        let folder = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?;
        let meta = folder.metadata()?;
        if !meta.is_dir() || meta.uid() != effective_uid() || meta.mode() & 0o7777 != 0o700 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "a tile folder that isn't private",
            ));
        }
        Ok(folder)
    }

    /// Takes the folder's lock, or says another run holds it.
    pub(super) fn lock_now(folder: &File) -> io::Result<()> {
        // SAFETY: the descriptor is the open file's own; flock doesn't take it or read memory.
        // The lock goes when the file is closed.
        if unsafe { libc::flock(folder.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Clears the runs' folders in `parent` that no run holds and that are older than `STALE`,
    /// and gives how many are left. A folder holding anything but tile files is left whole.
    pub(super) fn sweep(parent: &Path, now: SystemTime) -> io::Result<usize> {
        let mut left = 0;
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let name = entry.file_name();
            let ours = name
                .to_str()
                .and_then(|n| n.strip_prefix(SESSION))
                .is_some_and(|rest| {
                    rest.len() == 32 && rest.bytes().all(|b| b.is_ascii_hexdigit())
                });
            if !ours {
                continue;
            }
            if !clear_left(&entry.path(), now) {
                left += 1;
            }
            if left > MOST_SESSIONS {
                break;
            }
        }
        Ok(left)
    }

    /// Whether a run's folder was left and is now gone.
    fn clear_left(folder: &Path, now: SystemTime) -> bool {
        let Ok(lock) = open_private(folder) else {
            return false;
        };
        if lock_now(&lock).is_err() {
            return false;
        }
        let old = lock
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE);
        if !old {
            return false;
        }
        let Ok(entries) = fs::read_dir(folder) else {
            return false;
        };
        let mut files = Vec::new();
        for entry in entries.take(MOST_FILES + 2) {
            let Ok(entry) = entry else {
                return false;
            };
            let named = entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_prefix(NAMED))
                .is_some_and(|rest| {
                    rest.len() == 16 && rest.bytes().all(|b| b.is_ascii_hexdigit())
                });
            let plain = fs::symlink_metadata(entry.path()).is_ok_and(|meta| {
                meta.is_file() && meta.uid() == effective_uid() && meta.nlink() == 1
            });
            if !named || !plain {
                return false;
            }
            files.push(entry.path());
        }
        if files.len() > MOST_FILES + 1 {
            return false;
        }
        files.iter().all(|file| fs::remove_file(file).is_ok()) && fs::remove_dir(folder).is_ok()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::unix::*;
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;
    use std::time::{Duration, Instant, SystemTime};

    /// A private folder of the test's own to stand for the temp folder, gone after.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let mut random = [0u8; 8];
            use std::io::Read;
            fs::File::open("/dev/urandom")
                .unwrap()
                .read_exact(&mut random)
                .unwrap();
            let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
            let path = std::env::temp_dir().join(format!("cellix-files-test-{name}"));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }

        fn parent(&self) -> PathBuf {
            self.0.join(format!("cellix-tiles-{}", effective_uid()))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn ghostty(extra: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let mut vars: Vec<(String, String)> = vec![("TERM_PROGRAM".into(), "ghostty".into())];
        vars.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        move |name: &str| {
            vars.iter()
                .rev()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        }
    }

    /// What Ghostty does with a file it's named: reads it, then deletes it.
    fn take(path: &Path) -> Vec<u8> {
        let bytes = fs::read(path).unwrap();
        fs::remove_file(path).unwrap();
        bytes
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().mode() & 0o7777
    }

    #[test]
    fn files_go_only_to_ghostty_on_this_machine_with_nothing_between() {
        assert!(allowed(ghostty(&[])));
        assert!(allowed(ghostty(&[("TERM_PROGRAM", "Ghostty")])));
        assert!(allowed(ghostty(&[(SWITCH, "1")])));
        assert!(
            !allowed(ghostty(&[(SWITCH, "0")])),
            "the switch turns it off"
        );
        for elsewhere in ELSEWHERE {
            assert!(!allowed(ghostty(&[(elsewhere, "1")])), "{elsewhere}");
            // Set but empty still says something's between.
            assert!(!allowed(ghostty(&[(elsewhere, "")])), "{elsewhere} empty");
        }
        for other in ["kitty", "WezTerm", "iTerm.app", "vscode", ""] {
            assert!(!allowed(ghostty(&[("TERM_PROGRAM", other)])), "{other}");
        }
        assert!(!allowed(|_: &str| None), "a terminal that doesn't say");
        assert!(TileFiles::for_terminal(ghostty(&[("SSH_TTY", "/dev/ttys001")])).is_none());
    }

    #[test]
    fn the_folder_is_the_users_own_temp_folder_when_its_private() {
        let scratch = Scratch::new();
        let own = scratch.0.to_str().unwrap().to_string();
        assert_eq!(temp_root(Some(own.clone())), scratch.0);
        assert_eq!(temp_root(Some(format!("{own}/"))), scratch.0.join(""));
        // One others can write to, one that isn't there, a relative one, or none: /tmp.
        fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(temp_root(Some(own.clone())), PathBuf::from("/tmp"));
        fs::set_permissions(&scratch.0, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            temp_root(Some(format!("{own}/missing"))),
            PathBuf::from("/tmp")
        );
        assert_eq!(temp_root(Some("tmp".into())), PathBuf::from("/tmp"));
        assert_eq!(temp_root(None), PathBuf::from("/tmp"));
    }

    #[test]
    fn a_tile_file_is_private_whole_and_unpredictably_named() {
        let scratch = Scratch::new();
        let files = TileFiles::for_test(scratch.0.clone());
        let pixels: Vec<u8> = (0..=255).cycle().take(3 * 40 * 7).collect();
        let first = files.write(&pixels).unwrap();
        let second = files.write(b"\x01\x02\x03").unwrap();
        assert_ne!(first, second);
        let session = first.parent().unwrap();
        assert_eq!(session, files.folder().unwrap());
        assert_eq!(session.parent().unwrap(), scratch.parent());
        // The user's folder and the run's are 0700, the file 0600, all theirs, none a link.
        for (path, want) in [(&scratch.parent(), 0o700), (&session.into(), 0o700)] {
            assert_eq!(mode(path), want, "{path:?}");
        }
        assert_eq!(mode(&first), 0o600);
        let meta = fs::symlink_metadata(&first).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.uid(), effective_uid());
        // Ghostty takes nothing without this in the name; the rest is 64 random bits.
        let name = first.file_name().unwrap().to_str().unwrap();
        let random = name.strip_prefix(NAMED).unwrap();
        assert_eq!(random.len(), 16);
        assert!(random.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(take(&first), pixels, "written whole before it was named");
        assert_eq!(take(&second), b"\x01\x02\x03");
        // And named to the terminal as its path in base64.
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(payload(&first))
            .unwrap();
        assert_eq!(Path::new(std::str::from_utf8(&decoded).unwrap()), first);
    }

    #[test]
    fn a_parent_folder_that_isnt_private_or_is_a_link_takes_no_tiles() {
        let scratch = Scratch::new();
        let elsewhere = scratch.0.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, scratch.parent()).unwrap();
        let files = TileFiles::for_test(scratch.0.clone());
        assert_eq!(files.write(b"abc"), None, "not through a link");
        assert!(!files.on(), "and off for good");
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        fs::remove_file(scratch.parent()).unwrap();
        fs::create_dir(scratch.parent()).unwrap();
        fs::set_permissions(scratch.parent(), fs::Permissions::from_mode(0o755)).unwrap();
        let files = TileFiles::for_test(scratch.0.clone());
        assert_eq!(files.write(b"abc"), None, "not where others can look");
        assert_eq!(fs::read_dir(scratch.parent()).unwrap().count(), 0);
    }

    #[test]
    fn nothing_goes_as_a_file_until_the_terminal_has_taken_the_probe() {
        let scratch = Scratch::new();
        let files = TileFiles::with_ledger(Ledger::new(scratch.0.clone()));
        assert!(!files.on());
        assert_eq!(files.write(b"abc"), None, "before the probe");
        let question = files.question(32).unwrap();
        assert!(question.starts_with("\x1b_Gi=32,s=1,v=1,a=q,t=t,f=24;"));
        assert_eq!(files.question(32), None, "one probe a run");
        // The path it names holds one pixel.
        use base64::Engine;
        let name = question
            .strip_prefix("\x1b_Gi=32,s=1,v=1,a=q,t=t,f=24;")
            .and_then(|rest| rest.strip_suffix("\x1b\\"))
            .unwrap();
        let path = PathBuf::from(
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(name)
                    .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(take(&path), [0, 0, 0]);
        assert!(files.answered(true));
        assert!(files.on());
        assert!(files.write(b"abc").is_some());
    }

    #[test]
    fn a_probe_unanswered_or_not_taken_keeps_tiles_inline_for_the_run() {
        // No answer: its file is removed, and nothing goes as a file.
        let scratch = Scratch::new();
        let files = TileFiles::with_ledger(Ledger::new(scratch.0.clone()));
        files.question(32).unwrap();
        let session = files.folder().unwrap();
        assert!(!files.answered(false));
        assert_eq!(fs::read_dir(&session).unwrap().count(), 0, "probe removed");
        assert_eq!(files.write(b"abc"), None);
        // Answered OK, but the file is still there: a terminal that doesn't read it.
        let files = TileFiles::with_ledger(Ledger::new(scratch.0.clone()));
        files.question(32).unwrap();
        let started = Instant::now();
        assert!(!files.answered(true));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "waited a moment"
        );
        assert_eq!(fs::read_dir(files.folder().unwrap()).unwrap().count(), 0);
        assert_eq!(files.write(b"abc"), None);
        // Never asked, as under tmux: off.
        let files = TileFiles::with_ledger(Ledger::new(scratch.0.clone()));
        assert!(!files.answered(true));
        assert_eq!(files.question(32), None);
    }

    #[test]
    fn past_a_cap_tiles_go_inline_until_the_terminal_takes_some() {
        // The caps in use, as documented, and small ones to reach here.
        assert_eq!(
            (MOST_FILES, MOST_BYTES, MOST_TILE),
            (1024, 64 << 20, 16 << 20)
        );
        let scratch = Scratch::new();
        let small = |files, bytes, tile| {
            let mut ledger = Ledger::taken(scratch.0.clone());
            ledger.caps = Caps { files, bytes, tile };
            TileFiles::with_ledger(ledger)
        };
        let files = small(3, 1000, 10);
        assert_eq!(files.write(&[]), None, "nothing to send");
        assert_eq!(files.write(&[7; 11]), None, "too big a tile");
        assert!(files.on(), "a big tile goes inline, and files stay on");
        let mut paths: Vec<PathBuf> = (0..3).map(|_| files.write(b"x").unwrap()).collect();
        assert_eq!(files.write(b"x"), None, "past the count");
        assert!(files.on());
        take(&paths.remove(0));
        assert!(
            files.write(b"x").is_some(),
            "room again once the oldest is taken"
        );
        let files = small(100, 25, 10);
        let mut held: Vec<PathBuf> = (0..2).map(|_| files.write(&[1; 10]).unwrap()).collect();
        assert_eq!(files.write(&[1; 6]), None, "past the bytes");
        assert!(files.write(&[1; 5]).is_some(), "what still fits goes");
        take(&held.remove(0));
        assert!(files.write(&[1; 10]).is_some());
    }

    #[test]
    fn a_file_the_terminal_never_takes_turns_files_off_but_isnt_pulled_from_under_it() {
        let scratch = Scratch::new();
        let mut ledger = Ledger::taken(scratch.0.clone());
        let start = Instant::now();
        let first = ledger.write(b"one", start).unwrap();
        assert!(
            ledger
                .write(b"two", start + Duration::from_secs(9))
                .is_some()
        );
        assert_eq!(ledger.write(b"three", start + TAKEN_WITHIN), None);
        assert!(!ledger.on(), "given up for the run");
        assert_eq!(fs::read(&first).unwrap(), b"one", "still there to be read");
        take(&first);
        assert_eq!(
            ledger.write(b"four", start + TAKEN_WITHIN),
            None,
            "for good"
        );
    }

    #[test]
    fn a_write_that_fails_turns_files_off_for_good() {
        let scratch = Scratch::new();
        // A temp folder that's a file: nothing can be made in it.
        let root = scratch.0.join("a-file");
        fs::write(&root, b"").unwrap();
        let files = TileFiles::for_test(root);
        assert_eq!(files.write(b"abc"), None);
        assert!(!files.on());
        // A run's folder taken away under it.
        let files = TileFiles::for_test(scratch.0.clone());
        let path = files.write(b"abc").unwrap();
        take(&path);
        fs::remove_dir(files.folder().unwrap()).unwrap();
        assert_eq!(files.write(b"abc"), None);
        assert!(!files.on());
    }

    #[test]
    fn the_end_of_a_run_clears_what_the_terminal_left() {
        let scratch = Scratch::new();
        let files = TileFiles::for_test(scratch.0.clone());
        let paths: Vec<PathBuf> = (0..3).map(|n| files.write(&[n; 9]).unwrap()).collect();
        take(&paths[0]);
        let session = files.folder().unwrap();
        let other = files.clone();
        drop(files);
        assert!(paths[1].exists(), "a clone keeps the ledger");
        other.clear();
        assert!(!paths[1].exists() && !paths[2].exists());
        assert!(!session.exists());
        assert!(scratch.parent().exists(), "the user's folder stays");
        assert_eq!(other.write(b"abc"), None, "and nothing more is written");
        // Dropping the last clone clears as well.
        let files = TileFiles::for_test(scratch.0.clone());
        let path = files.write(b"abc").unwrap();
        drop(files);
        assert!(!path.exists() && !path.parent().unwrap().exists());
    }

    #[test]
    fn the_next_start_clears_what_a_run_that_was_killed_left() {
        let scratch = Scratch::new();
        let mut killed = Ledger::taken(scratch.0.clone());
        let left = killed.write(b"left", Instant::now()).unwrap();
        let left_folder = left.parent().unwrap().to_path_buf();
        killed.crash();
        drop(killed);
        assert!(left.exists());
        // A run still going holds its folder.
        let mut live = Ledger::taken(scratch.0.clone());
        let live_file = live.write(b"live", Instant::now()).unwrap();
        let parent = scratch.parent();
        let later = SystemTime::now() + STALE + Duration::from_secs(1);
        // Just left, a terminal may still be reading it.
        assert_eq!(sweep(&parent, SystemTime::now()).unwrap(), 2);
        assert!(left.exists());
        assert_eq!(
            sweep(&parent, later).unwrap(),
            1,
            "only the live one is left"
        );
        assert!(!left.exists() && !left_folder.exists());
        assert!(live_file.exists(), "a live run's files are never touched");
        drop(live);
        assert_eq!(sweep(&parent, later).unwrap(), 0);
    }

    #[test]
    fn a_left_folder_with_anything_but_tile_files_is_left_alone() {
        let scratch = Scratch::new();
        let outside = scratch.0.join("outside");
        fs::write(&outside, b"keep").unwrap();
        let later = SystemTime::now() + STALE + Duration::from_secs(1);
        let make = || {
            let mut killed = Ledger::taken(scratch.0.clone());
            let path = killed.write(b"left", Instant::now()).unwrap();
            killed.crash();
            path
        };
        // A link where a tile was: neither it nor what it points at is removed.
        let tile = make();
        fs::remove_file(&tile).unwrap();
        std::os::unix::fs::symlink(&outside, &tile).unwrap();
        assert_eq!(sweep(&scratch.parent(), later).unwrap(), 1);
        assert!(fs::symlink_metadata(&tile).is_ok());
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
        fs::remove_file(&tile).unwrap();
        fs::remove_dir(tile.parent().unwrap()).unwrap();
        // Something that isn't a tile.
        let tile = make();
        let other = tile.parent().unwrap().join("notes.txt");
        fs::write(&other, b"keep").unwrap();
        assert_eq!(sweep(&scratch.parent(), later).unwrap(), 1);
        assert!(tile.exists() && other.exists());
        // A folder that isn't a run's.
        fs::create_dir(scratch.parent().join("mine")).unwrap();
        assert_eq!(sweep(&scratch.parent(), later).unwrap(), 1);
        assert!(scratch.parent().join("mine").exists());
    }

    #[test]
    fn too_many_runs_folders_and_tiles_go_inline() {
        let scratch = Scratch::new();
        // Runs still going, each holding its folder.
        let mut live = Vec::new();
        for _ in 0..MOST_SESSIONS {
            let mut ledger = Ledger::taken(scratch.0.clone());
            assert!(ledger.write(b"x", Instant::now()).is_some());
            live.push(ledger);
        }
        let files = TileFiles::for_test(scratch.0.clone());
        assert_eq!(files.write(b"x"), None);
        assert!(!files.on());
        drop(live);
        let files = TileFiles::for_test(scratch.0.clone());
        assert!(files.write(b"x").is_some(), "room once they've ended");
    }
}
