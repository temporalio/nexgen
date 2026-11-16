use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// Rewraps the repository's Markdown files in place according to `.rumdl.toml`.
pub fn fmt_markdown() -> Result<()> {
    let repo_root = repo_root();
    let rumdl = Path::new("rumdl");
    let args = ["fmt", "."];
    let status = Command::new(&rumdl)
        .current_dir(&repo_root)
        .args(args)
        .status()
        .map_err(|source| Error::RunCommand {
            cwd: repo_root.clone(),
            command: format_rumdl_command(&rumdl, &args),
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed {
            cwd: repo_root,
            command: format_rumdl_command(&rumdl, &args),
            status,
        })
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn format_rumdl_command(rumdl: &Path, args: &[&str]) -> String {
    std::iter::once(rumdl.display().to_string())
        .chain(args.iter().map(|arg| (*arg).to_owned()))
        .collect::<Vec<_>>()
        .join(" ")
}
