// The `--format`/`--descriptors` CLI surface lives behind the `advanced`
// feature; the option-rejection checks are feature-agnostic.
#[cfg(all(unix, feature = "advanced"))]
mod format_tests {
    use std::env;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn project_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn sample_input_path(root: &Path) -> PathBuf {
        root.join("advanced/samples/inputs/workflow-service.wit")
    }

    fn descriptor_path(root: &Path) -> PathBuf {
        root.join("advanced/samples/descriptors/temporal_api.bin")
    }

    fn linked_inputs_path(root: &Path) -> PathBuf {
        root.join("advanced/samples/inputs/deps")
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("nexgen-{name}-{unique}"))
    }

    fn write_formatter_script(dir: &Path, name: &str, marker: &str) -> PathBuf {
        let script_path = dir.join(name);
        let script = format!(
            "#!/bin/sh\nfor arg do target=\"$arg\"; done\nif [ -d \"$target\" ]; then\n  if [ -f \"$target/__init__.py\" ]; then\n    target=\"$target/__init__.py\"\n  else\n    target=\"$target/index.ts\"\n  fi\nfi\nprintf '\\n{marker}\\n' >> \"$target\"\n"
        );
        fs::write(&script_path, script).unwrap();
        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
        script_path
    }

    fn formatter_path_env(dir: &Path) -> String {
        let existing_path = env::var("PATH").unwrap_or_default();
        format!("{}:{existing_path}", dir.display())
    }

    #[test]
    fn cli_generates_and_formats_python_file() {
        let root = project_root();
        let temp_dir = unique_temp_dir("python-format");
        fs::create_dir_all(&temp_dir).unwrap();
        let output_path = temp_dir.join("output");
        write_formatter_script(&temp_dir, "ruff", "# formatted by test");

        let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
            .env("PATH", formatter_path_env(&temp_dir))
            .args([
                "python",
                sample_input_path(&root).to_str().unwrap(),
                linked_inputs_path(&root).to_str().unwrap(),
                "--descriptors",
                descriptor_path(&root).to_str().unwrap(),
                "--output",
                output_path.to_str().unwrap(),
                "--support-package",
                "temporal_support",
                "--format",
            ])
            .status()
            .unwrap();

        assert!(status.success());

        let rendered = fs::read_to_string(output_path.join("__init__.py")).unwrap();
        assert!(rendered.contains("# formatted by test"));

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn cli_generates_and_formats_typescript_file() {
        let root = project_root();
        let temp_dir = unique_temp_dir("typescript-format");
        fs::create_dir_all(&temp_dir).unwrap();
        let output_path = temp_dir.join("output");
        write_formatter_script(&temp_dir, "prettier", "// formatted by test");

        let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
            .env("PATH", formatter_path_env(&temp_dir))
            .args([
                "ts",
                sample_input_path(&root).to_str().unwrap(),
                linked_inputs_path(&root).to_str().unwrap(),
                "--descriptors",
                descriptor_path(&root).to_str().unwrap(),
                "--output",
                output_path.to_str().unwrap(),
                "--support-package",
                "./support",
                "--format",
            ])
            .status()
            .unwrap();

        assert!(status.success());

        let rendered = fs::read_to_string(output_path.join("index.ts")).unwrap();
        assert!(rendered.contains("// formatted by test"));

        let _ = fs::remove_dir_all(temp_dir);
    }
}

#[cfg(unix)]
mod reject_tests {
    use std::process::Command;

    #[test]
    fn cli_rejects_legacy_and_target_specific_options() {
        let binary = env!("CARGO_BIN_EXE_nexgen");

        assert!(
            !Command::new(binary)
                .args(["--lang", "python"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            !Command::new(binary)
                .args(["python", "--date-time-types", "date"])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            !Command::new(binary)
                .args(["python", "--no-native-api"])
                .status()
                .unwrap()
                .success()
        );
    }
}
