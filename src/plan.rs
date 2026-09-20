//! Turn argv into a validated [`Plan`], or into the complete list of reasons
//! the batch is rejected.
//!
//! Three steps, in order:
//!
//! 1. **Resolution** — each argument becomes zero or more input paths. An
//!    argument that names an existing file is taken literally; otherwise, if it
//!    looks like a glob, it is expanded here (the shell normally does this
//!    first).
//! 2. **Mapping** — each input gets its output path: same directory and stem
//!    with an `.svg` extension, or flattened into `--output-dir`.
//! 3. **Preflight** — the whole batch is checked before anything is written.
//!
//! Preflight reports *every* problem it finds, not the first, so one run tells
//! the user everything they have to fix. A batch with any problem writes
//! nothing at all.
//!
//! All filesystem access goes through the [`Fs`] trait, so the tests exercise
//! the logic against an in-memory filesystem and touch the disk only where the
//! disk is the thing under test.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

/// Raster extensions we accept, lowercase. Compared case-insensitively.
const SUPPORTED_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff"];

/// The extension every output carries.
const OUTPUT_EXTENSION: &str = "svg";

/// Characters that make an argument worth trying as a glob pattern.
const GLOB_METACHARACTERS: [char; 3] = ['*', '?', '['];

/// What the caller asked for, as far as planning is concerned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanOptions {
    /// Write every output into this directory instead of next to its input.
    pub output_dir: Option<PathBuf>,
    /// Overwrite outputs that already exist.
    ///
    /// This never makes a batch with duplicate outputs valid: two inputs that
    /// map to one output are a mistake, not an overwrite.
    pub force: bool,
}

/// One input file and the output it will produce.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Job {
    /// The raster file to read, exactly as the user gave it.
    pub input: PathBuf,
    /// The SVG file to write.
    pub output: PathBuf,
}

/// A validated batch. Every job's output is free of conflicts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Jobs in a deterministic order (sorted by input path).
    pub jobs: Vec<Job>,
}

/// One reason a batch was rejected.
///
/// `Display` renders the machine-readable form the CLI prints: a kind, then
/// tab-separated paths, one problem per line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlanProblem {
    /// The output file already exists and `--force` was not given.
    OutputExists {
        /// The output that is in the way.
        output: PathBuf,
        /// The input that would have written it.
        input: PathBuf,
    },
    /// Two or more inputs map to the same output.
    DuplicateOutput {
        /// The contested output path.
        output: PathBuf,
        /// Every input that maps to it, sorted.
        inputs: Vec<PathBuf>,
    },
    /// The argument named a path that does not exist.
    InputNotFound(PathBuf),
    /// The argument named a directory. v1 does not recurse implicitly; use a
    /// glob such as `'dir/**/*.png'`.
    InputIsDirectory(PathBuf),
    /// The argument named a file whose extension is not a raster format we read.
    UnsupportedExtension(PathBuf),
    /// A glob pattern matched no supported raster file.
    GlobMatchedNothing(String),
    /// `--output-dir` exists but cannot be written to, or is not a directory.
    OutputDirNotWritable(PathBuf),
}

impl PlanProblem {
    /// The stable machine-readable kind, used as the first field of a line.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::OutputExists { .. } => "output-exists",
            Self::DuplicateOutput { .. } => "duplicate-output",
            Self::InputNotFound(_) => "input-not-found",
            Self::InputIsDirectory(_) => "input-is-directory",
            Self::UnsupportedExtension(_) => "unsupported-extension",
            Self::GlobMatchedNothing(_) => "glob-matched-nothing",
            Self::OutputDirNotWritable(_) => "output-dir-not-writable",
        }
    }
}

impl fmt::Display for PlanProblem {
    /// `KIND\tPATH[\tPATH...]`, so `cut -f1` and `cut -f2` work.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind())?;
        match self {
            Self::OutputExists { output, input } => {
                write!(f, "\t{}\t{}", output.display(), input.display())
            }
            Self::DuplicateOutput { output, inputs } => {
                write!(f, "\t{}", output.display())?;
                for input in inputs {
                    write!(f, "\t{}", input.display())?;
                }
                Ok(())
            }
            Self::InputNotFound(path)
            | Self::InputIsDirectory(path)
            | Self::UnsupportedExtension(path)
            | Self::OutputDirNotWritable(path) => write!(f, "\t{}", path.display()),
            Self::GlobMatchedNothing(pattern) => write!(f, "\t{pattern}"),
        }
    }
}

/// Every reason the batch was rejected. Never empty.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct PlanError {
    /// The problems, in a deterministic order.
    pub problems: Vec<PlanProblem>,
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, problem) in self.problems.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{problem}")?;
        }
        Ok(())
    }
}

/// The filesystem, as planning sees it.
///
/// Planning asks questions and never writes, with one exception documented on
/// [`Fs::can_write_dir`].
pub trait Fs {
    /// Does anything exist at this path?
    fn exists(&self, path: &Path) -> bool;
    /// Is this an existing directory?
    fn is_dir(&self, path: &Path) -> bool;
    /// Is this an existing regular file?
    fn is_file(&self, path: &Path) -> bool;
    /// Can a new file be created in this directory?
    ///
    /// The real implementation answers by creating a temporary file and
    /// deleting it, which is the only portable way to account for permissions,
    /// ACLs, and read-only mounts. The directory's contents are unchanged
    /// afterwards.
    fn can_write_dir(&self, path: &Path) -> bool;
    /// Expand a glob pattern to the paths it matches, sorted.
    ///
    /// An unparsable pattern yields no matches; the caller turns that into
    /// [`PlanProblem::GlobMatchedNothing`], which is what the user needs to
    /// know either way.
    fn glob(&self, pattern: &str) -> Vec<PathBuf>;
}

/// The real filesystem.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

impl Fs for RealFs {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn can_write_dir(&self, path: &Path) -> bool {
        path.is_dir()
            && tempfile::Builder::new()
                .prefix(".vectorise-probe")
                .tempfile_in(path)
                .is_ok()
    }

    fn glob(&self, pattern: &str) -> Vec<PathBuf> {
        let Ok(paths) = glob::glob(pattern) else {
            return Vec::new();
        };
        let mut matches: Vec<PathBuf> = paths.filter_map(Result::ok).collect();
        matches.sort_unstable();
        matches
    }
}

/// Build a validated [`Plan`] from command-line arguments.
///
/// # Errors
///
/// Returns every problem found, never just the first. A returned
/// [`PlanError`] always carries at least one [`PlanProblem`].
pub fn plan(inputs: &[PathBuf], options: &PlanOptions) -> Result<Plan, PlanError> {
    plan_with(inputs, options, &RealFs)
}

/// [`plan`] against a caller-supplied filesystem.
///
/// # Errors
///
/// Same as [`plan`].
pub fn plan_with<F: Fs + ?Sized>(
    inputs: &[PathBuf],
    options: &PlanOptions,
    fs: &F,
) -> Result<Plan, PlanError> {
    let (resolved, mut problems) = resolve(inputs, fs);

    let jobs: Vec<Job> = resolved
        .into_iter()
        .map(|input| {
            let output = map_output(&input, options);
            Job { input, output }
        })
        .collect();

    problems.extend(preflight(&jobs, options, fs));

    if problems.is_empty() {
        Ok(Plan { jobs })
    } else {
        Err(PlanError { problems })
    }
}

/// Resolve arguments to input paths, collecting a problem for each argument
/// that resolves to nothing usable.
///
/// An argument that names an existing file is always taken literally, even when
/// it contains glob metacharacters: a file really called `*.png` is a file, not
/// a pattern. Only when the literal path does not exist is the argument tried
/// as a glob, and then only if it carries a metacharacter.
///
/// Glob matches are filtered to readable raster files. A pattern that matches
/// only directories or only `.txt` files is reported as matching nothing rather
/// than as a pile of unsupported-extension problems, because the user named a
/// wildcard, not those files.
fn resolve<F: Fs + ?Sized>(arguments: &[PathBuf], fs: &F) -> (Vec<PathBuf>, Vec<PlanProblem>) {
    let mut inputs: Vec<PathBuf> = Vec::new();
    let mut problems: Vec<PlanProblem> = Vec::new();

    for argument in arguments {
        if fs.is_dir(argument) {
            problems.push(PlanProblem::InputIsDirectory(argument.clone()));
        } else if fs.is_file(argument) {
            if is_supported_extension(argument.extension()) {
                inputs.push(argument.clone());
            } else {
                problems.push(PlanProblem::UnsupportedExtension(argument.clone()));
            }
        } else if let Some(pattern) = glob_pattern(argument) {
            let matched = expand_glob(pattern, fs);
            if matched.is_empty() {
                problems.push(PlanProblem::GlobMatchedNothing(pattern.to_owned()));
            } else {
                inputs.extend(matched);
            }
        } else {
            problems.push(PlanProblem::InputNotFound(argument.clone()));
        }
    }

    inputs.sort();
    inputs.dedup();
    (inputs, problems)
}

/// Expand one pattern to the raster files it names.
fn expand_glob<F: Fs + ?Sized>(pattern: &str, fs: &F) -> Vec<PathBuf> {
    fs.glob(pattern)
        .into_iter()
        .filter(|path| fs.is_file(path) && is_supported_extension(path.extension()))
        .collect()
}

/// The output path for one input.
///
/// Without `--output-dir` the output sits next to its input with the extension
/// replaced, which keeps every inner dot: `a.b.png` becomes `a.b.svg`. With
/// `--output-dir` the batch is flattened onto the stem alone (ADR-0002).
fn map_output(input: &Path, options: &PlanOptions) -> PathBuf {
    let Some(dir) = options.output_dir.as_ref() else {
        return input.with_extension(OUTPUT_EXTENSION);
    };
    let stem = input.file_stem().unwrap_or(input.as_os_str());
    let mut name = stem.to_os_string();
    name.push(".");
    name.push(OUTPUT_EXTENSION);
    dir.join(name)
}

/// Check the whole batch. Returns every problem found, in a stable order:
/// the output directory first, then existing outputs in job order, then
/// duplicate outputs sorted by output path.
fn preflight<F: Fs + ?Sized>(jobs: &[Job], options: &PlanOptions, fs: &F) -> Vec<PlanProblem> {
    let mut problems = Vec::new();

    // An absent `--output-dir` is not a problem: it is created before the first
    // output is written. One that exists but refuses a new file is.
    if let Some(dir) = options.output_dir.as_ref()
        && fs.exists(dir)
        && !fs.can_write_dir(dir)
    {
        problems.push(PlanProblem::OutputDirNotWritable(dir.clone()));
    }

    if !options.force {
        for job in jobs {
            if fs.exists(&job.output) {
                problems.push(PlanProblem::OutputExists {
                    output: job.output.clone(),
                    input: job.input.clone(),
                });
            }
        }
    }

    // `--force` deliberately has no effect here: two inputs racing for one
    // output is a mistake in the command line, and overwriting would silently
    // discard one of them.
    let mut by_output: BTreeMap<&Path, Vec<&Path>> = BTreeMap::new();
    for job in jobs {
        by_output
            .entry(job.output.as_path())
            .or_default()
            .push(job.input.as_path());
    }
    for (output, inputs) in by_output {
        if inputs.len() > 1 {
            let mut inputs: Vec<PathBuf> = inputs.into_iter().map(Path::to_path_buf).collect();
            inputs.sort();
            problems.push(PlanProblem::DuplicateOutput {
                output: output.to_path_buf(),
                inputs,
            });
        }
    }

    problems
}

/// Is this extension one we can decode?
fn is_supported_extension(extension: Option<&OsStr>) -> bool {
    extension.and_then(OsStr::to_str).is_some_and(|extension| {
        SUPPORTED_EXTENSIONS
            .iter()
            .any(|supported| extension.eq_ignore_ascii_case(supported))
    })
}

/// The argument as a glob pattern, if it is one.
///
/// A pattern must be valid UTF-8 (the glob crate takes `&str`) and must carry a
/// metacharacter, so an ordinary missing file is reported as missing rather than
/// as a pattern that matched nothing.
fn glob_pattern(argument: &Path) -> Option<&str> {
    argument
        .to_str()
        .filter(|argument| argument.contains(GLOB_METACHARACTERS))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeSet;

    use super::*;

    /// An in-memory filesystem: a set of file paths and a set of directory
    /// paths, plus the directories that refuse writes.
    #[derive(Debug, Default)]
    struct MemFs {
        files: BTreeSet<PathBuf>,
        dirs: BTreeSet<PathBuf>,
        unwritable: BTreeSet<PathBuf>,
    }

    impl MemFs {
        fn new(files: &[&str]) -> Self {
            let mut fs = Self::default();
            for file in files {
                fs.add_file(file);
            }
            fs
        }

        fn add_file(&mut self, path: &str) -> &mut Self {
            let path = PathBuf::from(path);
            let mut parent = path.parent();
            while let Some(dir) = parent {
                if dir.as_os_str().is_empty() {
                    break;
                }
                self.dirs.insert(dir.to_path_buf());
                parent = dir.parent();
            }
            self.files.insert(path);
            self
        }

        fn add_dir(&mut self, path: &str) -> &mut Self {
            self.dirs.insert(PathBuf::from(path));
            self
        }

        fn make_unwritable(&mut self, path: &str) -> &mut Self {
            self.dirs.insert(PathBuf::from(path));
            self.unwritable.insert(PathBuf::from(path));
            self
        }
    }

    impl Fs for MemFs {
        fn exists(&self, path: &Path) -> bool {
            self.files.contains(path) || self.dirs.contains(path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.dirs.contains(path)
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.contains(path)
        }

        fn can_write_dir(&self, path: &Path) -> bool {
            self.dirs.contains(path) && !self.unwritable.contains(path)
        }

        fn glob(&self, pattern: &str) -> Vec<PathBuf> {
            // `require_literal_separator` reproduces what `glob::glob` does on
            // a real tree: it walks one path component at a time, so `*` never
            // crosses a `/` while `**` still does.
            let options = glob::MatchOptions {
                case_sensitive: true,
                require_literal_separator: true,
                require_literal_leading_dot: false,
            };
            let Ok(pattern) = glob::Pattern::new(pattern) else {
                return Vec::new();
            };
            let mut matches: Vec<PathBuf> = self
                .files
                .iter()
                .chain(self.dirs.iter())
                .filter(|path| pattern.matches_path_with(path, options))
                .cloned()
                .collect();
            matches.sort_unstable();
            matches
        }
    }

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    fn opts() -> PlanOptions {
        PlanOptions::default()
    }

    fn into_dir(dir: &str) -> PlanOptions {
        PlanOptions {
            output_dir: Some(PathBuf::from(dir)),
            ..PlanOptions::default()
        }
    }

    // --- resolution ---------------------------------------------------------

    #[test]
    fn resolve_literal_existing_file_is_kept() {
        let fs = MemFs::new(&["a/logo.png"]);
        let (inputs, problems) = resolve(&paths(&["a/logo.png"]), &fs);
        assert_eq!(inputs, paths(&["a/logo.png"]));
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn resolve_glob_fallback_expands_when_literal_missing() {
        let fs = MemFs::new(&["a/one.png", "a/two.png", "a/note.txt"]);
        let (inputs, problems) = resolve(&paths(&["a/*.png"]), &fs);
        assert_eq!(inputs, paths(&["a/one.png", "a/two.png"]));
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn resolve_glob_fallback_not_used_when_literal_exists() {
        // A file really named `*.png`. The literal wins; the other PNGs are not
        // dragged in.
        let fs = MemFs::new(&["a/*.png", "a/one.png"]);
        let (inputs, problems) = resolve(&paths(&["a/*.png"]), &fs);
        assert_eq!(inputs, paths(&["a/*.png"]));
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn resolve_recursive_glob_double_star_expands() {
        let fs = MemFs::new(&["a/one.png", "a/deep/two.png", "a/deep/deeper/three.png"]);
        let (inputs, problems) = resolve(&paths(&["a/**/*.png"]), &fs);
        assert_eq!(
            inputs,
            paths(&["a/deep/deeper/three.png", "a/deep/two.png", "a/one.png"])
        );
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn resolve_glob_no_match_is_problem() {
        let fs = MemFs::new(&["a/note.txt"]);
        let (inputs, problems) = resolve(&paths(&["a/*.png"]), &fs);
        assert!(inputs.is_empty());
        assert_eq!(
            problems,
            vec![PlanProblem::GlobMatchedNothing("a/*.png".to_owned())]
        );
    }

    #[test]
    fn resolve_results_are_deduplicated_and_sorted() {
        let fs = MemFs::new(&["a/b.png", "a/a.png"]);
        let (inputs, problems) =
            resolve(&paths(&["a/b.png", "a/a.png", "a/b.png", "a/*.png"]), &fs);
        assert_eq!(inputs, paths(&["a/a.png", "a/b.png"]));
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn resolve_unsupported_extension_is_problem() {
        let fs = MemFs::new(&["a/note.txt", "a/drawing.svg", "a/README"]);
        let (inputs, problems) = resolve(&paths(&["a/note.txt", "a/drawing.svg", "a/README"]), &fs);
        assert!(inputs.is_empty());
        assert_eq!(
            problems,
            vec![
                PlanProblem::UnsupportedExtension(PathBuf::from("a/note.txt")),
                PlanProblem::UnsupportedExtension(PathBuf::from("a/drawing.svg")),
                PlanProblem::UnsupportedExtension(PathBuf::from("a/README")),
            ]
        );
    }

    #[test]
    fn resolve_extension_match_is_case_insensitive() {
        let fs = MemFs::new(&["a/one.PNG", "a/two.Jpeg", "a/three.TIFF"]);
        let (inputs, problems) = resolve(&paths(&["a/one.PNG", "a/two.Jpeg", "a/three.TIFF"]), &fs);
        assert_eq!(inputs, paths(&["a/one.PNG", "a/three.TIFF", "a/two.Jpeg"]));
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn resolve_directory_argument_is_problem() {
        let mut fs = MemFs::new(&["a/one.png"]);
        fs.add_dir("a");
        let (inputs, problems) = resolve(&paths(&["a"]), &fs);
        assert!(inputs.is_empty());
        assert_eq!(
            problems,
            vec![PlanProblem::InputIsDirectory(PathBuf::from("a"))]
        );
    }

    #[test]
    fn resolve_missing_literal_without_metacharacters_is_not_found() {
        let fs = MemFs::new(&["a/one.png"]);
        let (inputs, problems) = resolve(&paths(&["a/absent.png"]), &fs);
        assert!(inputs.is_empty());
        assert_eq!(
            problems,
            vec![PlanProblem::InputNotFound(PathBuf::from("a/absent.png"))]
        );
    }

    #[test]
    fn resolve_glob_matching_only_directories_is_no_match() {
        let mut fs = MemFs::new(&["a/sub/one.png"]);
        fs.add_dir("a/sub");
        let (inputs, problems) = resolve(&paths(&["a/*"]), &fs);
        assert!(inputs.is_empty());
        assert_eq!(
            problems,
            vec![PlanProblem::GlobMatchedNothing("a/*".to_owned())]
        );
    }

    // --- mapping ------------------------------------------------------------

    #[test]
    fn map_output_same_dir_same_stem_svg_extension() {
        assert_eq!(
            map_output(Path::new("a/b/logo.png"), &opts()),
            PathBuf::from("a/b/logo.svg")
        );
    }

    #[test]
    fn map_output_dir_flattens_into_given_directory() {
        assert_eq!(
            map_output(Path::new("a/b/logo.png"), &into_dir("out")),
            PathBuf::from("out/logo.svg")
        );
    }

    #[test]
    fn map_output_dir_preserves_stem_only_not_subpath() {
        // ADR-0002: `--output-dir` flattens. `a/b/c.png` never becomes
        // `out/a/b/c.svg`.
        assert_eq!(
            map_output(Path::new("deep/nested/path/c.png"), &into_dir("out")),
            PathBuf::from("out/c.svg")
        );
    }

    #[test]
    fn map_multi_dot_stem_keeps_inner_dots() {
        assert_eq!(
            map_output(Path::new("a.b.png"), &opts()),
            PathBuf::from("a.b.svg")
        );
        assert_eq!(
            map_output(Path::new("dir/a.b.png"), &into_dir("out")),
            PathBuf::from("out/a.b.svg")
        );
    }

    // --- preflight ----------------------------------------------------------

    fn jobs_for(inputs: &[&str], options: &PlanOptions) -> Vec<Job> {
        inputs
            .iter()
            .map(|input| {
                let input = PathBuf::from(input);
                let output = map_output(&input, options);
                Job { input, output }
            })
            .collect()
    }

    #[test]
    fn preflight_existing_output_rejects_batch() {
        let fs = MemFs::new(&["a/one.png", "a/one.svg"]);
        let problems = preflight(&jobs_for(&["a/one.png"], &opts()), &opts(), &fs);
        assert_eq!(
            problems,
            vec![PlanProblem::OutputExists {
                output: PathBuf::from("a/one.svg"),
                input: PathBuf::from("a/one.png"),
            }]
        );
    }

    #[test]
    fn preflight_reports_all_problems_not_just_first() {
        let fs = MemFs::new(&[
            "a/one.png",
            "a/one.svg",
            "a/two.png",
            "a/two.svg",
            "a/x.jpg",
            "a/x.png",
        ]);
        let jobs = jobs_for(&["a/one.png", "a/two.png", "a/x.jpg", "a/x.png"], &opts());
        let problems = preflight(&jobs, &opts(), &fs);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert_eq!(
            problems
                .iter()
                .filter(|p| matches!(p, PlanProblem::OutputExists { .. }))
                .count(),
            2
        );
        assert_eq!(
            problems
                .iter()
                .filter(|p| matches!(p, PlanProblem::DuplicateOutput { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn preflight_duplicate_stem_across_extensions_rejects() {
        let fs = MemFs::new(&["a/x.png", "a/x.jpg"]);
        let problems = preflight(&jobs_for(&["a/x.jpg", "a/x.png"], &opts()), &opts(), &fs);
        assert_eq!(
            problems,
            vec![PlanProblem::DuplicateOutput {
                output: PathBuf::from("a/x.svg"),
                inputs: paths(&["a/x.jpg", "a/x.png"]),
            }]
        );
    }

    #[test]
    fn preflight_duplicate_stem_across_dirs_with_output_dir_rejects() {
        let mut fs = MemFs::new(&["a/x.png", "b/x.png"]);
        fs.add_dir("out");
        let options = into_dir("out");
        let problems = preflight(&jobs_for(&["a/x.png", "b/x.png"], &options), &options, &fs);
        assert_eq!(
            problems,
            vec![PlanProblem::DuplicateOutput {
                output: PathBuf::from("out/x.svg"),
                inputs: paths(&["a/x.png", "b/x.png"]),
            }]
        );
    }

    #[test]
    fn preflight_force_allows_existing_output_but_still_rejects_duplicates() {
        let fs = MemFs::new(&["a/x.png", "a/x.jpg", "a/x.svg", "a/y.png", "a/y.svg"]);
        let forced = PlanOptions {
            force: true,
            ..PlanOptions::default()
        };
        let jobs = jobs_for(&["a/x.jpg", "a/x.png", "a/y.png"], &forced);
        let problems = preflight(&jobs, &forced, &fs);
        assert_eq!(
            problems,
            vec![PlanProblem::DuplicateOutput {
                output: PathBuf::from("a/x.svg"),
                inputs: paths(&["a/x.jpg", "a/x.png"]),
            }],
            "--force silences OutputExists but never DuplicateOutput"
        );
    }

    #[test]
    fn preflight_unwritable_output_dir_is_problem() {
        let mut fs = MemFs::new(&["a/one.png"]);
        fs.make_unwritable("out");
        let options = into_dir("out");
        let problems = preflight(&jobs_for(&["a/one.png"], &options), &options, &fs);
        assert_eq!(
            problems,
            vec![PlanProblem::OutputDirNotWritable(PathBuf::from("out"))]
        );
    }

    #[test]
    fn preflight_absent_output_dir_is_not_a_problem() {
        // The directory is created when the first output is written.
        let fs = MemFs::new(&["a/one.png"]);
        let options = into_dir("out");
        let problems = preflight(&jobs_for(&["a/one.png"], &options), &options, &fs);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn preflight_output_dir_that_is_a_file_is_problem() {
        let fs = MemFs::new(&["a/one.png", "out"]);
        let options = into_dir("out");
        let problems = preflight(&jobs_for(&["a/one.png"], &options), &options, &fs);
        assert_eq!(
            problems,
            vec![PlanProblem::OutputDirNotWritable(PathBuf::from("out"))]
        );
    }

    // --- plan ---------------------------------------------------------------

    #[test]
    fn plan_happy_path_produces_sorted_jobs() {
        let fs = MemFs::new(&["a/b.png", "a/a.png"]);
        let plan = plan_with(&paths(&["a/*.png"]), &opts(), &fs).expect("valid batch");
        assert_eq!(
            plan.jobs,
            vec![
                Job {
                    input: PathBuf::from("a/a.png"),
                    output: PathBuf::from("a/a.svg"),
                },
                Job {
                    input: PathBuf::from("a/b.png"),
                    output: PathBuf::from("a/b.svg"),
                },
            ]
        );
    }

    #[test]
    fn plan_reports_resolution_and_preflight_problems_together() {
        let fs = MemFs::new(&["a/one.png", "a/one.svg"]);
        let error =
            plan_with(&paths(&["a/one.png", "a/absent.png"]), &opts(), &fs).expect_err("rejected");
        assert_eq!(
            error.problems,
            vec![
                PlanProblem::InputNotFound(PathBuf::from("a/absent.png")),
                PlanProblem::OutputExists {
                    output: PathBuf::from("a/one.svg"),
                    input: PathBuf::from("a/one.png"),
                },
            ]
        );
    }

    #[test]
    fn plan_with_no_usable_inputs_is_an_error() {
        let fs = MemFs::new(&["a/note.txt"]);
        let error = plan_with(&paths(&["a/note.txt"]), &opts(), &fs).expect_err("rejected");
        assert_eq!(error.problems.len(), 1);
    }

    #[test]
    fn plan_error_is_never_empty() {
        let fs = MemFs::default();
        let error = plan_with(&paths(&["missing.png"]), &opts(), &fs).expect_err("rejected");
        assert!(!error.problems.is_empty());
    }

    #[test]
    fn plan_with_no_arguments_is_an_empty_batch_not_an_error() {
        // clap requires at least one INPUT, so this only happens through the
        // library API. Nothing to do is not a failure.
        let fs = MemFs::default();
        let plan = plan_with(&[], &opts(), &fs).expect("nothing to do");
        assert!(plan.jobs.is_empty());
    }

    #[test]
    fn plan_force_accepts_a_batch_whose_outputs_exist() {
        let fs = MemFs::new(&["a/one.png", "a/one.svg"]);
        let forced = PlanOptions {
            force: true,
            ..PlanOptions::default()
        };
        let plan = plan_with(&paths(&["a/one.png"]), &forced, &fs).expect("forced");
        assert_eq!(plan.jobs.len(), 1);
    }

    // --- rendering ----------------------------------------------------------

    #[test]
    fn problem_display_is_tab_separated_and_stable() {
        let problem = PlanProblem::OutputExists {
            output: PathBuf::from("a/one.svg"),
            input: PathBuf::from("a/one.png"),
        };
        assert_eq!(problem.to_string(), "output-exists\ta/one.svg\ta/one.png");

        let problem = PlanProblem::DuplicateOutput {
            output: PathBuf::from("a/x.svg"),
            inputs: paths(&["a/x.jpg", "a/x.png"]),
        };
        assert_eq!(
            problem.to_string(),
            "duplicate-output\ta/x.svg\ta/x.jpg\ta/x.png"
        );

        assert_eq!(
            PlanProblem::GlobMatchedNothing("a/*.png".to_owned()).to_string(),
            "glob-matched-nothing\ta/*.png"
        );
        assert_eq!(
            PlanProblem::InputIsDirectory(PathBuf::from("a")).to_string(),
            "input-is-directory\ta"
        );
    }

    #[test]
    fn plan_error_display_is_one_problem_per_line() {
        let error = PlanError {
            problems: vec![
                PlanProblem::InputNotFound(PathBuf::from("a.png")),
                PlanProblem::InputIsDirectory(PathBuf::from("b")),
            ],
        };
        assert_eq!(
            error.to_string(),
            "input-not-found\ta.png\ninput-is-directory\tb"
        );
    }

    // --- helpers ------------------------------------------------------------

    #[test]
    fn supported_extension_check_is_case_insensitive_and_rejects_unknown() {
        assert!(is_supported_extension(Some(OsStr::new("png"))));
        assert!(is_supported_extension(Some(OsStr::new("PNG"))));
        assert!(is_supported_extension(Some(OsStr::new("Jpeg"))));
        assert!(!is_supported_extension(Some(OsStr::new("svg"))));
        assert!(!is_supported_extension(Some(OsStr::new("txt"))));
        assert!(!is_supported_extension(None));
    }

    #[test]
    fn glob_detection_needs_a_metacharacter() {
        assert_eq!(glob_pattern(Path::new("a/*.png")), Some("a/*.png"));
        assert_eq!(glob_pattern(Path::new("a/**/b.png")), Some("a/**/b.png"));
        assert_eq!(glob_pattern(Path::new("a/?.png")), Some("a/?.png"));
        assert_eq!(glob_pattern(Path::new("a/[ab].png")), Some("a/[ab].png"));
        assert_eq!(glob_pattern(Path::new("a/b.png")), None);
    }

    // --- the real filesystem ------------------------------------------------

    #[test]
    fn real_fs_answers_about_an_actual_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("one.png");
        std::fs::write(&file, b"not really a png").expect("write");

        let fs = RealFs;
        assert!(fs.exists(&file));
        assert!(fs.is_file(&file));
        assert!(!fs.is_dir(&file));
        assert!(fs.is_dir(dir.path()));
        assert!(fs.can_write_dir(dir.path()));
        assert!(!fs.can_write_dir(&file));

        let pattern = format!("{}/*.png", dir.path().display());
        assert_eq!(fs.glob(&pattern), vec![file]);
    }

    #[test]
    fn real_fs_write_probe_leaves_the_directory_unchanged() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("one.png"), b"x").expect("write");

        let before = listing(dir.path());
        assert!(RealFs.can_write_dir(dir.path()));
        assert_eq!(listing(dir.path()), before);
    }

    #[cfg(unix)]
    #[test]
    fn real_fs_reports_a_read_only_directory_as_unwritable() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).expect("create");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).expect("chmod");

        let writable = RealFs.can_write_dir(&locked);

        // Restore before the assert so the tempdir can always be removed.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        assert!(!writable, "a 0o500 directory must not be reported writable");
    }

    fn listing(dir: &Path) -> BTreeSet<PathBuf> {
        std::fs::read_dir(dir)
            .expect("read_dir")
            .map(|entry| entry.expect("entry").path())
            .collect()
    }
}
