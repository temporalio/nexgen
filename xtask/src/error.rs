use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

use nexgen::language::Language;

pub(crate) type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Nexgen(#[from] nexgen::error::Error),

    #[error("invalid example configuration `{path}`: {reason}")]
    InvalidExampleConfig { path: PathBuf, reason: String },

    #[error(
        "refusing to delete example output path `{path}`: it is not a directory inside `{root}"
    )]
    ExampleOutputPathOutsideRoot { path: PathBuf, root: PathBuf },

    #[error("failed to run command `{command}` in `{cwd}`: {source}")]
    RunCommand {
        cwd: PathBuf,
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("command `{command}` failed in `{cwd}` with status {status}")]
    CommandFailed {
        cwd: PathBuf,
        command: String,
        status: ExitStatus,
    },

    #[error(
        "Markdown formatting check failed. Run `cargo fmt-markdown` to rewrap the files listed above, then commit the result."
    )]
    MarkdownUnformatted,

    #[error("unknown {language} example `{example_id}")]
    UnknownExampleId {
        language: Language,
        example_id: String,
    },
}
