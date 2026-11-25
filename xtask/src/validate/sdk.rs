//! Execute SDK-native workflow-service tests against the pinned SDK submodules.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{ValidationLog, format_command};
use crate::error::{Error, Result};

const LOCAL_SERVER_OVERRIDES: &[&str] = &[
    "DISABLE_SYSTEM_NEXUS_TESTS",
    "TEMPORAL_IS_CLOUD_TESTS",
    "TEMPORAL_TEST_ENV_CONFIG_SERVER",
    "TEMPORAL_TEST_CLIENT_TARGET_HOST",
    "TEMPORAL_TEST_CLIENT_NAMESPACE",
    "TEMPORAL_NAMESPACE",
    "SERVICE_ADDR",
    "SERVICE_HTTP_ADDR",
];

pub(super) fn validate(root: &Path, language: &str, log: &mut ValidationLog) -> Result<()> {
    if !["go", "python", "typescript", "dotnet"].contains(&language) {
        return Err(invalid(format!("unsupported SDK language {language:?}")));
    }
    let root = io_at(root, root.canonicalize())?;
    let inputs = root.join("advanced/sdk");
    let base = root.join("target/sdk-validation");
    io_at(&base, fs::create_dir_all(&base))?;
    let binary_dir = base.join("nexgen");
    let mut runner = Runner {
        log,
        env: BTreeMap::new(),
    };
    runner.run(
        &root,
        "cargo",
        &[
            "build",
            "--locked",
            "--features",
            "advanced",
            "--bin",
            "nexgen",
            "--target-dir",
            &binary_dir.to_string_lossy(),
        ],
    )?;
    let binary = binary_dir
        .join("debug")
        .join(format!("nexgen{}", std::env::consts::EXE_SUFFIX));
    runner
        .env
        .insert("NEXGEN_BIN".into(), binary.clone().into());
    runner
        .env
        .insert("NEX_GEN_BIN".into(), binary.clone().into());
    let mut paths = vec![binary.parent().unwrap().to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    runner.env.insert(
        "PATH".into(),
        std::env::join_paths(paths)
            .map_err(|error| invalid(format!("constructing PATH: {error}")))?,
    );

    let relative_module = Path::new("advanced/sdk/repos").join(language);
    let module = root.join(&relative_module);
    if !module.join(".git").exists() {
        runner.run(
            &root,
            "git",
            &[
                "submodule",
                "update",
                "--init",
                "--",
                &relative_module.to_string_lossy(),
            ],
        )?;
    }
    let revision = runner.output(&module, "git", &["rev-parse", "HEAD"])?;
    runner.note(&format!(
        "Validating sdk-{language} submodule at {revision}"
    ))?;
    // Retain diagnostics without dirtying or resetting the dependency checkout.
    let checkout = io_at(
        &base,
        tempfile::Builder::new()
            .prefix(&format!("{language}-"))
            .tempdir_in(&base),
    )?
    .keep();
    runner.run(
        &module,
        "git",
        &[
            "worktree",
            "add",
            "--detach",
            &checkout.to_string_lossy(),
            &revision,
        ],
    )?;
    runner.run(
        &checkout,
        "git",
        &[
            "submodule",
            "update",
            "--init",
            "--recursive",
            "--depth",
            "1",
        ],
    )?;
    let support = inputs.join(language);
    copy_tree(&support, &checkout)?;
    let reports = checkout.join(".nexgen-test-results");
    io_at(&reports, fs::create_dir(&reports))?;
    if language == "go" {
        return validate_go(&root, &checkout, &reports, &mut runner);
    }
    let core = checkout.join(match language {
        "python" => "temporalio/bridge/sdk-core",
        "typescript" => "packages/core-bridge/sdk-core",
        "dotnet" => "src/Temporalio/Bridge/sdk-core",
        _ => unreachable!(),
    });
    isolate_cargo(&core.parent().unwrap().join("Cargo.toml"))?;
    stage_wit(
        &root,
        &core.join("crates/protos/protos/api_upstream/nexus"),
        "workflow-service.wit",
    )?;
    match language {
        "python" => validate_python(&checkout, &reports, &mut runner),
        "typescript" => validate_typescript(&checkout, &reports, &mut runner),
        "dotnet" => validate_dotnet(&checkout, &reports, &mut runner),
        _ => unreachable!(),
    }
}

fn validate_go(
    root: &Path,
    checkout: &Path,
    reports: &Path,
    runner: &mut Runner<'_>,
) -> Result<()> {
    runner.run(checkout, "go", &["mod", "download"])?;
    let api = PathBuf::from(runner.output(
        checkout,
        "go",
        &["list", "-m", "-f", "{{.Dir}}", "go.temporal.io/api"],
    )?);
    // Never change the shared, read-only Go module cache.
    let local_api = checkout.join(".nexgen-api");
    copy_tree(&api, &local_api)?;
    make_writable(&local_api)?;
    stage_wit(
        root,
        &local_api.join("systemnexus"),
        "workflowservice/v1/workflow-service.wit",
    )?;
    runner.run(
        checkout,
        "go",
        &[
            "mod",
            "edit",
            &format!("-replace=go.temporal.io/api={}", local_api.display()),
        ],
    )?;
    runner.run(checkout, "go", &["run", "./internal/cmd/system-nexus-gen"])?;
    runner.run(checkout, "go", &["test", "-run=^$", "./..."])?;
    runner.run(
        &checkout.join("internal/cmd/build"),
        "go",
        &[
            "run",
            ".",
            "integration-test",
            "-dev-server",
            "-module",
            "test",
            "-run",
            "^TestSystemNexusSuite$",
            "-packages",
            ".",
            "-log-dir",
            &reports.to_string_lossy(),
            "-console-output",
            "full",
        ],
    )
}

fn validate_python(checkout: &Path, reports: &Path, runner: &mut Runner<'_>) -> Result<()> {
    migrate_native_flag(
        &checkout.join("scripts/gen_nexus_system_api.py"),
        "args.append(\"--native-api\")",
        "args.extend([\"--targets\", \"workflow\"])",
    )?;
    runner.run(checkout, "uv", &["sync", "--locked", "--all-extras"])?;
    runner.run(
        checkout,
        "uv",
        &[
            "run",
            "--no-sync",
            "python",
            "scripts/gen_nexus_system_api.py",
        ],
    )?;
    runner.run(
        checkout,
        "uv",
        &["run", "--no-sync", "maturin", "develop", "--uv"],
    )?;
    // Catch broken generated exports before workflow tests retry until timeout.
    runner.run(
        checkout,
        "uv",
        &[
            "run",
            "--no-sync",
            "basedpyright",
            "--project",
            "basedpyrightconfig.json",
            "temporalio/nexus/system",
            "temporalio/worker/_workflow_instance.py",
            "tests/nexus/test_temporal_system_nexus.py",
            "tests/nexus/test_streams.py",
            "tests/nexus/test_streams_with_worker.py",
            "tests/worker/test_workflow_streams.py",
        ],
    )?;
    let report = reports.join("python.xml");
    runner.run(
        checkout,
        "uv",
        &[
            "run",
            "--no-sync",
            "pytest",
            "tests/nexus/test_temporal_system_nexus.py",
            "tests/nexus/test_streams.py",
            "tests/nexus/test_streams_with_worker.py",
            "tests/worker/test_workflow_streams.py",
            "tests/testing/test_stream_store.py",
            "tests/contrib/streams",
            "--workflow-environment",
            "local",
            &format!("--junitxml={}", report.display()),
        ],
    )
}

fn validate_typescript(checkout: &Path, reports: &Path, runner: &mut Runner<'_>) -> Result<()> {
    migrate_native_flag(
        &checkout.join("packages/workflow/scripts/gen-system-nexus.ts"),
        "'--native-api',",
        "'--targets', 'workflow',",
    )?;
    runner.run(checkout, "pnpm", &["install", "--frozen-lockfile"])?;
    runner.run(
        checkout,
        "pnpm",
        &["--filter", "@temporalio/workflow", "gen:system-nexus"],
    )?;
    runner.run(checkout, "pnpm", &["run", "build"])?;
    runner
        .env
        .insert("TEST_RESULTS_DIR".into(), reports.as_os_str().to_owned());
    runner.run(
        &checkout.join("packages/test"),
        "pnpm",
        &[
            "exec",
            "tsx",
            "../../scripts/ava-ci.ts",
            "./lib/test-integration-system-nexus.cloud-unavailable.js",
        ],
    )
}

fn validate_dotnet(checkout: &Path, reports: &Path, runner: &mut Runner<'_>) -> Result<()> {
    migrate_native_flag(
        &checkout.join("src/Temporalio.SystemNexus.Generator/Program.cs"),
        "arguments.Add(\"--native-api\");",
        "arguments.Add(\"--targets\");\n        arguments.Add(\"workflow\");",
    )?;
    runner.run(
        checkout,
        "dotnet",
        &["run", "--project", "src/Temporalio.SystemNexus.Generator"],
    )?;
    runner.run(
        checkout,
        "dotnet",
        &[
            "test",
            "tests/Temporalio.Tests",
            "--nologo",
            "--filter",
            concat!(
                "FullyQualifiedName~Temporalio.Tests.Nexus.NotificationTests|",
                "FullyQualifiedName~Temporalio.Tests.Nexus.CompletionCallbackTests|",
                "FullyQualifiedName~Temporalio.Tests.Nexus.NexusHandlerCallbackTests|",
                "FullyQualifiedName~Temporalio.Tests.Worker.SystemNexusTests|",
                "FullyQualifiedName~Temporalio.Tests.Worker.ExternalStorageTargetTests.SystemNexus_ConcurrentDestinations_SelectTheirOwnStorageDriversAndReplay",
            ),
            "--logger",
            "trx;LogFileName=system-nexus.trx",
            "--results-directory",
            &reports.to_string_lossy(),
        ],
    )
}

struct Runner<'a> {
    log: &'a mut ValidationLog,
    env: BTreeMap<OsString, OsString>,
}

impl Runner<'_> {
    fn run(&mut self, cwd: &Path, program: &str, args: &[&str]) -> Result<()> {
        self.output(cwd, program, args).map(|_| ())
    }

    fn output(&mut self, cwd: &Path, program: &str, args: &[&str]) -> Result<String> {
        self.log.command(cwd, program, args)?;
        let mut command = Command::new(program);
        command.current_dir(cwd).args(args).envs(&self.env);
        // SDK tests must use their local servers, not a developer's namespace.
        for key in LOCAL_SERVER_OVERRIDES {
            command.env_remove(key);
        }
        let output = command.output().map_err(|source| Error::RunCommand {
            cwd: cwd.to_owned(),
            command: format_command(program, args),
            source,
        })?;
        self.log.output(&output.stdout, &output.stderr)?;
        if !output.status.success() {
            return Err(Error::CommandFailed {
                cwd: cwd.to_owned(),
                command: format_command(program, args),
                status: output.status,
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn note(&mut self, message: &str) -> Result<()> {
        self.log
            .output(format!("\n==> {message}\n").as_bytes(), &[])
    }
}

fn stage_wit(root: &Path, destination: &Path, workflow_path: &str) -> Result<()> {
    let shared = root.join("advanced/samples/inputs");
    io_at(destination, fs::create_dir_all(destination))?;
    let workflow = destination.join(workflow_path);
    let parent = workflow.parent().unwrap();
    io_at(parent, fs::create_dir_all(parent))?;
    copy_file(&root.join("advanced/sdk/workflow-service.wit"), &workflow)?;
    // Native SDK generation emits notification models in the same invocation.
    copy_file(
        &shared.join("notification-service.wit"),
        &destination.join("notification-service.wit"),
    )?;
    for entry in io_at(&shared, fs::read_dir(shared.join("deps")))? {
        let entry = io_at(&shared, entry)?;
        let target = destination.join("deps").join(entry.file_name());
        if target.exists() {
            io_at(&target, fs::remove_dir_all(&target))?;
        }
        copy_tree(&entry.path(), &target)?;
    }
    Ok(())
}

fn isolate_cargo(manifest: &Path) -> Result<()> {
    let text = read(manifest)?;
    let config: toml::Table = toml::from_str(&text)
        .map_err(|error| invalid(format!("{}: {error}", manifest.display())))?;
    if !config.contains_key("workspace") {
        write(
            manifest,
            &(text + "\n[workspace]\nexclude = [\"sdk-core\"]\n"),
        )?;
    }
    Ok(())
}

fn migrate_native_flag(path: &Path, old: &str, new: &str) -> Result<()> {
    let text = read(path)?;
    match text.matches(old).count() {
        0 => Ok(()),
        1 => write(path, &text.replacen(old, new, 1)),
        _ => Err(invalid(format!(
            "ambiguous legacy CLI invocation in {}",
            path.display()
        ))),
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    io_at(destination, fs::create_dir_all(destination))?;
    for entry in io_at(source, fs::read_dir(source))? {
        let entry = io_at(source, entry)?;
        let target = destination.join(entry.file_name());
        if io_at(&entry.path(), entry.file_type())?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn make_writable(path: &Path) -> Result<()> {
    let metadata = io_at(path, fs::metadata(path))?;
    let mut permissions = metadata.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    io_at(path, fs::set_permissions(path, permissions))?;
    if metadata.is_dir() {
        for entry in io_at(path, fs::read_dir(path))? {
            make_writable(&io_at(path, entry)?.path())?;
        }
    }
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    io_at(destination, fs::copy(source, destination)).map(|_| ())
}

fn read(path: &Path) -> Result<String> {
    io_at(path, fs::read_to_string(path))
}

fn write(path: &Path, text: &str) -> Result<()> {
    io_at(path, fs::write(path, text))
}

fn io_at<T>(path: &Path, result: io::Result<T>) -> Result<T> {
    result.map_err(|error| invalid(format!("{}: {error}", path.display())))
}

fn invalid(message: impl Into<String>) -> Error {
    Error::SdkValidation(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_authored_wit_and_support_verbatim() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let temp = tempfile::tempdir().unwrap();
        let stale = temp.path().join("deps/nexus-temporal-types/obsolete.wit");
        fs::create_dir_all(stale.parent().unwrap()).unwrap();
        fs::write(&stale, "obsolete").unwrap();
        let other = temp.path().join("deps/other/model.wit");
        fs::create_dir_all(other.parent().unwrap()).unwrap();
        fs::write(&other, "upstream").unwrap();
        stage_wit(&root, temp.path(), "workflow-service.wit").unwrap();
        assert!(!stale.exists());
        assert_eq!(read(&other).unwrap(), "upstream");
        assert_eq!(
            read(&temp.path().join("workflow-service.wit")).unwrap(),
            read(&root.join("advanced/sdk/workflow-service.wit")).unwrap(),
        );
        let go = temp.path().join("go-wit");
        stage_wit(&root, &go, "workflowservice/v1/workflow-service.wit").unwrap();
        assert_eq!(
            read(&go.join("workflowservice/v1/workflow-service.wit")).unwrap(),
            read(&root.join("advanced/sdk/workflow-service.wit")).unwrap(),
        );
        assert_eq!(
            read(&temp.path().join("deps/nexus-temporal-types/model.wit")).unwrap(),
            read(&root.join("advanced/samples/inputs/deps/nexus-temporal-types/model.wit"))
                .unwrap(),
        );
        for (language, files) in [
            ("go", &["internal/workflowservice_support.go"][..]),
            ("python", &["temporalio/nexus/system/_support.py"][..]),
            (
                "typescript",
                &["packages/workflow/src/nexus/system/support.ts"][..],
            ),
            (
                "dotnet",
                &[
                    "src/Temporalio/Nexus/SystemNexus/ProtoExtensions.cs",
                    "src/Temporalio/Nexus/SystemNexus/SystemNexusConverterContext.cs",
                    "src/Temporalio/Nexus/SystemNexus/TemporalFunctionNames.cs",
                    "src/Temporalio/Nexus/SystemNexus/TemporalWorkflowContext.cs",
                    "src/Temporalio/Nexus/SystemNexus/WorkflowServiceSerializationContexts.cs",
                    "src/Temporalio/Nexus/SystemNexus/WorkflowServiceStorageTargets.cs",
                ][..],
            ),
        ] {
            let source = root.join("advanced/sdk").join(language);
            let checkout = temp.path().join(language);
            for file in files {
                let destination = checkout.join(file);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::write(destination, "SDK baseline support").unwrap();
            }
            copy_tree(&source, &checkout).unwrap();
            for file in files {
                assert_eq!(
                    read(&checkout.join(file)).unwrap(),
                    read(&source.join(file)).unwrap(),
                    "{language}: {file}",
                );
            }
        }
    }

    #[test]
    fn cargo_boundary_is_idempotent_and_preserves_existing_workspaces() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("Cargo.toml");
        for text in [
            "[package]\nname = \"bridge\"\nversion = \"0.1.0\"\n",
            "[workspace]\nmembers = [\"bridge\"]\n",
        ] {
            write(&manifest, text).unwrap();
            isolate_cargo(&manifest).unwrap();
            let isolated = read(&manifest).unwrap();
            assert!(isolated.starts_with(text));
            isolate_cargo(&manifest).unwrap();
            assert_eq!(read(&manifest).unwrap(), isolated);
        }
    }

    #[test]
    fn legacy_cli_translation_is_checked_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("generator");
        write(&path, "old old").unwrap();
        assert!(migrate_native_flag(&path, "old", "new").is_err());
        write(&path, "old").unwrap();
        migrate_native_flag(&path, "old", "new").unwrap();
        migrate_native_flag(&path, "old", "new").unwrap();
        assert_eq!(read(&path).unwrap(), "new");
    }
}
