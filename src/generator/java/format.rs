use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

const VERSION: &str = "1.24.0";

/// Prepare formatting for Java files under the output directory.
pub(crate) fn command(output: &Path, scratch: &Path) -> Result<Option<Command>> {
    let root = fs::canonicalize(output).map_err(|source| Error::ReadFile {
        path: output.to_owned(),
        source,
    })?;
    let mut files = Vec::new();
    collect_java_files(&root, &mut files)?;
    files.sort();
    if files.is_empty() {
        return Ok(None);
    }
    let jar = formatter_jar(output)?;
    // Use the Java launcher's argument-file parser, not google-java-format's:
    // the latter splits on whitespace without honoring quotes.
    let arguments = std::iter::once(Path::new("-jar"))
        .chain(std::iter::once(jar.as_path()))
        .chain(std::iter::once(Path::new("--replace")))
        .chain(files.iter().map(PathBuf::as_path))
        .map(quote_argument)
        .collect::<Vec<_>>()
        .join("\n");
    let argument_file = scratch.join("java-format.args");
    fs::write(&argument_file, arguments).map_err(|source| Error::WriteFile {
        path: argument_file.clone(),
        source,
    })?;
    let mut argument = std::ffi::OsString::from("@");
    argument.push(argument_file);
    let mut command = Command::new("java");
    command.arg(argument);
    Ok(Some(command))
}

fn collect_java_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let read_error = |source| Error::ReadFile {
        path: directory.to_owned(),
        source,
    };
    for entry in fs::read_dir(directory).map_err(read_error)? {
        let entry = entry.map_err(read_error)?;
        let file_type = entry.file_type().map_err(read_error)?;
        if file_type.is_dir() {
            collect_java_files(&entry.path(), files)?;
        } else if file_type.is_file() && entry.path().extension().is_some_and(|ext| ext == "java") {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn quote_argument(path: &Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('\u{c}', "\\f");
    format!("\"{escaped}\"")
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

#[cfg(test)]
mod tests {
    use super::{collect_java_files, quote_argument};
    use std::fs;
    use std::path::Path;

    #[test]
    fn collects_only_java_files_recursively() {
        let output = tempfile::tempdir().unwrap();
        fs::create_dir(output.path().join("nested")).unwrap();
        let java = output.path().join("nested/UserCode.java");
        fs::write(&java, "class UserCode {}").unwrap();
        fs::write(output.path().join("notes.txt"), "leave alone").unwrap();
        let mut files = Vec::new();
        collect_java_files(output.path(), &mut files).unwrap();
        assert_eq!(files, vec![java]);
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinks_outside_output_or_into_cycles() {
        use std::os::unix::fs::symlink;

        let output = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let java = outside.path().join("UserCode.java");
        fs::write(&java, "class UserCode {}").unwrap();
        symlink(outside.path(), output.path().join("outside")).unwrap();
        symlink(output.path(), output.path().join("cycle")).unwrap();
        symlink(java, output.path().join("Linked.java")).unwrap();
        let mut files = Vec::new();
        collect_java_files(output.path(), &mut files).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn quotes_java_launcher_arguments() {
        assert_eq!(
            quote_argument(Path::new("dir with spaces/Foo.java")),
            "\"dir with spaces/Foo.java\""
        );
        assert_eq!(
            quote_argument(Path::new("C:\\dir\\a\"b\n\t\r\u{c}.java")),
            "\"C:\\\\dir\\\\a\\\"b\\n\\t\\r\\f.java\""
        );
    }
}
