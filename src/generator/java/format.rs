use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// Prepare formatting for Java files under the output directory.
pub(crate) fn command(output: &Path) -> Result<Option<Command>> {
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
    let mut command = Command::new("google-java-format");
    command.arg("--replace").args(files);
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

#[cfg(test)]
mod tests {
    use super::collect_java_files;
    use std::fs;

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
}
