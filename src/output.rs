//! Write a file so that it either appears complete or does not appear at all.
//!
//! A conversion that is interrupted must not leave a half-written `.svg` where
//! a tool or a human will read it as finished. Every write therefore goes to a
//! temporary file in the **target directory**, is flushed and fsynced, and only
//! then is renamed into place. A rename within one directory is atomic on every
//! filesystem we target.
//!
//! The temporary file lives in the target directory rather than in `/tmp`
//! because a rename across filesystems is not a rename: it is a copy, and a
//! copy is not atomic.
//!
//! # Interruption
//!
//! The atomic rename guarantees that no *output* is ever partial. It does not
//! guarantee that no temporary file survives a hard kill: `SIGINT` and
//! `SIGKILL` both terminate the process without running destructors, so a file
//! being written at that instant stays behind. Temporary files are therefore
//! named `.vectorise-*` and hidden, so a leftover is obviously ours and
//! obviously junk.
//!
//! # Races
//!
//! Preflight checks that no output exists, but it runs before any conversion.
//! Something can create the file in between. Without `--force` the rename uses
//! `persist_noclobber`, which fails rather than overwriting, so the race is
//! reported instead of losing someone's file.
//!
//! # Permissions
//!
//! A temporary file is created private (mode 0600) on purpose, because its
//! contents are not yet meant to be read. An output file is not: it should look
//! like any other file the user created, which means 0666 masked by the
//! process umask. Rust exposes no way to read the umask without `unsafe`, so
//! the mode is probed once per process by creating an ordinary file and asking
//! the filesystem what it got.
//!
//! # Durability
//!
//! The file is fsynced before the rename, and on Linux the directory is fsynced
//! after it. Without the second fsync a crash can leave a directory entry that
//! points at nothing. macOS updates directory metadata as part of `rename`, so
//! the extra fsync is a Linux concern.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::OnceLock;

/// Prefix for in-flight output files. Hidden, and unmistakably ours, so a
/// leftover from a hard kill can be recognized and removed.
pub const TEMPORARY_PREFIX: &str = ".vectorise-";

/// Why an output could not be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The output already existed at the moment of the rename, and `--force`
    /// was not given. Preflight cannot prevent this; only the rename can.
    #[error("{path}: already exists")]
    AlreadyExists {
        /// The output we refused to overwrite.
        path: PathBuf,
    },
    /// Anything the filesystem refused.
    #[error("{path}: {source}")]
    Io {
        /// The output we were writing.
        path: PathBuf,
        /// What the filesystem said.
        source: io::Error,
    },
}

impl WriteError {
    /// The output this error is about.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::AlreadyExists { path } | Self::Io { path, .. } => path,
        }
    }

    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

/// Write `contents` to `path`, atomically.
///
/// Creates the parent directory if it does not exist. With `force`, an existing
/// file is replaced; without it, an existing file is an error.
///
/// # Errors
///
/// [`WriteError::AlreadyExists`] when the output appeared between preflight and
/// the rename, and [`WriteError::Io`] for anything the filesystem refused.
///
/// # Examples
///
/// ```
/// use vectorise::output::write_atomically;
///
/// let dir = tempfile::tempdir().expect("tempdir");
/// let path = dir.path().join("logo.svg");
///
/// write_atomically(&path, "<svg/>", false).expect("writes");
/// assert_eq!(std::fs::read_to_string(&path).expect("reads"), "<svg/>");
///
/// // A second write without `force` refuses rather than overwriting.
/// assert!(write_atomically(&path, "<svg/>", false).is_err());
/// assert!(write_atomically(&path, "<svg id=\"2\"/>", true).is_ok());
/// ```
pub fn write_atomically(path: &Path, contents: &str, force: bool) -> Result<(), WriteError> {
    write_atomically_with(path, force, |file| file.write_all(contents.as_bytes()))
}

/// [`write_atomically`], with the body supplied by a closure.
///
/// The closure exists so a test can fail mid-write and assert that neither a
/// partial output nor a temporary file survives.
///
/// # Errors
///
/// Same as [`write_atomically`], plus whatever `fill` returns, reported as
/// [`WriteError::Io`].
pub fn write_atomically_with<F>(path: &Path, force: bool, fill: F) -> Result<(), WriteError>
where
    F: FnOnce(&mut dyn Write) -> io::Result<()>,
{
    let directory = parent_of(path);
    std::fs::create_dir_all(directory).map_err(|source| WriteError::io(path, source))?;

    // `NamedTempFile` removes the file when it is dropped, so every early
    // return below leaves the directory as it found it.
    let mut temporary = tempfile::Builder::new()
        .prefix(TEMPORARY_PREFIX)
        .tempfile_in(directory)
        .map_err(|source| WriteError::io(path, source))?;

    fill(temporary.as_file_mut()).map_err(|source| WriteError::io(path, source))?;
    temporary
        .as_file_mut()
        .sync_all()
        .map_err(|source| WriteError::io(path, source))?;
    relax_permissions(temporary.as_file());

    let persisted = if force {
        temporary
            .persist(path)
            .map(|_| ())
            .map_err(|error| error.error)
    } else {
        temporary
            .persist_noclobber(path)
            .map(|_| ())
            .map_err(|error| error.error)
    };
    persisted.map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            WriteError::AlreadyExists {
                path: path.to_path_buf(),
            }
        } else {
            WriteError::io(path, source)
        }
    })?;

    sync_directory(directory);
    Ok(())
}

/// Give the file the permissions an ordinary output would have.
///
/// `NamedTempFile` creates at 0600, which is right for a temporary file and
/// wrong for the SVG it becomes. A failure here is cosmetic, so it is ignored.
#[cfg(unix)]
fn relax_permissions(file: &File) {
    use std::os::unix::fs::PermissionsExt as _;

    let _ = file.set_permissions(std::fs::Permissions::from_mode(default_file_mode()));
}

#[cfg(not(unix))]
const fn relax_permissions(_file: &File) {}

/// The mode an ordinary new file gets on this machine: 0666 masked by umask.
///
/// Probed once, by creating a file and asking. The umask cannot change under
/// us in a way that matters, and the alternative is a `libc::umask` call this
/// crate forbids.
#[cfg(unix)]
fn default_file_mode() -> u32 {
    static MODE: OnceLock<u32> = OnceLock::new();
    *MODE.get_or_init(|| probe_file_mode().unwrap_or(0o644))
}

/// Create a file the ordinary way and report the permissions it ended up with.
#[cfg(unix)]
fn probe_file_mode() -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;

    let path = std::env::temp_dir().join(format!(".vectorise-umask-probe-{}", std::process::id()));
    let file = File::create(&path).ok()?;
    let mode = file.metadata().ok()?.permissions().mode() & 0o7777;
    drop(file);
    let _ = std::fs::remove_file(&path);
    Some(mode)
}

/// The directory an output lives in. A bare filename lives in the current one.
fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Flush the directory entry, on the platforms where `rename` does not.
///
/// A failure here costs durability across a crash, not correctness, so it is
/// logged rather than returned: the file is already in place and the conversion
/// succeeded.
///
/// macOS updates directory metadata as part of `rename`, so there is nothing
/// to do and the function compiles away.
#[cfg(target_os = "linux")]
fn sync_directory(directory: &Path) {
    match File::open(directory).and_then(|handle| handle.sync_all()) {
        Ok(()) => {}
        Err(error) => {
            tracing::debug!(
                directory = %directory.display(),
                %error,
                "could not fsync the output directory"
            );
        }
    }
}

#[cfg(not(target_os = "linux"))]
const fn sync_directory(_directory: &Path) {}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use std::collections::BTreeSet;
    use std::io::{self, Write};
    use std::path::{Path, PathBuf};

    use super::{WriteError, write_atomically, write_atomically_with};

    /// Everything in a directory, so a test can assert it is unchanged.
    fn listing(directory: &Path) -> BTreeSet<PathBuf> {
        std::fs::read_dir(directory)
            .expect("read_dir")
            .map(|entry| entry.expect("entry").path())
            .collect()
    }

    #[test]
    fn output_writes_the_whole_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("one.svg");

        write_atomically(&path, "<svg/>", false).expect("writes");
        assert_eq!(std::fs::read_to_string(&path).expect("reads"), "<svg/>");
        assert_eq!(listing(dir.path()), BTreeSet::from([path]));
    }

    #[test]
    fn output_creates_the_parent_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/deeper/one.svg");

        write_atomically(&path, "<svg/>", false).expect("writes");
        assert!(path.is_file());
    }

    #[test]
    fn output_write_is_atomic_no_partial_file_on_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("one.svg");
        let before = listing(dir.path());

        let error = write_atomically_with(&path, false, |file: &mut dyn Write| {
            // Write something, then fail: exactly the case a naive
            // `File::create` plus `write_all` would leave half done.
            file.write_all(b"<svg xmlns=")?;
            Err(io::Error::other("the writer gave up"))
        })
        .expect_err("the write fails");

        assert!(matches!(error, WriteError::Io { .. }), "{error:?}");
        assert!(!path.exists(), "no partial output");
        assert_eq!(
            listing(dir.path()),
            before,
            "and no temporary file left behind"
        );
    }

    #[test]
    fn output_uses_create_new_semantics_to_beat_toctou() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("one.svg");
        // Someone else got there between preflight and now.
        std::fs::write(&path, "theirs").expect("write");

        let error = write_atomically(&path, "ours", false).expect_err("refuses");
        assert!(
            matches!(&error, WriteError::AlreadyExists { path: reported } if reported == &path),
            "{error:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("reads"),
            "theirs",
            "their file is untouched"
        );
        assert_eq!(error.path(), path);
        assert_eq!(listing(dir.path()), BTreeSet::from([path]));
    }

    #[test]
    fn output_force_replaces_an_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("one.svg");
        std::fs::write(&path, "old").expect("write");

        write_atomically(&path, "new", true).expect("overwrites");
        assert_eq!(std::fs::read_to_string(&path).expect("reads"), "new");
        assert_eq!(listing(dir.path()), BTreeSet::from([path]));
    }

    #[test]
    fn output_reports_the_path_on_an_unwritable_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A file where a directory should be: the parent cannot be created.
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, "not a directory").expect("write");

        let path = blocker.join("one.svg");
        let error = write_atomically(&path, "<svg/>", false).expect_err("refuses");
        assert!(matches!(error, WriteError::Io { .. }), "{error:?}");
        assert_eq!(error.path(), path);
    }

    #[cfg(unix)]
    #[test]
    fn output_has_the_permissions_an_ordinary_file_would() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");

        // What `File::create` produces here is the yardstick: same umask, same
        // filesystem.
        let reference = dir.path().join("reference");
        std::fs::File::create(&reference).expect("create");
        let expected = std::fs::metadata(&reference)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;

        let path = dir.path().join("one.svg");
        write_atomically(&path, "<svg/>", false).expect("writes");
        let actual = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(
            actual, expected,
            "an output must not be more private than any other file"
        );
        assert_ne!(actual, 0o600, "the temporary file's mode must not survive");
    }

    #[test]
    fn output_refuses_to_rename_over_a_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("one.svg");
        std::fs::create_dir(&path).expect("mkdir");
        std::fs::write(path.join("inside"), "something").expect("write");

        // Which error the kernel reports differs: macOS says ENOTEMPTY, which
        // Rust maps to `AlreadyExists`, and Linux says EISDIR, which it does
        // not. Either way the rename fails and the directory survives.
        let error = write_atomically(&path, "<svg/>", false).expect_err("refuses");
        assert_eq!(error.path(), path, "{error:?}");
        assert!(path.join("inside").is_file(), "the directory is untouched");
    }

    #[test]
    fn output_temporary_files_are_hidden_and_identifiable() {
        assert!(super::TEMPORARY_PREFIX.starts_with('.'), "hidden");
        assert!(
            super::TEMPORARY_PREFIX.contains("vectorise"),
            "recognizable as ours"
        );
    }

    #[test]
    fn output_writes_a_bare_filename_into_the_current_directory() {
        // `parent_of` has to turn an empty parent into ".".
        assert_eq!(super::parent_of(Path::new("one.svg")), Path::new("."));
        assert_eq!(super::parent_of(Path::new("a/one.svg")), Path::new("a"));
        assert_eq!(super::parent_of(Path::new("/one.svg")), Path::new("/"));
    }
}
