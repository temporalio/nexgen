use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// Pinned rumdl release used to format human-authored Markdown. crates.io
/// versions are immutable, and `cargo install --locked` builds with the
/// dependency versions recorded in that release's `Cargo.lock`. Update this
/// deliberately, then rerun `cargo fmt-markdown` and review the diff.
const RUMDL_VERSION: &str = "0.2.79";

/// Rewraps the repository's Markdown files in place according to `.rumdl.toml`.
pub fn fmt_markdown() -> Result<()> {
    let repo_root = repo_root();
    let rumdl = rumdl_path(&repo_root)?;
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

/// Returns the pinned rumdl binary, installing it under `target/tools` on
/// first use so contributors and CI never depend on whatever is on `PATH`.
pub fn rumdl_path(repo_root: &Path) -> Result<PathBuf> {
    let install_root = repo_root
        .join("target/tools")
        .join(format!("rumdl-{RUMDL_VERSION}"));
    let binary = install_root
        .join("bin")
        .join(format!("rumdl{}", std::env::consts::EXE_SUFFIX));
    if binary.is_file() {
        return Ok(binary);
    }

    println!(
        "==> Installing rumdl {RUMDL_VERSION} into {}",
        install_root.display()
    );
    let version = format!("={RUMDL_VERSION}");
    let install_root_arg = install_root.to_string_lossy();
    let args = [
        "install",
        "rumdl",
        "--locked",
        "--version",
        version.as_str(),
        "--root",
        install_root_arg.as_ref(),
    ];
    let status = Command::new("cargo")
        .current_dir(repo_root)
        .args(args)
        .status()
        .map_err(|source| Error::RunCommand {
            cwd: repo_root.to_path_buf(),
            command: format!("cargo {}", args.join(" ")),
            source,
        })?;
    if !status.success() {
        return Err(Error::CommandFailed {
            cwd: repo_root.to_path_buf(),
            command: format!("cargo {}", args.join(" ")),
            status,
        });
    }
    Ok(binary)
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
