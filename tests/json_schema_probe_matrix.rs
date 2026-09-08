//! Does the generator's output *compile*?
//!
//! `tests/generate_go.rs` renders to a `String` and greps it; the TypeScript,
//! Java and Python generator tests are text assertions. Only the committed
//! samples ever reached a real compiler, so a schema shape absent from them
//! could — and repeatedly did — emit code that no toolchain accepts: five Go
//! build breaks, an unparseable TypeScript `if () {`, a Python
//! `IndentationError` at import.
//!
//! This file closes that hole from the other side. It generates a matrix of
//! deliberately awkward schemas and runs the **real** toolchain over
//! **unformatted** output:
//!
//! | target | build | evaluate |
//! |---|---|---|
//! | Go | `go vet` (which type-checks) | — |
//! | TypeScript | `tsc --noEmit` | a module import |
//! | Python | `py_compile`, on the declared 3.10 floor too | a module import |
//! | Java | `javac --release 8` | — |
//!
//! Unformatted matters: `ruff format` reparses and rewrites, which is why a
//! nested same-quote f-string that is a `SyntaxError` below Python 3.12 shipped
//! in the samples. Evaluating matters: a pinned `pattern` that Rust's `regex`
//! accepts and ECMA-262-with-`u` rejects type-checks cleanly and throws
//! `SyntaxError` from `new RegExp` at import.
//!
//! A probe that a target still fails carries a `broken` row naming the finding.
//! The row must match, and must keep matching: a probe that starts passing fails
//! the test so the marker cannot outlive the bug.

mod toolchain;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use toolchain::{TARGETS, Target, Workspace, brief, command, repository_root, run};

/// A target that legitimately refuses to *load* a schema the others accept.
///
/// P15 scopes identifier validity to the **emitted target**, not to the schema:
/// a name that is invalid in a language you are not generating produces no
/// diagnostic (`features/properties.md:132-144`). So a four-way load
/// disagreement is the system working as specified — it is only a four-way
/// disagreement about the accepted/rejected **wire** value set that P1 forbids,
/// and that is the conformance driver's business, not this file's.
///
/// Declaring one here excludes the target from the build stages (there is
/// nothing to compile) without recording a failure. The declaration is still
/// live: if the target starts loading the schema, the probe fails.
struct ScopedLoad {
    target: Target,
    /// Substring the diagnostic must contain, so the row cannot drift on to a
    /// different rejection.
    diagnostic: &'static str,
    /// Why this target, and only this target, is right to refuse.
    rationale: &'static str,
}

/// A target that currently fails a probe, and the finding that owns it.
struct Broken {
    target: Target,
    /// Substring that must appear in the diagnostic, so the row cannot drift on
    /// to a different failure.
    diagnostic: &'static str,
    finding: &'static str,
}

struct MatrixProbe {
    /// Also the generated package/directory name; snake_case for Go and Python.
    id: &'static str,
    /// The model name the generator derives from the file stem.
    model: &'static str,
    intent: &'static str,
    schema: &'static str,
    /// When the loader refuses the shape for **every** target, the substring its
    /// diagnostic must contain. The probe then asserts only that — there is
    /// nothing to compile.
    load_rejected: Option<&'static str>,
    /// Targets whose load verdict is *supposed* to differ from the others'.
    scoped_load: &'static [ScopedLoad],
    broken: &'static [Broken],
}

/// The shapes that broke, plus the neighbours of each.
static PROBES: &[MatrixProbe] = &[
    MatrixProbe {
        id: "closed_empty_object",
        model: "ClosedEmptyObject",
        intent: "A closed object with no declared members: the unknown-key check has zero terms to join.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: ClosedEmptyObject
type: object
additionalProperties: false
required: [inner]
properties:
  inner:
    description: A closed object that declares nothing at all.
    type: object
    additionalProperties: false
    properties: {}
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "nullable_scalars",
        model: "NullableScalars",
        intent: "Nullable non-string members of every kind — the wrapper carries no type, so a target that reads it mistypes the field or drops its constraints.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: NullableScalars
type: object
additionalProperties: false
properties:
  count:
    oneOf: [{ type: integer }, { type: 'null' }]
  ratio:
    oneOf: [{ type: number, minimum: 0, maximum: 10, multipleOf: 5 }, { type: 'null' }]
  flag:
    oneOf: [{ type: boolean }, { type: 'null' }]
  list:
    oneOf: [{ type: array, minItems: 1, items: { type: integer } }, { type: 'null' }]
  choice:
    oneOf: [{ type: string, enum: [alpha, beta] }, { type: 'null' }]
  fixed:
    oneOf: [{ type: integer, const: 3 }, { type: 'null' }]
  bounded:
    oneOf: [{ type: string, minLength: 2, maxLength: 4, pattern: '^[a-z]+$' }, { type: 'null' }]
  stamp:
    oneOf: [{ type: string, format: date-time }, { type: 'null' }]
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "encoded_bounds",
        model: "EncodedBounds",
        intent: "contentEncoding beside a string-shaped constraint: the member is bytes, so a rune count or a regex over it does not type-check.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: EncodedBounds
type: object
additionalProperties: false
properties:
  blob:
    type: string
    contentEncoding: base64
    minLength: 4
    maxLength: 40
  patterned:
    type: string
    contentEncoding: base64url
    pattern: '^[A-Za-z0-9_-]+$'
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "encoded_string_format",
        model: "EncodedStringFormat",
        intent: "A string-shaped format constrains the encoded wire string before contentEncoding materializes supported bytes.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: EncodedStringFormat
type: object
additionalProperties: false
properties:
  payload:
    type: string
    format: uri-reference
    contentEncoding: base64
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "encoded_temporal_format",
        model: "EncodedTemporalFormat",
        intent: "Temporal format and contentEncoding cannot both materialize the same string slot into incompatible native types.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: EncodedTemporalFormat
type: object
additionalProperties: false
properties:
  payload:
    type: string
    format: date-time
    contentEncoding: base64
",
        load_rejected: Some("cannot be combined with `contentEncoding`"),
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "unique_materialized",
        model: "UniqueMaterialized",
        intent: "uniqueItems over materialized elements: the duplicate key cannot be the schema's `string`, and []byte is not comparable at all.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: UniqueMaterialized
type: object
additionalProperties: false
properties:
  stamps:
    type: array
    uniqueItems: true
    items: { type: string, format: date-time }
  blobs:
    type: array
    uniqueItems: true
    items: { type: string, contentEncoding: base64 }
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "temporal_collections",
        model: "TemporalCollections",
        intent: "Arrays and typed maps of temporal members: the element loop is emitted for any format, its body only for a sibling string constraint.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: TemporalCollections
type: object
additionalProperties: false
properties:
  stamps:
    type: array
    items: { type: string, format: date-time }
  spans:
    type: array
    items: { type: string, format: duration }
  byName:
    type: object
    additionalProperties: { type: string, format: time }
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "temporal_keys",
        model: "TemporalKeys",
        intent: "A materializing format inside propertyNames: a materialized value cannot assert a key (decision D6).",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: TemporalKeys
type: object
additionalProperties: false
properties:
  byDate:
    type: object
    propertyNames: { type: string, format: date }
    additionalProperties: { type: integer }
",
        load_rejected: Some("it materializes a native date/time value"),
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "enum_default",
        model: "EnumDefault",
        intent: "An enum with a default: the accessor returns the primitive while the field is the closed defined type.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: EnumDefault
type: object
additionalProperties: false
properties:
  mode:
    type: string
    enum: [fast, slow]
    default: fast
  retries:
    type: integer
    default: 3
  tier:
    type: integer
    enum: [1, 2]
    default: 1
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "hostile_regex",
        model: "HostileRegex",
        intent: "An ordinary phone pattern: `\\-` is legal for Rust's regex and illegal under ECMA-262 with the mandatory `u` flag.",
        schema: r#"$schema: https://json-schema.org/draft/2020-12/schema
title: HostileRegex
type: object
additionalProperties: false
properties:
  phone:
    type: string
    pattern: '^\d{3}\-\d{4}$'
"#,
        load_rejected: Some("which is a JavaScript SyntaxError"),
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "portable_regex",
        model: "PortableRegex",
        intent: "Patterns the gate does accept: each has to compile in all four runtimes, including at TypeScript module-evaluation time.",
        schema: r#"$schema: https://json-schema.org/draft/2020-12/schema
title: PortableRegex
type: object
additionalProperties: false
properties:
  phone:
    type: string
    pattern: '^\d{3}-\d{4}$'
  slug:
    type: string
    pattern: '^[a-z0-9]+(?:-[a-z0-9]+)*$'
  quantified:
    type: string
    pattern: '^(?:ab|cd){1,4}$'
  classes:
    type: string
    pattern: '^[\w.-]+@[\w-]+\.[a-z]{2,}$'
  alternation:
    type: string
    pattern: '^(alpha|beta|gamma)(-\d+)?$'
"#,
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "quoted_doc",
        model: "QuotedDoc",
        intent: "A documentation string that ends in a double quote, at every level the emitters render docs.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: QuotedDoc
description: 'He said \"hi\"'
type: object
additionalProperties: false
properties:
  note:
    description: 'ends with a quote: \"'
    type: string
  other:
    description: 'a backslash \\\\ and a quote \"'
    type: string
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "count_bounds",
        model: "CountBounds",
        intent: "Array count bounds, whose reason string is built by nesting a same-quote f-string — a SyntaxError below Python 3.12.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: CountBounds
type: object
additionalProperties: false
properties:
  entries:
    type: array
    minItems: 1
    maxItems: 3
    items: { type: string }
  bag:
    type: object
    minProperties: 1
    maxProperties: 3
    additionalProperties: { type: string }
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "required_arrays",
        model: "RequiredArrays",
        intent: "Required and nullable arrays: a nil slice must not become a wire null for a non-nullable member.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: RequiredArrays
type: object
additionalProperties: false
required: [tags, maybeTags]
properties:
  tags:
    type: array
    items: { type: string }
  maybeTags:
    oneOf: [{ type: array, items: { type: string } }, { type: 'null' }]
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "reserved_methods",
        model: "ReservedMethods",
        intent: "A member whose derived identifier collides with an emitter's own fixed method name.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: ReservedMethods
type: object
additionalProperties: false
properties:
  validate: { type: string }
",
        load_rejected: None,
        scoped_load: &[ScopedLoad {
            target: Target::Go,
            diagnostic: "identifier collision in go output",
            rationale: "Go's fixed method set contains `Validate`, so P15 requires an \
                        `x-go-name` override; Java, Python and TypeScript have no such member \
                        and compile the schema unchanged. P15 scopes identifier validity to \
                        the emitted target, so the verdicts are supposed to differ.",
        }],
        broken: &[],
    },
    MatrixProbe {
        id: "reserved_nested",
        model: "ReservedNested",
        intent: "Members whose derived identifiers collide with the nested classes and runtime types an emitter synthesizes.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: ReservedNested
type: object
additionalProperties: false
properties:
  deserializer: { type: string }
  serializer: { type: string }
  violation: { type: string }
",
        load_rejected: None,
        scoped_load: &[ScopedLoad {
            target: Target::Java,
            diagnostic: "identifier collision in java output",
            rationale: "Java is the only target that nests its converter inside the model and \
                        binds `violation` as a loop variable in the same method body as the \
                        member slots, so `violation` needs an `x-java-name` override there and \
                        nowhere else. Go, Python and TypeScript synthesize no such name and \
                        compile the schema unchanged.",
        }],
        broken: &[],
    },
    MatrixProbe {
        id: "reserved_locals",
        model: "ReservedLocals",
        intent: "A member whose derived identifier collides with a local the generated deserializer binds — the scope below the model's fields and methods.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: ReservedLocals
type: object
additionalProperties: false
properties:
  index: { type: string }
  node: { type: string }
  tags:
    type: array
    items: { type: string }
",
        load_rejected: None,
        scoped_load: &[ScopedLoad {
            target: Target::Java,
            diagnostic: "generated deserializer local",
            rationale: "The Java deserializer declares each member slot at method scope, and \
                        Java forbids both a duplicate at that scope and a nested block \
                        redeclaring an enclosing local — so `index` beside any array member is \
                        `variable index is already defined in method deserialize(...)`. The \
                        other three bind nothing a member can collide with: Go's parse locals \
                        are unexported and prefixed, Python's are `_`-suffixed, and TypeScript \
                        destructures into a fresh scope. P15 scopes identifier validity to the \
                        emitted target, so the verdicts are supposed to differ.",
        }],
        broken: &[],
    },
    MatrixProbe {
        id: "format_matrix",
        model: "FormatMatrix",
        intent: "Every asserted non-materializing format in one model, so each pinned regex has to compile in all four runtimes.",
        schema: "\
$schema: https://json-schema.org/draft/2020-12/schema
title: FormatMatrix
type: object
additionalProperties: false
properties:
  id: { type: string, format: uuid }
  host: { type: string, format: hostname }
  mail: { type: string, format: email }
  addr: { type: string, format: ipv4 }
  addr6: { type: string, format: ipv6 }
  link: { type: string, format: uri }
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
    MatrixProbe {
        id: "deprecated_operation",
        model: "ProbeInput",
        intent: "A deprecated operation in a definitions-only package: the deprecation annotation is evaluated at import.",
        schema: "\
nexusrpc: '1.0.0'
$schema: https://json-schema.org/draft/2020-12/schema
description: A service whose only operation is deprecated.
services:
  ProbeService:
    fqn: probe.v1.ProbeService
    description: Probe service.
    operations:
      legacyPing:
        description: Deprecated liveness probe.
        deprecated: true
        input: { $ref: '#/$defs/ProbeInput' }
$defs:
  ProbeInput:
    description: The deprecated operation's input.
    type: object
    additionalProperties: false
    properties:
      nonce: { type: string }
",
        load_rejected: None,
        scoped_load: &[],
        broken: &[],
    },
];

#[derive(Default)]
struct Failures(BTreeMap<(String, Target), Vec<String>>);

impl Failures {
    fn record(&mut self, probe: &str, target: Target, stage: &str, detail: &str) {
        self.0
            .entry((probe.to_string(), target))
            .or_default()
            .push(format!("{stage}: {}", brief(detail)));
    }
}

fn write_schema(workspace: &Workspace, probe: &MatrixProbe) -> PathBuf {
    let schemas = workspace.root().join("schemas");
    fs::create_dir_all(&schemas).expect("create schema directory");
    // The generator derives the root model's name from the file stem, so the
    // file name is part of the probe.
    let path = schemas.join(format!("{}.yaml", probe.model.replace(' ', "")));
    fs::write(&path, probe.schema).expect("write probe schema");
    path
}

/// Which probe a diagnostic belongs to, by looking for its directory name.
fn attribute<'a>(line: &str, ids: &'a [&'a str]) -> Option<&'a str> {
    ids.iter()
        .copied()
        .find(|id| line.contains(&format!("/{id}/")) || line.starts_with(&format!("{id}/")))
}

#[test]
fn generated_output_compiles_for_every_probe_schema() {
    let workspace = Workspace::new("probe-matrix");
    let mut failures = Failures::default();
    let mut compiled: BTreeSet<(String, Target)> = BTreeSet::new();
    let mut scoped_loads: Vec<String> = Vec::new();

    let mut generated: Vec<&MatrixProbe> = Vec::new();
    let mut per_target: BTreeMap<Target, Vec<&MatrixProbe>> = BTreeMap::new();
    for probe in PROBES {
        let schema = write_schema(&workspace, probe);
        let mut load_errors: BTreeMap<Target, String> = BTreeMap::new();
        for target in TARGETS {
            if let Err(error) = workspace.generate(target, &schema, probe.id) {
                load_errors.insert(target, error);
            }
        }
        if load_errors.len() == TARGETS.len() {
            let expected = probe.load_rejected.unwrap_or_else(|| {
                panic!(
                    "{}: the loader rejected a shape the matrix expects it to accept:\n{}",
                    probe.id,
                    load_errors.values().next().expect("a diagnostic")
                )
            });
            for (target, error) in &load_errors {
                assert!(
                    error.contains(expected),
                    "{} / {target}: rejected, but not for {expected:?}:\n{error}",
                    probe.id
                );
            }
            continue;
        }
        assert!(
            probe.load_rejected.is_none(),
            "{}: the loader now accepts it for at least one target, so its expected rejection is stale",
            probe.id
        );
        // A load verdict that differs per target is only a defect when it is
        // undeclared. P15 scopes identifier validity to the emitted target, so a
        // target listed in `scoped_load` is *supposed* to refuse; anything else
        // is a real per-target load divergence and is recorded as that target's
        // failure.
        for (target, error) in &load_errors {
            match probe
                .scoped_load
                .iter()
                .find(|scoped| scoped.target == *target)
            {
                Some(scoped) => {
                    assert!(
                        error.contains(scoped.diagnostic),
                        "{} / {target}: refuses to load, but not for {:?} ({}):\n{error}",
                        probe.id,
                        scoped.diagnostic,
                        scoped.rationale
                    );
                    scoped_loads.push(format!(
                        "{} / {target}: load scoped to the emitted target — {}",
                        probe.id, scoped.rationale
                    ));
                }
                None => failures.record(probe.id, *target, "load", error),
            }
        }
        // A declared scoped load that no longer happens is stale: the target
        // started accepting the schema, and the declaration has to go.
        for scoped in probe.scoped_load {
            assert!(
                load_errors.contains_key(&scoped.target),
                "{} / {}: now loads, so its scoped_load declaration is stale — delete it",
                probe.id,
                scoped.target
            );
        }
        generated.push(probe);
        for target in TARGETS {
            if !load_errors.contains_key(&target) {
                per_target.entry(target).or_default().push(probe);
            }
        }
    }
    let empty: Vec<&MatrixProbe> = Vec::new();
    let for_target = |target: Target| per_target.get(&target).unwrap_or(&empty).clone();
    let ids: Vec<&str> = generated.iter().map(|probe| probe.id).collect();

    check_go(
        &workspace,
        &for_target(Target::Go),
        &mut failures,
        &mut compiled,
    );
    check_java(
        &workspace,
        &for_target(Target::Java),
        &mut failures,
        &mut compiled,
    );
    check_python(
        &workspace,
        &for_target(Target::Python),
        &mut failures,
        &mut compiled,
    );
    check_typescript(
        &workspace,
        &for_target(Target::TypeScript),
        &ids,
        &mut failures,
        &mut compiled,
    );

    let mut report = Vec::new();
    let mut open = Vec::new();
    for probe in &generated {
        for target in TARGETS {
            let key = (probe.id.to_string(), target);
            let observed = failures.0.get(&key);
            let expected = probe.broken.iter().find(|entry| entry.target == target);
            match (observed, expected) {
                (Some(details), None) => report.push(format!(
                    "{} / {target}: generated output does not build\n    ({})\n    {}",
                    probe.id,
                    probe.intent,
                    details.join("\n    ")
                )),
                (None, Some(entry)) => {
                    assert!(
                        compiled.contains(&key),
                        "{} / {target}: neither built nor failed — the harness did not run it",
                        probe.id
                    );
                    report.push(format!(
                        "{} / {target}: now builds, so its `broken` row for {} is stale — delete it",
                        probe.id, entry.finding
                    ));
                }
                (Some(details), Some(entry)) => {
                    let joined = details.join("\n    ");
                    if joined.contains(entry.diagnostic) {
                        open.push(format!("{} / {target} pins {}", probe.id, entry.finding));
                    } else {
                        report.push(format!(
                            "{} / {target}: fails, but not with {:?} ({})\n    {joined}",
                            probe.id, entry.diagnostic, entry.finding
                        ));
                    }
                }
                (None, None) => {}
            }
        }
    }

    if !scoped_loads.is_empty() {
        eprintln!(
            "load verdicts scoped to the emitted target (by design, not defects):\n  {}",
            scoped_loads.join("\n  ")
        );
    }
    if !open.is_empty() {
        eprintln!("still open:\n  {}", open.join("\n  "));
    }
    assert!(
        report.is_empty(),
        "the probe matrix found generated code no toolchain accepts:\n\n{}",
        report.join("\n\n")
    );
}

fn check_go(
    workspace: &Workspace,
    probes: &[&MatrixProbe],
    failures: &mut Failures,
    compiled: &mut BTreeSet<(String, Target)>,
) {
    let root = match toolchain::prepare_go_module(workspace) {
        Ok(root) => root,
        Err(error) => panic!("could not prepare the Go probe module: {error}"),
    };
    for probe in probes {
        compiled.insert((probe.id.to_string(), Target::Go));
        // `go vet` type-checks before it analyses, so it subsumes `go build` and
        // additionally reports the shadowing and printf defects a build ignores.
        let vet = run(command("go")
            .current_dir(&root)
            .arg("vet")
            .arg(format!("./{}", probe.id)));
        if !vet.ok {
            failures.record(probe.id, Target::Go, "go vet", &vet.detail);
        }
    }
}

fn check_java(
    workspace: &Workspace,
    probes: &[&MatrixProbe],
    failures: &mut Failures,
    compiled: &mut BTreeSet<(String, Target)>,
) {
    let dirs: Vec<String> = probes.iter().map(|probe| probe.id.to_string()).collect();
    let (_, broken) = match toolchain::compile_java(workspace, &dirs) {
        Ok(result) => result,
        Err(error) => panic!("could not compile the Java probes: {error}"),
    };
    for probe in probes {
        compiled.insert((probe.id.to_string(), Target::Java));
        if let Some(error) = broken.get(probe.id) {
            failures.record(probe.id, Target::Java, "javac --release 8", error);
        }
    }
}

fn python_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "py"))
        .collect();
    out.sort();
    out
}

fn check_python(
    workspace: &Workspace,
    probes: &[&MatrixProbe],
    failures: &mut Failures,
    compiled: &mut BTreeSet<(String, Target)>,
) {
    let root = workspace.target_root(Target::Python);
    let runner = root.join("smoke.py");
    fs::create_dir_all(&root).expect("create python probe root");
    fs::write(
        &runner,
        include_str!("../samples/conformance/runners/smoke.py"),
    )
    .expect("write python smoke runner");

    // The declared floor, not just the interpreter that happens to be installed:
    // an f-string nesting rule relaxed in 3.12 makes a 3.13-clean file a
    // SyntaxError on 3.10.
    let mut interpreters: Vec<(String, Vec<String>)> = vec![(
        "py_compile".to_string(),
        vec![
            toolchain::python_interpreter()
                .to_string_lossy()
                .into_owned(),
        ],
    )];
    for version in ["3.10", "3.11"] {
        let argv = vec![
            "uv".to_string(),
            "run".to_string(),
            "--no-project".to_string(),
            "--quiet".to_string(),
            "--python".to_string(),
            version.to_string(),
            "python".to_string(),
        ];
        // Only sweep a version uv can actually produce here. An unavailable
        // interpreter is a missing check, not a generator defect, so it must not
        // masquerade as one.
        let (program, leading) = argv.split_first().expect("interpreter argv");
        if run(command(program).args(leading).args(["-c", "pass"])).ok {
            interpreters.push((format!("py_compile ({version})"), argv));
        } else {
            eprintln!("skipping the Python {version} sweep: uv cannot provide that interpreter");
        }
    }

    for probe in probes {
        compiled.insert((probe.id.to_string(), Target::Python));
        let files = python_files(&root.join(probe.id));
        for (stage, argv) in &interpreters {
            let (program, leading) = argv.split_first().expect("interpreter argv");
            let compile = run(command(program)
                .current_dir(&root)
                .args(leading)
                .args(["-m", "py_compile"])
                .args(&files));
            if !compile.ok {
                failures.record(probe.id, Target::Python, stage, &compile.detail);
            }
        }
    }

    let result_path = root.join("smoke.json");
    let execution = run(toolchain::python_runtime_command()
        .expect("provision the Python probe runtime")
        .current_dir(&root)
        .arg(&runner)
        .arg(&result_path)
        .arg(&root)
        .args(probes.iter().map(|probe| probe.id)));
    if !execution.ok {
        panic!(
            "the Python import smoke runner failed:\n{}",
            execution.detail
        );
    }
    let smoke: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(&result_path).expect("read python smoke result"))
            .expect("parse python smoke result");
    for (package, outcome) in smoke {
        if outcome != "ok" {
            failures.record(&package, Target::Python, "import", &outcome);
        }
    }
}

fn check_typescript(
    workspace: &Workspace,
    probes: &[&MatrixProbe],
    ids: &[&str],
    failures: &mut Failures,
    compiled: &mut BTreeSet<(String, Target)>,
) {
    let root = workspace.target_root(Target::TypeScript);
    fs::create_dir_all(&root).expect("create typescript probe root");
    if let Err(error) = toolchain::link_node_modules(&root) {
        panic!("could not link the samples' node_modules: {error}");
    }
    let samples = repository_root().join("samples/typescript");
    fs::write(
        root.join("tsconfig.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "compilerOptions": {
                "target": "ES2022",
                "lib": ["ES2022", "esnext.temporal"],
                "module": "ES2022",
                "moduleResolution": "bundler",
                "types": ["node"],
                "strict": true,
                "noEmit": true,
                "skipLibCheck": true,
                "esModuleInterop": true,
            },
            "include": ["**/*.ts"],
        }))
        .expect("render tsconfig"),
    )
    .expect("write tsconfig");
    fs::write(
        root.join("package.json"),
        "{\n  \"name\": \"nexgen-probe-matrix\",\n  \"private\": true,\n  \"type\": \"module\"\n}\n",
    )
    .expect("write package.json");
    for probe in probes {
        compiled.insert((probe.id.to_string(), Target::TypeScript));
    }

    let tsc = samples.join("node_modules/.bin/tsc");
    let check = run(command(&tsc.to_string_lossy())
        .current_dir(&root)
        .args(["--noEmit", "--pretty", "false"]));
    if !check.ok {
        let mut unattributed = Vec::new();
        for line in check
            .detail
            .lines()
            .filter(|line| line.contains(": error TS"))
        {
            match attribute(line, ids) {
                Some(id) => failures.record(id, Target::TypeScript, "tsc --noEmit", line),
                None => unattributed.push(line.to_string()),
            }
        }
        assert!(
            unattributed.is_empty(),
            "tsc reported errors outside any probe package:\n{}",
            unattributed.join("\n")
        );
    }

    fs::write(
        root.join("registry.ts"),
        toolchain::typescript_registry_of(
            &probes
                .iter()
                .map(|probe| {
                    (
                        probe.id.to_string(),
                        probe.id.to_string(),
                        probe.model.to_string(),
                    )
                })
                .collect::<Vec<_>>(),
        ),
    )
    .expect("write registry");
    fs::write(
        root.join("smoke.test.ts"),
        include_str!("../samples/conformance/runners/smoke.test.ts"),
    )
    .expect("write smoke test");
    let result_path = root.join("smoke.json");
    let vitest = samples.join("node_modules/.bin/vitest");
    let execution = run(command(&vitest.to_string_lossy())
        .current_dir(&root)
        .env("NEXGEN_CONFORMANCE_RESULT", &result_path)
        .args(["run", "--root", ".", "smoke.test.ts"]));
    if !execution.ok {
        panic!(
            "the TypeScript import smoke runner failed:\n{}",
            execution.detail
        );
    }
    let smoke: BTreeMap<String, String> = serde_json::from_str(
        &fs::read_to_string(&result_path).expect("read typescript smoke result"),
    )
    .expect("parse typescript smoke result");
    for (id, outcome) in smoke {
        if outcome != "ok" {
            failures.record(&id, Target::TypeScript, "import", &outcome);
        }
    }
}
