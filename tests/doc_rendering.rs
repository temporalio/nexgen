use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::{GenerateRequest, generate_to_file};

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

const HOSTILE_DOCUMENTATION_SCHEMA: &str = r##"nexusrpc: "1.0.0"
$schema: https://json-schema.org/draft/2020-12/schema
description: Hostile documentation compilation fixture.
services:
  HostileService:
    fqn: example.docs.v1.HostileService
    description: |-
      Service prose containing */ and <service> & enough words to cross the target formatting boundary without losing its authored structure.

      Service second paragraph.

      +build ignore

      +builder is ordinary prose

      go:generate hostile-tool
    operations:
      run:
        description: |-
          Operation prose containing */ and <operation> & enough words to cross the target formatting boundary without losing its authored structure.

          Operation second paragraph.
        input: { $ref: "#/$defs/Hostile" }
        output: { $ref: "#/$defs/Hostile" }
$defs:
  Hostile:
    title: Hostile title */ & <summary>
    description: |-
      Type prose containing */ and <type> & enough words to cross the target formatting boundary without losing its authored structure.

      Type second paragraph has \ and C:\unicorn plus triple quotes """ safely.
    deprecated: true
    type: object
    additionalProperties: false
    properties:
      value:
        title: Value title */ & <summary>
        description: |-
          Field prose containing */ and <field> & enough words to cross the target formatting boundary without losing its authored structure.

          Field second paragraph contains a backslash \ and triple quotes """ safely.
        deprecated: true
        type: string
    required: [value]
"##;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn unique_temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"))
}

fn generate(language: nexgen::language::Language, java_package_name: Option<&str>) -> PathBuf {
    let temp_dir = unique_temp_dir(language.as_str());
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("hostile.nexusrpc.yaml");
    let output_dir_name = java_package_name
        .and_then(|package| package.rsplit('.').next())
        .unwrap_or("output");
    let output_path = temp_dir.join(output_dir_name);
    fs::write(&input_path, HOSTILE_DOCUMENTATION_SCHEMA).unwrap();
    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: java_package_name.map(str::to_string),
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    output_path
}

/// Installs the sample's npm dependencies when the checkout has not run
/// `npm ci` yet; CI validates Rust before it touches the TypeScript samples.
fn ensure_typescript_dependencies(sample_dir: &Path) {
    if sample_dir.join("node_modules").exists() {
        return;
    }

    let install_status = Command::new("npm")
        .current_dir(sample_dir)
        .args(["install", "--no-fund", "--no-audit"])
        .status()
        .unwrap();
    assert!(
        install_status.success(),
        "npm install failed in {}",
        sample_dir.display()
    );
}

fn read_files(root: &Path, extension: &str) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, extension: &str, files: &mut BTreeMap<PathBuf, String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, extension, files);
            } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read_to_string(path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, extension, &mut files);
    files
}

fn joined_files(root: &Path, extension: &str) -> String {
    read_files(root, extension)
        .into_values()
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_hostile_comment_lines_fit(rendered: &str) {
    let fixture_fragments = [
        "Hostile title",
        "Value title",
        "Service prose",
        "Operation prose",
        "Type prose",
        "Field prose",
        "formatting boundary",
        "boundary without",
        "second paragraph",
        "Deprecated:",
        "@deprecated",
    ];
    for line in rendered.lines() {
        if fixture_fragments
            .iter()
            .any(|fragment| line.contains(fragment))
        {
            assert!(
                line.chars().count() <= 88,
                "documentation line exceeds 88 columns ({}): {line}",
                line.chars().count()
            );
        }
    }
}

#[test]
fn go_hostile_documentation_is_wrapped_and_syntax_safe() {
    let output = generate(nexgen::language::Language::Go, None);
    let rendered = joined_files(&output, "go");
    assert!(rendered.contains("// Hostile title */ & <summary>\n//\n// Type prose"));
    assert!(
        rendered.contains("//\n// Type second paragraph has \\ and C:\\unicorn plus triple quotes")
    );
    assert!(rendered.contains("//\n// Deprecated: This type is deprecated."));
    assert!(rendered.contains("\t// Value title */ & <summary>\n\t//\n\t// Field prose"));
    assert!(rendered.contains("\t//\n\t// Deprecated: This field is deprecated."));
    assert!(rendered.contains("// Service second paragraph."));
    assert!(rendered.contains("// \\+build ignore"));
    assert!(rendered.contains("// +builder is ordinary prose"));
    assert!(!rendered.contains("// \\+builder"));
    assert!(rendered.contains("// go\\:generate hostile-tool"));
    assert!(rendered.contains("\t// Operation second paragraph."));
    assert_hostile_comment_lines_fit(&rendered);

    for path in read_files(&output, "go").keys() {
        let status = Command::new("gofmt")
            .arg("-w")
            .arg(output.join(path))
            .status()
            .unwrap();
        assert!(status.success(), "gofmt rejected {}", path.display());
        let second_pass = Command::new("gofmt")
            .arg("-l")
            .arg(output.join(path))
            .output()
            .unwrap();
        assert!(second_pass.status.success(), "gofmt recheck failed");
        assert!(
            second_pass.stdout.is_empty(),
            "gofmt was not idempotent for {}: {}",
            path.display(),
            String::from_utf8_lossy(&second_pass.stdout)
        );
    }
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn typescript_hostile_documentation_is_wrapped_escaped_and_typechecks() {
    let output = generate(nexgen::language::Language::TypeScript, None);
    let rendered = joined_files(&output, "ts");
    assert!(rendered.contains(" * Hostile title * / & <summary>\n *\n * Type prose"));
    assert!(
        rendered.contains(" *\n * Type second paragraph has \\ and C:\\unicorn plus triple quotes")
    );
    assert!(rendered.contains(" *\n * @deprecated\n */\nexport interface Hostile"));
    assert!(rendered.contains("   * Value title * / & <summary>\n   *\n   * Field prose"));
    assert!(rendered.contains("   *\n   * @deprecated\n   */\n  value: string;"));
    assert!(rendered.contains(" * Service second paragraph."));
    assert!(rendered.contains("   * Operation second paragraph."));
    assert!(!rendered.contains("containing */"));
    assert_hostile_comment_lines_fit(&rendered);

    let root = project_root();
    let typescript_root = root.join("samples/typescript");
    ensure_typescript_dependencies(&typescript_root);
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        typescript_root.join("node_modules"),
        output.parent().unwrap().join("node_modules"),
    )
    .unwrap();
    let mut command = Command::new("npm");
    command.current_dir(&typescript_root).args([
        "exec",
        // Fail instead of fetching a same-named package from the registry when
        // the local compiler is missing.
        "--no",
        "--",
        "tsc",
        "--ignoreConfig",
        "--noEmit",
        "--target",
        "ES2022",
        "--lib",
        "ES2022,esnext.temporal",
        "--module",
        "ES2022",
        "--moduleResolution",
        "bundler",
        "--types",
        "node",
        "--strict",
        "--skipLibCheck",
        "--allowImportingTsExtensions",
        "--esModuleInterop",
    ]);
    for path in read_files(&output, "ts").keys() {
        command.arg(output.join(path));
    }
    assert!(
        command.status().unwrap().success(),
        "hostile TypeScript did not typecheck"
    );
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn python_hostile_documentation_is_wrapped_escaped_and_parses() {
    let output = generate(nexgen::language::Language::Python, None);
    let rendered = joined_files(&output, "py");
    assert!(rendered.contains("\"\"\"Hostile title */ & <summary>\n\n    Type prose"));
    assert!(
        rendered.contains("Type second paragraph has \\\\ and C:\\\\unicorn plus triple quotes")
    );
    assert!(rendered.contains("\"\"\"Value title */ & <summary>\n\n    Field prose"));
    assert!(rendered.contains("Service second paragraph."));
    assert!(rendered.contains("Operation second paragraph."));
    assert_hostile_comment_lines_fit(&rendered);

    let checker = r#"import ast, pathlib, sys
for path in pathlib.Path(sys.argv[1]).rglob("*.py"):
    ast.parse(path.read_text(), filename=str(path), feature_version=(3, 10))
"#;
    let status = Command::new("python3")
        .args(["-c", checker, output.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "hostile Python did not parse as Python 3.10"
    );
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}

#[test]
fn java_hostile_documentation_is_wrapped_escaped_and_compiles() {
    let output = generate(
        nexgen::language::Language::Java,
        Some("json_schema.hostile"),
    );
    let rendered = joined_files(&output, "java");
    assert!(rendered.contains(" * Hostile title * / &amp; &lt;summary&gt;\n *\n * Type prose"));
    assert!(
        rendered.contains(
            " *\n * Type second paragraph has &#92; and C:&#92;unicorn plus triple quotes"
        )
    );
    assert!(rendered.contains(" *\n * @deprecated This type is deprecated.\n */\n@Deprecated"));
    assert!(
        rendered
            .contains("     * Value title * / &amp; &lt;summary&gt;\n     *\n     * Field prose")
    );
    assert!(rendered.contains(" * Service second paragraph."));
    assert!(rendered.contains("     * Operation second paragraph."));
    assert!(!rendered.contains("containing */"));
    assert!(!rendered.contains("<service>"));
    assert_hostile_comment_lines_fit(&rendered);

    let root = project_root();
    let java_root = root.join("samples/java");
    let init_script = output.parent().unwrap().join("compile-hostile.gradle");
    let destination = output.parent().unwrap().join("classes");
    fs::write(
        &init_script,
        r#"allprojects {
    afterEvaluate {
        tasks.register("compileHostileDocumentation", JavaCompile) {
            source = fileTree(System.getProperty("hostileSource"))
            classpath = sourceSets.main.compileClasspath
            destinationDirectory = file(System.getProperty("hostileClasses"))
            options.release = 8
        }
    }
}
"#,
    )
    .unwrap();
    let mut command = Command::new("./gradlew");
    command
        .current_dir(java_root)
        .arg("--no-daemon")
        .arg("-I")
        .arg(&init_script)
        .arg(format!("-DhostileSource={}", output.display()))
        .arg(format!("-DhostileClasses={}", destination.display()))
        .arg("compileHostileDocumentation");
    let homebrew_java_home =
        Path::new("/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home");
    if homebrew_java_home.is_dir() {
        command.env("JAVA_HOME", homebrew_java_home);
    }
    let status = command.status().unwrap();
    assert!(status.success(), "hostile Java did not compile");
    fs::remove_dir_all(output.parent().unwrap()).unwrap();
}
