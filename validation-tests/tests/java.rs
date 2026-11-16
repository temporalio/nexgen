//! Generated Java compilation and runtime integration tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nexgen::{GenerateRequest, generate_to_file};

mod support;
use support::common;

#[path = "support/json_schema/selected_tests.rs"]
mod json_schema_tests;

const JSON_SCHEMA_TARGET: support::json_schema::toolchain::Target =
    support::json_schema::toolchain::Target::Java;

use common::write_bare_ref_alias_closure;

#[test]
fn java_hostile_documentation_is_wrapped_escaped_and_compiles() {
    support::doc_rendering::java_hostile_documentation_is_wrapped_escaped_and_compiles();
}

static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn unique_output_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"))
}

fn read_java_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("java") {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read_to_string(&path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(dir, dir, &mut files);
    files
}

#[test]
fn java_json_authored_datetime_does_not_shadow_offset_date_time() {
    let temp_dir = unique_output_path("java-json-datetime-shadow");
    let gradle_root = temp_dir.join("project");
    fs::create_dir_all(&gradle_root).unwrap();
    fs::copy(
        project_root().join("samples/java/build.gradle"),
        gradle_root.join("build.gradle"),
    )
    .unwrap();
    fs::copy(
        project_root().join("samples/java/settings.gradle"),
        gradle_root.join("settings.gradle"),
    )
    .unwrap();
    let input_path = temp_dir.join("shadow.nexusrpc.yaml");
    fs::write(
        &input_path,
        r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
$defs:
  DateTime:
    type: object
    properties:
      label: { type: string }
  Event:
    type: object
    required: [authored, timestamp, nullableCollection, nullableElement]
    properties:
      authored: { $ref: "#/$defs/DateTime" }
      timestamp: { type: string, format: date-time }
      optionalTimestamp: { type: string, format: date-time }
      optionalCollection:
        type: array
        items: { type: string, format: date-time }
      nullableCollection:
        oneOf:
          - type: array
            items: { type: string, format: date-time }
          - { type: "null" }
      nullableElement:
        type: array
        items:
          oneOf:
            - { type: string, format: date-time }
            - { type: "null" }
      bothCombined:
        oneOf:
          - type: array
            items:
              oneOf:
                - { type: string, format: date-time }
                - { type: "null" }
          - { type: "null" }
"##,
    )
    .unwrap();
    let output_path = gradle_root.join("src/main/java/example/shadow");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Java,
        input_paths: vec![input_path],
        support_package: None,
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: Some("example.shadow".to_string()),
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();

    let event = fs::read_to_string(output_path.join("Event.java")).unwrap();
    assert!(event.contains("OffsetDateTime timestamp"), "{event}");
    assert!(
        event.contains("@Nullable OffsetDateTime optionalTimestamp"),
        "{event}"
    );
    for declaration in [
        "@Nullable List<OffsetDateTime> optionalCollection",
        "@Nullable List<OffsetDateTime> nullableCollection",
        "List<@Nullable OffsetDateTime> nullableElement",
        "@Nullable List<@Nullable OffsetDateTime> bothCombined",
    ] {
        assert!(event.contains(declaration), "{declaration}\n{event}");
    }
    assert!(event.contains("DateTime authored"), "{event}");
    assert!(event.contains("import java.time.OffsetDateTime;"));

    let test_package = gradle_root.join("src/test/java/example/shadow");
    fs::create_dir_all(&test_package).unwrap();
    fs::write(
        test_package.join("DateTimeTypeIdentityTest.java"),
        r#"package example.shadow;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.time.OffsetDateTime;
import org.junit.jupiter.api.Test;

final class DateTimeTypeIdentityTest {
    @Test
    void authoredAndTemporalDateTimesRemainDistinct() throws Exception {
        assertEquals(DateTime.class, Event.class.getMethod("getAuthored").getReturnType());
        assertEquals(OffsetDateTime.class, Event.class.getMethod("getTimestamp").getReturnType());
    }

    @Test
    void collectionAndElementNullabilityRemainIndependent() throws Exception {
        ObjectMapper mapper = new ObjectMapper();
        Event event = mapper.readValue(
            "{\"authored\":{},\"timestamp\":\"2024-01-02T03:04:05Z\"," +
            "\"nullableCollection\":null," +
            "\"nullableElement\":[null,\"2024-01-02T03:04:05Z\"]," +
            "\"bothCombined\":[null]}",
            Event.class);
        assertNull(event.getOptionalCollection());
        assertNull(event.getNullableCollection());
        assertNull(event.getNullableElement().get(0));
        assertNull(event.getBothCombined().get(0));
        assertEquals(2, mapper.readValue(mapper.writeValueAsString(event), Event.class)
            .getNullableElement().size());
    }
}
"#,
    )
    .unwrap();
    let status = Command::new(project_root().join("samples/java/gradlew"))
        .args(["test", "--no-daemon"])
        .current_dir(&gradle_root)
        .status()
        .expect("run Gradle DateTime identity test");
    assert!(status.success());
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn java_json_bare_ref_root_alias_collapses_to_the_target_class() {
    let temp_dir = unique_output_path("java-json-bare-ref-alias");
    let input = write_bare_ref_alias_closure(&temp_dir);
    let gradle_root = temp_dir.join("project");
    fs::create_dir_all(&gradle_root).unwrap();
    fs::copy(
        project_root().join("samples/java/build.gradle"),
        gradle_root.join("build.gradle"),
    )
    .unwrap();
    fs::copy(
        project_root().join("samples/java/settings.gradle"),
        gradle_root.join("settings.gradle"),
    )
    .unwrap();
    let output_path = gradle_root.join("src/main/java/example/aliases");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Java,
        input_paths: vec![input],
        support_package: None,
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        config: Default::default(),
        java_package_name: Some("example.aliases".to_string()),
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();

    assert!(output_path.join("target/main/Main.java").is_file());
    assert!(
        !output_path.join("alias/alternate/Alternate.java").exists(),
        "Java must not synthesize a class for a bare-ref alias"
    );
    assert!(
        !output_path.join("target/main/Mirror.java").exists(),
        "Java must also collapse a bare-ref `$defs` alias"
    );
    let service = fs::read_to_string(output_path.join("service/AliasService.java")).unwrap();
    assert!(
        service.contains("import example.aliases.target.main.Main;"),
        "{service}"
    );
    assert!(service.contains("Main echo(Main input);"), "{service}");
    assert!(!service.contains("Alternate"), "{service}");
    let holder = fs::read_to_string(output_path.join("service/Holder.java")).unwrap();
    assert!(holder.contains("private final Main item;"), "{holder}");
    assert!(!holder.contains("Alternate"), "{holder}");

    let test_package = gradle_root.join("src/test/java/example/aliases");
    fs::create_dir_all(&test_package).unwrap();
    fs::write(
        test_package.join("BareAliasTest.java"),
        r#"package example.aliases;

import static org.junit.jupiter.api.Assertions.assertEquals;

import com.fasterxml.jackson.databind.ObjectMapper;
import example.aliases.service.Holder;
import example.aliases.target.main.Main;
import org.junit.jupiter.api.Test;

final class BareAliasTest {
    @Test
    void aliasPositionsUseTheTargetClassAtRuntime() throws Exception {
        Main value = new Main("ok");
        Holder holder = new Holder(value);
        assertEquals(value, holder.getItem());

        ObjectMapper mapper = new ObjectMapper();
        Main decoded = mapper.readValue(mapper.writeValueAsBytes(value), Main.class);
        assertEquals(value, decoded);
    }
}
"#,
    )
    .unwrap();
    let status = Command::new(project_root().join("samples/java/gradlew"))
        .args(["--no-daemon", "test"])
        .current_dir(&gradle_root)
        .status()
        .expect("run Gradle alias compile/runtime test");
    assert!(status.success(), "Gradle alias test failed: {status}");
    fs::remove_dir_all(temp_dir).unwrap();
}

/// The pinned temporal grammar admits a fractional second of any width, and
/// every target keeps what its own type can hold rather than rejecting: Go, Java
/// and TypeScript at nanoseconds, Python at microseconds. That is P1's exception
/// (b) — loss only at the target type's genuine capacity limit — and it is what
/// `samples/python/tests/test_temporal.py` already pins. `java.time`'s ISO
/// parser throws past nine digits, so `TemporalSupport` truncates first
/// (`new:java-rejects-12-digit-fraction`).
#[test]
fn java_json_truncates_an_over_long_fractional_second() {
    let temp_dir = unique_output_path("java-json-fractional-second");
    let gradle_root = temp_dir.join("project");
    fs::create_dir_all(&gradle_root).unwrap();
    fs::copy(
        project_root().join("samples/java/build.gradle"),
        gradle_root.join("build.gradle"),
    )
    .unwrap();
    fs::copy(
        project_root().join("samples/java/settings.gradle"),
        gradle_root.join("settings.gradle"),
    )
    .unwrap();
    let input_path = temp_dir.join("fracsec.yaml");
    fs::write(
        &input_path,
        r##"$schema: https://json-schema.org/draft/2020-12/schema
title: Fractional Defaults
type: object
properties:
  ts: { type: string, format: date-time }
  tod: { type: string, format: time }
  wideDefault:
    type: string
    format: date-time
    default: "2021-06-15t12:30:45.123456789012+14:00"
  plusBoundary:
    type: string
    format: date-time
    default: "2021-06-15T12:30:45+14:00"
  minusBoundary:
    type: string
    format: date-time
    default: "2021-06-15T12:30:45-14:00"
"##,
    )
    .unwrap();
    let output_path = gradle_root.join("src/main/java/fracsec");

    generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Java,
        input_paths: vec![input_path],
        support_package: None,
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        java_package_name: Some("fracsec".to_string()),
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap();

    let rendered = read_java_files(&output_path);
    let support = &rendered[&PathBuf::from("TemporalSupport.java")];
    // The grammar still admits any width — the cap is not in the regex.
    assert!(
        support.contains("(\\\\.[0-9]+)?"),
        "the pinned fraction must stay unbounded\n{support}"
    );
    // The typed date-time truncates to OffsetDateTime's nanoseconds;
    // string-carried time keeps every digit and uses owned string arithmetic.
    for expected in [
        "private static String truncateFraction(String value) {",
        "public static OffsetDateTime parseDateTimeLiteral(String value) {",
        "return OffsetDateTime.parse(truncateFraction(value).toUpperCase());",
        "return parseDateTimeLiteral(value);",
        "return canonicalTime(value);",
    ] {
        assert!(support.contains(expected), "{expected}\n{support}");
    }
    assert!(
        !support.contains("LocalDate.parse(truncateFraction("),
        "a date carries no fraction\n{support}"
    );

    let model = &rendered[&PathBuf::from("Fracsec.java")];
    assert!(
        model.contains(
            "TemporalSupport.parseDateTimeLiteral(\"2021-06-15T12:30:45.123456789012+14:00\")"
        ),
        "{model}"
    );

    let test_package = gradle_root.join("src/test/java/fracsec");
    fs::create_dir_all(&test_package).unwrap();
    fs::write(
        test_package.join("FractionalDefaultsTest.java"),
        r#"package fracsec;

import static org.junit.jupiter.api.Assertions.assertEquals;
import java.time.OffsetDateTime;
import java.time.ZoneOffset;
import java.util.Collections;
import org.junit.jupiter.api.Test;

final class FractionalDefaultsTest {
    @Test
    void dateTimeDefaultsUseTheMaterializedParser() {
        Fracsec value = new Fracsec(null, null, null, null, null, Collections.emptyMap());
        assertEquals(
            OffsetDateTime.parse("2021-06-15T12:30:45.123456789+14:00"),
            value.getWideDefaultOrDefault());
        assertEquals(ZoneOffset.ofHours(14), value.getPlusBoundaryOrDefault().getOffset());
        assertEquals(ZoneOffset.ofHours(-14), value.getMinusBoundaryOrDefault().getOffset());
    }
}
"#,
    )
    .unwrap();
    let status = Command::new(project_root().join("samples/java/gradlew"))
        .args([
            "test",
            "--no-daemon",
            "--tests",
            "fracsec.FractionalDefaultsTest",
        ])
        .current_dir(&gradle_root)
        .status()
        .expect("run Gradle fractional date-time default test");
    assert!(status.success());

    let invalid_input = temp_dir.join("invalid-default.yaml");
    fs::write(
        &invalid_input,
        r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  invalid:
    type: string
    format: date-time
    default: "2021-06-15T12:30:45+14:01"
"#,
    )
    .unwrap();
    let error = generate_to_file(&GenerateRequest {
        config: Default::default(),
        language: nexgen::language::Language::Java,
        input_paths: vec![invalid_input],
        support_package: None,
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join("invalid"),
        format: false,
        java_package_name: Some("invalid".to_string()),
        ts_date_time_types: Default::default(),
        dotnet_namespace: None,
    })
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("is not a valid date-time") && error.contains("+14:01"),
        "{error}"
    );
    fs::remove_dir_all(temp_dir).unwrap();
}
