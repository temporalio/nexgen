use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use nexgen::error::Error as NexgenError;

use crate::error::{Error, Result};

#[derive(Clone, Copy)]
pub enum ValidationLanguage {
    Rust,
    Python,
    Typescript,
    Go,
    Java,
    Dotnet,
    // Confirms hand-authored Markdown formats have been auto formatted.
    Markdown,
    // Full validation additionally checks agreement between all JSON Schema
    // backends, after every language's dependencies have been provisioned.
    CrossLanguage,
}

pub struct ValidateRequest {
    pub language: Option<ValidationLanguage>,
    pub verbose: bool,
    pub log_dir: Option<PathBuf>,
}

pub fn validate(request: &ValidateRequest) -> Result<()> {
    let repo_root = repo_root();
    let log_dir = request
        .log_dir
        .clone()
        .unwrap_or_else(|| repo_root.join("target/validate-logs"));
    fs::create_dir_all(&log_dir).map_err(|source| NexgenError::WriteFile {
        path: log_dir.clone(),
        source,
    })?;
    for language in validation_languages(request.language) {
        let mut log = ValidationLog::new(&log_dir, language, request.verbose)?;
        println!(
            "==> Validating {} (log: {})",
            language.name(),
            log.path.display()
        );
        let started = Instant::now();
        let result = match language {
            ValidationLanguage::Rust => validate_rust(&repo_root, &mut log),
            ValidationLanguage::Python => validate_python(&repo_root, &mut log),
            ValidationLanguage::Typescript => validate_typescript(&repo_root, &mut log),
            ValidationLanguage::Go => validate_go(&repo_root, &mut log),
            ValidationLanguage::Java => validate_java(&repo_root, &mut log),
            ValidationLanguage::Dotnet => validate_dotnet(&repo_root, &mut log),
            ValidationLanguage::Markdown => validate_markdown(&repo_root, &mut log),
            ValidationLanguage::CrossLanguage => validate_conformance(&repo_root, &mut log),
        };
        let result = result.and_then(|()| {
            if !matches!(
                language,
                ValidationLanguage::Rust
                    | ValidationLanguage::Markdown
                    | ValidationLanguage::CrossLanguage
            ) {
                validate_generated_code(&repo_root, language, &mut log)?;
            }
            Ok(())
        });
        match result {
            Ok(()) => println!("    passed in {:.1?}", started.elapsed()),
            Err(error) => {
                println!(
                    "    FAILED in {:.1?}; inspect {}",
                    started.elapsed(),
                    log.path.display()
                );
                log.print_on_failure();
                return Err(error);
            }
        }
    }
    Ok(())
}

fn validation_languages(language: Option<ValidationLanguage>) -> Vec<ValidationLanguage> {
    match language {
        Some(language) => vec![language],
        None => vec![
            ValidationLanguage::Rust,
            ValidationLanguage::Python,
            ValidationLanguage::Typescript,
            ValidationLanguage::Go,
            ValidationLanguage::Java,
            ValidationLanguage::Dotnet,
            ValidationLanguage::Markdown,
            ValidationLanguage::CrossLanguage,
        ],
    }
}

impl ValidationLanguage {
    fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Markdown => "Markdown",
            Self::Python => "Python",
            Self::Typescript => "TypeScript",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::Dotnet => ".NET",
            Self::CrossLanguage => "cross-language JSON Schema conformance",
        }
    }

    fn log_name(self) -> &'static str {
        match self {
            Self::Typescript => "typescript",
            Self::Dotnet => "dotnet",
            Self::Rust => "rust",
            Self::Markdown => "markdown",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::CrossLanguage => "conformance",
        }
    }
}

struct ValidationLog {
    path: PathBuf,
    file: File,
    verbose: bool,
}

impl ValidationLog {
    fn new(log_dir: &Path, language: ValidationLanguage, verbose: bool) -> Result<Self> {
        let path = log_dir.join(format!("{}.log", language.log_name()));
        let mut file = File::create(&path).map_err(|source| NexgenError::WriteFile {
            path: path.clone(),
            source,
        })?;
        writeln!(file, "# nexgen validation: {}", language.name()).map_err(|source| {
            NexgenError::WriteFile {
                path: path.clone(),
                source,
            }
        })?;
        Ok(Self {
            path,
            file,
            verbose,
        })
    }

    fn command(&mut self, cwd: &Path, program: &str, args: &[&str]) -> Result<()> {
        let command = format_command(program, args);
        let heading = format!("\n==> (cd {} && {command})\n", cwd.display());
        self.write(heading.as_bytes())?;
        if self.verbose {
            print!("{heading}");
        }
        Ok(())
    }

    fn write(&mut self, output: &[u8]) -> Result<()> {
        self.file
            .write_all(output)
            .map_err(|source| NexgenError::WriteFile {
                path: self.path.clone(),
                source,
            })?;
        self.file.flush().map_err(|source| NexgenError::WriteFile {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }

    fn output(&mut self, stdout: &[u8], stderr: &[u8]) -> Result<()> {
        self.write(stdout)?;
        self.write(stderr)?;
        if self.verbose {
            io::stdout().write_all(stdout).ok();
            io::stderr().write_all(stderr).ok();
        }
        Ok(())
    }

    fn print_on_failure(&mut self) {
        if let Err(error) = self.file.flush() {
            eprintln!(
                "failed to flush validation log {}: {error}",
                self.path.display()
            );
            return;
        }
        match fs::read_to_string(&self.path) {
            Ok(contents) => eprintln!("\n==> Validation log ({})\n{contents}", self.path.display()),
            Err(error) => eprintln!(
                "failed to read validation log {}: {error}",
                self.path.display()
            ),
        }
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn sample_roots(repo_root: &Path, language: &str) -> [PathBuf; 2] {
    [
        repo_root.join("samples").join(language),
        repo_root.join("advanced/samples").join(language),
    ]
}

fn validate_rust(repo_root: &Path, log: &mut ValidationLog) -> Result<()> {
    run(log, repo_root, "cargo", &["fmt", "--all", "--check"])?;
    // Default members are nexgen and xtask. The toolchain integration-test
    // package is selected only by non-Rust validation stages.
    run(log, repo_root, "cargo", &["test", "--all-features"])
}

fn validate_generated_code(
    repo_root: &Path,
    language: ValidationLanguage,
    log: &mut ValidationLog,
) -> Result<()> {
    run(log, repo_root, "cargo", &integration_test_args(language))
}

fn validate_conformance(repo_root: &Path, log: &mut ValidationLog) -> Result<()> {
    // Other backends resolve dependencies on demand. TypeScript links the
    // sample project's installed dependencies into its scratch workspaces.
    let typescript = repo_root.join("samples/typescript");
    if !typescript.join("node_modules").is_dir() {
        run(log, &typescript, "npm", &["ci"])?;
    }
    validate_generated_code(repo_root, ValidationLanguage::CrossLanguage, log)
}

fn integration_test_args(language: ValidationLanguage) -> [&'static str; 6] {
    [
        "test",
        "--package",
        "nexgen-validation-tests",
        "--all-features",
        "--test",
        language.log_name(),
    ]
}

fn validate_markdown(repo_root: &Path, log: &mut ValidationLog) -> Result<()> {
    match run(log, repo_root, "rumdl", &["check", "."]) {
        // rumdl exits with 1 when files need formatting; other failures are
        // tool or configuration errors and are reported as-is.
        Err(Error::CommandFailed { status, .. }) if status.code() == Some(1) => {
            Err(Error::MarkdownUnformatted)
        }
        result => result,
    }
}

fn validate_python(repo_root: &Path, log: &mut ValidationLog) -> Result<()> {
    validate_generated_examples(repo_root, "python", log)?;
    for root in sample_roots(repo_root, "python") {
        run(log, &root, "uv", &["sync", "--locked"])?;
        run(log, &root, "uv", &["run", "ruff", "check", "."])?;
        run(log, &root, "uv", &["run", "ruff", "format", "--check", "."])?;
        run(log, &root, "uv", &["run", "basedpyright", "--warnings"])?;
        run(log, &root, "uv", &["run", "pytest"])?;
        run(
            log,
            &root,
            "uv",
            &[
                "run",
                "--isolated",
                "--python",
                "3.10",
                "--locked",
                "python",
                "-m",
                "pytest",
            ],
        )?;
    }
    Ok(())
}

fn validate_typescript(repo_root: &Path, log: &mut ValidationLog) -> Result<()> {
    validate_generated_examples(repo_root, "typescript", log)?;
    for root in sample_roots(repo_root, "typescript") {
        run(log, &root, "npm", &["ci"])?;
        run(
            log,
            &root,
            "npm",
            &["exec", "--", "prettier", "--check", "."],
        )?;
        run(log, &root, "npm", &["run", "typecheck"])?;
        run(log, &root, "npm", &["run", "test"])?;
    }
    Ok(())
}

fn validate_go(repo_root: &Path, log: &mut ValidationLog) -> Result<()> {
    validate_generated_examples(repo_root, "go", log)?;
    for root in sample_roots(repo_root, "go") {
        let output = run_output(log, &root, "gofmt", &["-l", "."])?;
        if !output.is_empty() {
            return Err(Error::RunCommand {
                cwd: root,
                command: format!("gofmt required for:\n{output}"),
                source: io::Error::other("gofmt found unformatted files"),
            });
        }
        run(log, &root, "go", &["test", "./..."])?;
    }
    Ok(())
}

fn validate_java(repo_root: &Path, log: &mut ValidationLog) -> Result<()> {
    validate_generated_examples(repo_root, "java", log)?;
    for root in sample_roots(repo_root, "java") {
        run(log, &root, "./gradlew", &["build", "--no-daemon"])?;
    }
    Ok(())
}

fn validate_dotnet(repo_root: &Path, log: &mut ValidationLog) -> Result<()> {
    validate_generated_examples(repo_root, "dotnet", log)?;
    for root in sample_roots(repo_root, "dotnet") {
        run(log, &root, "dotnet", &["test", "tests/", "--nologo"])?;
    }
    let workflow_service_docs_root = repo_root.join("advanced/samples/dotnet");
    run(
        log,
        &workflow_service_docs_root,
        "dotnet",
        &[
            "build",
            "Nexgen.DotNetWorkflowServiceDocs.csproj",
            "--nologo",
        ],
    )?;
    run(
        log,
        &workflow_service_docs_root,
        "dotnet",
        &[
            "build",
            "Nexgen.DotNetMultiOperationService.csproj",
            "--nologo",
        ],
    )?;
    Ok(())
}

fn validate_generated_examples(
    repo_root: &Path,
    language: &str,
    log: &mut ValidationLog,
) -> Result<()> {
    let language = match language {
        "python" | "typescript" | "go" | "java" | "dotnet" => language,
        _ => unreachable!("validation language must support generated examples"),
    };
    run(
        log,
        repo_root,
        "cargo",
        &["build-examples", "--lang", language],
    )?;
    run(
        log,
        repo_root,
        "git",
        &["diff", "--exit-code", "--", "samples", "advanced/samples"],
    )
}

fn run(log: &mut ValidationLog, cwd: &Path, program: &str, args: &[&str]) -> Result<()> {
    log.command(cwd, program, args)?;
    let mut command = Command::new(program);
    command.current_dir(cwd).args(args);
    let output = command.output().map_err(|source| Error::RunCommand {
        cwd: cwd.to_path_buf(),
        command: format_command(program, args),
        source,
    })?;
    log.output(&output.stdout, &output.stderr)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::CommandFailed {
            cwd: cwd.to_path_buf(),
            command: format_command(program, args),
            status: output.status,
        })
    }
}

fn run_output(log: &mut ValidationLog, cwd: &Path, program: &str, args: &[&str]) -> Result<String> {
    log.command(cwd, program, args)?;
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|source| Error::RunCommand {
            cwd: cwd.to_path_buf(),
            command: format_command(program, args),
            source,
        })?;
    log.output(&output.stdout, &output.stderr)?;
    if !output.status.success() {
        return Err(Error::CommandFailed {
            cwd: cwd.to_path_buf(),
            command: format_command(program, args),
            status: output.status,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn format_command(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{ValidationLanguage, integration_test_args, validation_languages};

    #[test]
    fn each_toolchain_stage_selects_one_integration_target() {
        for (language, target) in [
            (ValidationLanguage::Python, "python"),
            (ValidationLanguage::Typescript, "typescript"),
            (ValidationLanguage::Go, "go"),
            (ValidationLanguage::Java, "java"),
            (ValidationLanguage::Dotnet, "dotnet"),
            (ValidationLanguage::CrossLanguage, "conformance"),
        ] {
            assert_eq!(
                integration_test_args(language),
                [
                    "test",
                    "--package",
                    "nexgen-validation-tests",
                    "--all-features",
                    "--test",
                    target,
                ]
            );
        }
    }

    #[test]
    fn full_validation_ends_with_one_cross_language_stage() {
        let languages: Vec<_> = validation_languages(None)
            .into_iter()
            .map(|language| language.log_name())
            .collect();
        assert_eq!(
            languages,
            [
                "rust",
                "python",
                "typescript",
                "go",
                "java",
                "dotnet",
                "markdown",
                "conformance"
            ]
        );
    }

    #[test]
    fn a_selected_stage_runs_on_its_own() {
        for language in [
            ValidationLanguage::Rust,
            ValidationLanguage::Python,
            ValidationLanguage::Typescript,
            ValidationLanguage::Go,
            ValidationLanguage::Java,
            ValidationLanguage::Dotnet,
            ValidationLanguage::Markdown,
            ValidationLanguage::CrossLanguage,
        ] {
            let stages = validation_languages(Some(language));
            assert_eq!(stages.len(), 1);
            assert_eq!(stages[0].log_name(), language.log_name());
        }
    }
}
