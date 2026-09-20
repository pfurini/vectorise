//! Error types and the process exit codes they map to.
//!
//! The library returns typed errors; the binary turns them into an
//! [`ExitCode`]. Nothing in `src/` panics: clippy denies `panic!` and
//! `unwrap`, and internal invariant violations become errors instead.

/// How the process reports its outcome to the shell.
///
/// The numeric values follow BSD `sysexits.h` where one applies, so shell
/// scripts can distinguish "the batch was rejected before anything was written"
/// from "some files failed after others were written".
///
/// # Examples
///
/// ```
/// use vectorise::error::ExitCode;
///
/// assert_eq!(u8::from(ExitCode::Ok), 0);
/// assert_eq!(u8::from(ExitCode::Usage), 64);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExitCode {
    /// Every input was converted.
    Ok = 0,
    /// One or more inputs failed; the others were written.
    Partial = 1,
    /// Preflight rejected the batch. Nothing was written.
    Preflight = 2,
    /// The command line itself was wrong (BSD `EX_USAGE`).
    Usage = 64,
    /// An I/O failure outside per-file conversion (BSD `EX_IOERR`), such as
    /// being unable to create `--output-dir`.
    Io = 74,
}

impl From<ExitCode> for u8 {
    fn from(code: ExitCode) -> Self {
        code as Self
    }
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        Self::from(u8::from(code))
    }
}

#[cfg(test)]
mod tests {
    use super::ExitCode;

    #[test]
    fn exit_code_values_match_the_cli_contract() {
        assert_eq!(u8::from(ExitCode::Ok), 0);
        assert_eq!(u8::from(ExitCode::Partial), 1);
        assert_eq!(u8::from(ExitCode::Preflight), 2);
        assert_eq!(u8::from(ExitCode::Usage), 64);
        assert_eq!(u8::from(ExitCode::Io), 74);
    }
}
