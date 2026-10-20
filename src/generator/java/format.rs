use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};
use crate::generator::GeneratedFiles;

const VERSION: &str = "1.24.0";

/// Prepare formatting for only this invocation's emitted Java files.
pub(crate) fn commands(output: &Path, generated: &GeneratedFiles) -> Result<Vec<Command>> {
    let root = fs::canonicalize(output).map_err(|source| Error::ReadFile {
        path: output.to_owned(),
        source,
    })?;
    let files = generated
        .files
        .keys()
        .filter(|path| path.extension().is_some_and(|ext| ext == "java"))
        .map(|path| root.join(path))
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let jar = formatter_jar(output)?;
    // Bound argument sizes on platforms with small command-line limits.
    Ok(files
        .chunks(32)
        .map(|batch| {
            let mut command = Command::new("java");
            command.arg("-jar").arg(&jar).arg("--replace").args(batch);
            command
        })
        .collect())
}

fn formatter_jar(output: &Path) -> Result<PathBuf> {
    if let Some(jar) = env::var_os("NEXGEN_JAVA_FORMAT_JAR") {
        return Ok(PathBuf::from(jar));
    }
    let cache = env::var_os("XDG_CACHE_HOME")
        .or_else(|| env::var_os("LOCALAPPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".cache"))
                .unwrap_or_else(env::temp_dir)
        })
        .join("nexgen");
    let name = format!("google-java-format-{VERSION}-all-deps.jar");
    let jar = cache.join(&name);
    if jar.is_file() {
        return Ok(jar);
    }
    fs::create_dir_all(&cache).map_err(|source| Error::WriteFile {
        path: cache.clone(),
        source,
    })?;
    let download = tempfile::NamedTempFile::new_in(&cache).map_err(|source| Error::WriteFile {
        path: cache,
        source,
    })?;
    let url = format!(
        "https://repo.maven.apache.org/maven2/com/google/googlejavaformat/google-java-format/{VERSION}/{name}"
    );
    eprintln!(
        "Downloading google-java-format {VERSION} to {}",
        jar.display()
    );
    crate::run_formatter(
        Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--connect-timeout",
                "15",
                "--max-time",
                "120",
                "--output",
            ])
            .arg(download.path())
            .arg(url),
        output,
    )?;
    download.persist(&jar).map_err(|error| Error::WriteFile {
        path: jar.clone(),
        source: error.error,
    })?;
    Ok(jar)
}
