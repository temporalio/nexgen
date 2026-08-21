// Drives the `nexgen` binary over the WIT/proto CLI surface (`--descriptors`,
// `--native-api`, `--support-file`), all behind the `advanced` feature.
#![cfg(feature = "advanced")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use heck::ToSnakeCase;
use nexgen::SupportFiles;
use nexgen::generator::generate_source;
use nexgen::spec::SupportFragmentSpec;
use nexgen::{GenerateRequest, generate_to_file};

mod common;
use common::json_input_path;

const PRIMARY_EXAMPLE_ID: &str = "workflow-service";
const TYPE_ROUNDTRIP_EXAMPLE_ID: &str = "type-roundtrip";
static OUTPUT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A property whose union has one inline structured object branch (named
/// `<Union>Object`) and one scalar branch.
/// Unions in positions with no property of their own: an array element (inline
/// and `$ref`), a map member (inline), plus a nullable element for contrast.
const ELEMENT_UNION_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  segments:
    type: array
    items:
      oneOf:
        - { type: string }
        - { type: integer }
  choices:
    type: array
    items: { $ref: "#/$defs/Choice" }
  entries: { $ref: "#/$defs/Entries" }
  slots:
    type: array
    items:
      oneOf:
        - { type: string }
        - { type: "null" }
$defs:
  Choice:
    oneOf:
      - { type: string }
      - { type: boolean }
  Entries:
    type: object
    additionalProperties:
      oneOf:
        - { type: string }
        - { type: integer }
"##;

const INLINE_OBJECT_BRANCH_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  payload:
    oneOf:
      - type: object
        required: [text]
        properties:
          text: { type: string, minLength: 1 }
      - { type: string }
"#;

/// A union whose **non-object** branches each declare constraints of their own:
/// once the wire token selects a branch, the value is held to everything that
/// branch declares — including a closed value set — in both directions.
const BRANCH_CONSTRAINT_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  value:
    oneOf:
      - { type: string, minLength: 3, pattern: "^[a-z]+$" }
      - { type: integer, minimum: 1 }
  listOrName:
    oneOf:
      - { type: array, items: { type: number }, minItems: 1, uniqueItems: true }
      - { type: string, enum: [auto, manual] }
"#;

/// Exercises the Python-specific runtime surface that cannot be inferred from
/// annotations alone: typed extras on a mixed object, the complete scalar
/// matcher vocabulary, key-shape validation, and native closed/default values.
const PYTHON_CONFORMANCE_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [known]
properties:
  known: { type: string }
  codes:
    type: array
    items: { type: string }
    contains: { type: string, minLength: 2, pattern: "^x" }
  emails:
    type: array
    items: { type: string }
    contains: { type: string, format: email }
  integralNumbers:
    type: array
    items: { type: number }
    contains:
      type: integer
      minimum: 2
      exclusiveMaximum: 10
      multipleOf: 2
  flags:
    type: array
    items: { type: boolean }
    contains: { const: true }
  modes:
    type: array
    items: { type: string }
    contains: { enum: [fast, safe] }
  day:
    type: string
    format: date
    minLength: 10
    maxLength: 10
    pattern: "^2026-"
  fixedDay:
    type: string
    format: date
    const: "2026-08-21"
  dayChoice:
    type: string
    format: date
    enum: ["2026-08-21", "2026-08-22"]
  defaultDay:
    type: string
    format: date
    default: "2026-08-21"
  maybeDay:
    oneOf:
      - { type: string, format: date }
      - { type: "null" }
    default: "2026-08-22"
  fixedBlob:
    type: string
    contentEncoding: base64
    minLength: 4
    maxLength: 4
    pattern: "^YQ==$"
    const: "YQ=="
additionalProperties:
  $ref: "#/$defs/Extra"
minProperties: 1
$defs:
  Extra:
    type: object
    additionalProperties: false
    required: [amount]
    properties:
      amount: { type: integer, minimum: 0 }
  PatternNames:
    type: object
    additionalProperties: { type: string }
    propertyNames: { type: string, minLength: 2, maxLength: 8, pattern: "^[a-z]+$" }
  EnumNames:
    type: object
    additionalProperties: { type: string }
    propertyNames: { type: string, enum: [alpha, beta] }
  FormatNames:
    type: object
    additionalProperties: { type: string }
    propertyNames: { type: string, format: email }
"##;

const PYTHON_CONFORMANCE_RUNTIME_CHECK: &str = r#"
import datetime
import sys

root, package = sys.argv[1], sys.argv[2]
sys.path.insert(0, root)
models = __import__(package + ".models", fromlist=["*"])
definitions = __import__(package + "._definitions", fromlist=["*"])

Contract = models.Contract
Extra = models.Extra
PatternNames = models.PatternNames
EnumNames = models.EnumNames
FormatNames = models.FormatNames
ValidationError = definitions.ValidationError

def converter(model):
    return getattr(model, "__temporal_transfer_type_converter")

def violations(call):
    try:
        call()
    except ValidationError as error:
        return [(item.path, item.reason) for item in error.violations]
    raise AssertionError("expected ValidationError")

wire = {
    "known": "ok",
    "extra": {"amount": 2},
    "codes": ["no", "xyz"],
    "emails": ["bad", "a@example.com"],
    "integralNumbers": [1.5, 4],
    "day": "2026-08-21",
    "fixedDay": "2026-08-21",
    "dayChoice": "2026-08-22",
    "fixedBlob": "YQ==",
}
value = converter(Contract).from_transfer_type(wire, Contract)
assert isinstance(value.additional_properties["extra"], Extra)
assert value.additional_properties["extra"].amount == 2
assert value.day == datetime.date(2026, 8, 21)
assert value.fixed_day == datetime.date(2026, 8, 21)
assert value.day_choice == datetime.date(2026, 8, 22)
assert value.default_day == datetime.date(2026, 8, 21)
assert value.maybe_day == datetime.date(2026, 8, 22)
assert value.fixed_blob == b"a"
assert converter(Contract).to_transfer_type(value) == wire

assert violations(lambda: converter(Contract).from_transfer_type(
    {"known": "ok", "extra": {"amount": -1}}, Contract
)) == [("extra.amount", "must be >= 0, got -1")]

collision = Contract(known="ok", additional_properties={"known": Extra(amount=1)})
reported = violations(lambda: converter(Contract).to_transfer_type(collision))
assert reported == [("known", "additional property collides with declared property")], reported

# The matcher must guard the raw item type before applying numeric predicates:
# bool is an invalid number item and must not count as integer 1.
reported = violations(lambda: converter(Contract).from_transfer_type(
    {"known": "ok", "integralNumbers": [True, 3]}, Contract
))
assert reported == [
    ("integralNumbers[0]", "expected number"),
    ("integralNumbers", "no element matches the required schema"),
], reported

assert violations(lambda: converter(Contract).from_transfer_type(
    {"known": "ok", "codes": ["ab", "yellow"]}, Contract
)) == [("codes", "no element matches the required schema")]
assert violations(lambda: converter(Contract).to_transfer_type(
    Contract(known="ok", emails=["bad", "also-bad"])
)) == [("emails", "no element matches the required schema")]
assert violations(lambda: converter(Contract).from_transfer_type(
    {"known": "ok", "flags": [False, False], "modes": ["slow"]}, Contract
)) == [
    ("flags", "no element matches the required schema"),
    ("modes", "no element matches the required schema"),
]

for model, good in [
    (PatternNames, {"alpha": "x"}),
    (EnumNames, {"alpha": "x"}),
    (FormatNames, {"a@example.com": "x"}),
]:
    parsed = converter(model).from_transfer_type(good, model)
    assert converter(model).to_transfer_type(parsed) == good

reported = violations(lambda: converter(PatternNames).from_transfer_type(
    {"A": "x", "toolongkey": "y"}, PatternNames
))
assert reported == [
    ("A", 'invalid property name "A": must have length >= 2, got 1'),
    ("A", r'invalid property name "A": must match pattern ^[a-z]+\Z'),
    ("toolongkey", 'invalid property name "toolongkey": must have length <= 8, got 10'),
], reported
assert violations(lambda: converter(EnumNames).to_transfer_type(
    EnumNames(additional_properties={"gamma": "x"})
)) == [("gamma", 'invalid property name "gamma": must equal an allowed value')]
assert violations(lambda: converter(FormatNames).from_transfer_type(
    {"bad": "x"}, FormatNames
)) == [("bad", 'invalid property name "bad": must be a valid email')]

# Sibling string constraints apply to the original wire spelling and to the
# canonical string produced from a native value.
assert violations(lambda: converter(Contract).from_transfer_type(
    {"known": "ok", "day": "2025-08-21"}, Contract
)) == [("day", 'must match pattern ^2026-, got "2025-08-21"')]
changed = Contract(known="ok", day=datetime.date(2025, 8, 21))
assert violations(lambda: converter(Contract).to_transfer_type(changed)) == [
    ("day", 'must match pattern ^2026-, got "2025-08-21"')
]

wrong_closed = Contract(
    known="ok",
    fixed_day=datetime.date(2026, 8, 22),
    day_choice=datetime.date(2026, 8, 23),
    fixed_blob=b"b",
)
reported = violations(lambda: converter(Contract).to_transfer_type(wrong_closed))
assert reported == [
    ("fixedDay", 'must equal "2026-08-21"'),
    ("dayChoice", 'must be one of ["2026-08-21", "2026-08-22"], got "2026-08-23"'),
    ("fixedBlob", r'must match pattern ^YQ==\Z, got "Yg=="'),
    ("fixedBlob", 'must equal "YQ=="'),
], reported
"#;

/// A property-position union whose **last** branch converts through a model's
/// converter. Nothing in the annotation stops a member holding a value no branch
/// admits, and the serialize dispatch guards every branch but the last, so that
/// value reaches the last branch's converter — which fails on whatever attribute
/// it reads first. `mixed`'s last branch is a scalar, so its fallthrough returns
/// the bad value instead: same missing check, quieter symptom.
const UNION_DISPATCH_FALLTHROUGH_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
properties:
  pick:
    oneOf:
      - { $ref: "#/$defs/Circle" }
      - { $ref: "#/$defs/Square" }
  mixed:
    oneOf:
      - { $ref: "#/$defs/Circle" }
      - { type: string, minLength: 2 }
$defs:
  Circle:
    type: object
    required: [kind, radius]
    properties:
      kind: { type: string, const: circle }
      radius: { type: number }
  Square:
    type: object
    required: [kind, side]
    properties:
      kind: { type: string, const: square }
      side: { type: number }
"##;

/// Drives the generated converter for `UNION_DISPATCH_FALLTHROUGH_SCHEMA`: a
/// member matching no branch must raise the union's own aggregated
/// `ValidationError`, at the member's path, alongside every other violation the
/// model collected — not the `AttributeError` the fallthrough branch's converter
/// used to raise, which escaped the `except ValidationError` and discarded them.
const UNION_DISPATCH_FALLTHROUGH_RUNTIME_CHECK: &str = r#"
import sys

root, package = sys.argv[1], sys.argv[2]
sys.path.insert(0, root)
models = __import__(package + ".models", fromlist=["*"])
definitions = __import__(package + "._definitions", fromlist=["*"])

Bag, Circle = models.Bag, models.Circle
ValidationError = definitions.ValidationError
converter = getattr(Bag, "__temporal_transfer_type_converter")

valid = {"pick": {"kind": "circle", "radius": 1.5}, "mixed": "ok"}
model = converter.from_transfer_type(valid, Bag)
assert converter.to_transfer_type(model) == valid, converter.to_transfer_type(model)

# Neither member is admitted by any branch: `pick` used to reach
# `_SquareTransferTypeConverter` and raise `AttributeError: 'int' object has no
# attribute 'kind'`, taking `mixed`'s violation down with it.
try:
    converter.to_transfer_type(Bag(pick=42, mixed=7, additional_properties={}))
except ValidationError as error:
    reported = [(violation.path, violation.reason) for violation in error.violations]
else:
    raise AssertionError("serializing members no branch admits did not raise")

assert reported == [
    ("pick", "expected one of: Circle, Square"),
    ("mixed", "expected one of: Circle, string"),
], reported

# A branch's own constraint is still reported under the member's path, not the
# empty path the union function collects it at.
try:
    converter.to_transfer_type(Bag(pick=Circle(kind="circle", radius=1.5), mixed="x", additional_properties={}))
except ValidationError as error:
    reported = [(violation.path, violation.reason) for violation in error.violations]
else:
    raise AssertionError("a short string branch did not raise")

assert reported == [("mixed", "must have length >= 2, got 1")], reported
"#;

/// Properties named after the converter body's *own* identifiers — its locals
/// (`violations`, `raw`, `out`), the builtins it calls (`len`, `int`, `str`,
/// `bool`, `dict`, `isinstance`), the modules it imports (`typing`, `math`, `re`),
/// the loop temporaries it uses (`key`, `value`) and the converter method's
/// parameters (`self`, `type_hint`).
///
/// A property may be named anything, so none of these is reserved. No sample
/// schema declares one, which is why this lives here rather than in the Python
/// sample suite: the shadow it used to cause was *silently* wrong (the collected
/// violations were thrown away and an invalid payload came back as a model), so
/// nothing short of running the generated converter proves it is gone.
///
/// The object is open so the catch-all's `_<MODEL>_DECLARED` frozenset — another
/// name synthesized from these properties — is exercised too.
const SHADOWED_NAME_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: true
required: [violations]
properties:
  violations: { type: string, minLength: 2 }
  raw: { type: string }
  out: { type: string }
  len: { type: integer }
  int: { type: integer }
  str: { type: string }
  bool: { type: boolean }
  dict: { type: object, additionalProperties: true }
  isinstance: { type: string }
  typing: { type: string }
  math: { type: number }
  re: { type: string, pattern: "^[a-z]+$" }
  key: { type: string }
  value: { type: string }
  self: { type: string }
  typeHint: { type: string }
"#;

/// Drives the generated converter for `SHADOWED_NAME_SCHEMA` end to end: a valid
/// payload must round-trip, and an invalid one must raise the aggregated
/// `ValidationError` with every violation intact in **both** directions.
const SHADOWED_NAME_RUNTIME_CHECK: &str = r#"
import sys

root, package = sys.argv[1], sys.argv[2]
sys.path.insert(0, root)
models = __import__(package + ".models", fromlist=["*"])
definitions = __import__(package + "._definitions", fromlist=["*"])

Shadow = models.Shadow
converter = getattr(Shadow, "__temporal_transfer_type_converter")

valid = {
    "violations": "ok",
    "raw": "r",
    "out": "o",
    "len": 1,
    "int": 2,
    "str": "s",
    "bool": True,
    "dict": {"a": 1},
    "isinstance": "i",
    "typing": "t",
    "math": 1.5,
    "re": "abc",
    "key": "k",
    "value": "v",
    "self": "me",
    "typeHint": "h",
    "unknown": [1, 2],
}

# Every one of `raw`, `len`, `int`, `str`, `bool`, `dict`, `isinstance`, `typing`,
# `math` and `out` used to crash *every* payload, valid ones included.
model = converter.from_transfer_type(valid, Shadow)
assert model.violations == "ok", model.violations
assert model.type_hint == "h", model.type_hint
assert model.additional_properties == {"unknown": [1, 2]}, model.additional_properties
assert converter.to_transfer_type(model) == valid, converter.to_transfer_type(model)

expected = [
    ("violations", "must have length >= 2, got 1"),
    ("math", "must be a finite number, got inf"),
    ("re", 'must match pattern ^[a-z]+\\Z, got "ABC"'),
]

# The critical case: a property named `violations` rebound the violation
# accumulator, so the collected violations were discarded and the invalid payload
# came back as a model. Reaching the `else` here is that silent failure.
bad = dict(valid, violations="a", re="ABC", math=float("inf"))
try:
    converter.from_transfer_type(bad, Shadow)
except definitions.ValidationError as error:
    got = [(item.path, item.reason) for item in error.violations]
    assert got == expected, got
else:
    raise AssertionError("an invalid payload was accepted: validation was disabled")

# The serialize body has locals of its own, so it needs the same proof (P12). A
# dataclass validates nothing on assignment, so the model is simply mutated.
model.violations = "a"
model.re = "ABC"
model.math = float("inf")
try:
    converter.to_transfer_type(model)
except definitions.ValidationError as error:
    got = [(item.path, item.reason) for item in error.violations]
    assert got == expected, got
else:
    raise AssertionError("an invalid model was serialized: validation was disabled")
"#;

const PYTHON_DATACLASS_DEFAULT_SCHEMA: &str = r#"$schema: https://json-schema.org/draft/2020-12/schema
type: object
required: [requiredPlain, requiredNullable]
properties:
  requiredPlain: { type: string }
  requiredNullable:
    oneOf: [{ type: integer }, { type: "null" }]
  optionalPlain: { type: boolean }
  optionalNullable:
    oneOf: [{ type: string }, { type: "null" }]
  nullableItems:
    type: array
    items:
      oneOf: [{ type: string }, { type: "null" }]
  greeting:
    type: string
    default: hello
    deprecated: true
    x-py-name: salutation
"#;

const PYTHON_DATACLASS_DEFAULT_RUNTIME_CHECK: &str = r#"
import sys

root, package = sys.argv[1], sys.argv[2]
sys.path.insert(0, root)
models = __import__(package + ".models", fromlist=["*"])

Model = models.Model
converter = getattr(Model, "__temporal_transfer_type_converter")

try:
    Model(required_plain="x")
except TypeError:
    pass
else:
    raise AssertionError("required nullable constructor argument became optional")

unset = Model(required_plain="x", required_nullable=None)
other = Model(required_plain="x", required_nullable=None)
assert unset.salutation == "hello"
assert converter.to_transfer_type(unset) == {
    "requiredPlain": "x",
    "requiredNullable": None,
}
assert unset.additional_properties == {}
assert other.additional_properties == {}
assert unset.additional_properties is not other.additional_properties
assert unset == other
assert "_salutation" not in repr(unset)

explicit_default = Model(
    required_plain="x", required_nullable=None, salutation="hello"
)
assert converter.to_transfer_type(explicit_default)["greeting"] == "hello"
assert explicit_default != unset

unset.salutation = "bye"
assert unset.salutation == "bye"
assert converter.to_transfer_type(unset)["greeting"] == "bye"
del unset.salutation
assert unset.salutation == "hello"
assert "greeting" not in converter.to_transfer_type(unset)
assert unset == other
"#;

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn descriptor_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/descriptors/temporal_api.bin")
}

fn linked_inputs_path(root: &Path) -> PathBuf {
    root.join("advanced/samples/inputs/deps")
}

fn example_input_paths(root: &Path, example_id: &str) -> Vec<PathBuf> {
    let input = input_path(root, example_id);
    let mut paths = vec![input.clone()];
    if fs::read_to_string(&input)
        .unwrap()
        .contains("use nexus:temporal-types/")
    {
        paths.push(linked_inputs_path(root));
    }
    paths
}

fn python_root(root: &Path) -> PathBuf {
    root.join("advanced/samples/python")
}

fn samples_python_root(root: &Path) -> PathBuf {
    root.join("samples/python")
}

fn input_path(root: &Path, example_id: &str) -> PathBuf {
    let flat_path = root
        .join("advanced/samples/inputs")
        .join(format!("{example_id}.wit"));
    if flat_path.is_file() {
        flat_path
    } else {
        root.join("advanced/samples/inputs")
            .join(example_id)
            .join("main.wit")
    }
}

fn python_output_path(root: &Path, example_id: &str) -> PathBuf {
    python_root(root)
        .join("wit")
        .join(example_id.to_snake_case())
}

fn python_json_definitions_output_path(root: &Path, example_id: &str) -> PathBuf {
    // Definitions are the beginner-facing samples, flattened under samples/python/<snake>.
    samples_python_root(root).join(example_id.to_snake_case())
}

fn python_json_api_output_path(root: &Path, example_id: &str) -> PathBuf {
    python_root(root)
        .join("json_schema")
        .join("api")
        .join(example_id.to_snake_case())
}

fn python_example_ids(root: &Path) -> Vec<String> {
    let mut ids = fs::read_dir(root.join("advanced/samples/inputs"))
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            let example_id = if path.is_file() {
                path.file_stem()?.to_string_lossy().into_owned()
            } else if path.join("main.wit").is_file() {
                path.file_name()?.to_string_lossy().into_owned()
            } else {
                return None;
            };
            if python_output_path(root, &example_id).is_dir() {
                Some(example_id)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn read_python_package_files(dir: &Path) -> BTreeMap<PathBuf, String> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(root, &path, files);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("py") {
                if path
                    .file_name()
                    .and_then(|file_name| file_name.to_str())
                    .is_some_and(|file_name| file_name.starts_with("test_"))
                {
                    continue;
                }
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

fn assert_python_validation_exports(package_init: &str) {
    for expected in [
        "from ._definitions import (",
        "    ValidationError,",
        "    Violation,",
        "    \"ValidationError\",",
        "    \"Violation\",",
    ] {
        assert!(
            package_init.contains(expected),
            "{expected}\n{package_init}"
        );
    }
}

fn render_output_files(files: BTreeMap<PathBuf, String>) -> String {
    files
        .into_iter()
        .map(|(path, contents)| format!("### {}\n{contents}", path.display()))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn generate_python_to_string(input_paths: &[PathBuf], descriptor_paths: &[PathBuf]) -> String {
    let temp_dir = unique_output_path("python-rendered");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: input_paths.to_vec(),
        support_paths: Vec::new(),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: true,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = if output_path.is_file() {
        fs::read_to_string(&output_path).unwrap()
    } else {
        render_output_files(read_python_package_files(&output_path))
    };
    fs::remove_dir_all(temp_dir).unwrap();
    rendered
}

fn generate_python_package_files(
    input_paths: &[PathBuf],
    descriptor_paths: &[PathBuf],
) -> BTreeMap<PathBuf, String> {
    let temp_dir = unique_output_path("python-package");
    let output_path = temp_dir.join("output");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: input_paths.to_vec(),
        support_paths: Vec::new(),
        descriptor_paths: descriptor_paths.to_vec(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: true,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let files = read_python_package_files(&output_path);
    fs::remove_dir_all(temp_dir).unwrap();
    files
}

fn generate_formatted_python_output(root: &Path, example_id: &str, output_path: &Path) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nexgen"));
    command
        .arg("python")
        .args(example_input_paths(root, example_id))
        .args([
            "--descriptors",
            descriptor_path(root).to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--native-api",
        ]);
    let status = command.status().unwrap();
    assert!(status.success());

    let format_status = Command::new("uv")
        .current_dir(python_root(root))
        .args([
            "run",
            "ruff",
            "format",
            "--line-length",
            "88",
            "--config",
            "pyproject.toml",
            output_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(format_status.success());
}

fn generate_formatted_json_python_output(
    root: &Path,
    example_id: &str,
    output_path: &Path,
    generate_native_api: bool,
) {
    let input_path = json_input_path(root, example_id);
    let mut args = vec![
        "python",
        input_path.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ];
    if generate_native_api {
        args.push("--native-api");
    }

    let status = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());

    let format_status = Command::new("uv")
        .current_dir(python_root(root))
        .args([
            "run",
            "ruff",
            "format",
            "--line-length",
            "88",
            "--config",
            "pyproject.toml",
            output_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(format_status.success());
}

fn assert_python_310_syntax_compatible(package_dir: &Path) {
    let checker = r#"
import ast
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
for path in sorted(root.rglob("*.py")):
    source = path.read_text()
    try:
        ast.parse(source, filename=str(path), feature_version=(3, 10))
    except SyntaxError as exc:
        print(f"{path}: {exc}")
        raise
"#;
    let status = Command::new(
        project_root()
            .join("advanced/samples/python/.venv/bin/python")
            .to_str()
            .unwrap(),
    )
    .args(["-c", checker, package_dir.to_str().unwrap()])
    .status()
    .unwrap();
    assert!(status.success());
}

/// The interpreter the generated packages are exercised with. The advanced
/// project's environment is the one already provisioned with `temporalio`, which
/// the generated converters import.
fn sample_python_interpreter() -> PathBuf {
    project_root().join("advanced/samples/python/.venv/bin/python")
}

/// Runs `script` under that interpreter, failing the test on a non-zero exit. Used
/// where a rendered-output assertion cannot reach the behavior under test — a
/// silently disabled validator renders perfectly readable code.
fn assert_python_script_succeeds(script: &str, args: &[&str]) {
    let status = Command::new(sample_python_interpreter())
        .args(["-c", script])
        .args(args)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "generated package failed its runtime check"
    );
}

fn unique_output_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = OUTPUT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nexgen-{label}-{unique}-{counter}"))
}

#[test]
fn python_examples_generation_matches_checked_in_output() {
    let root = project_root();
    for example_id in python_example_ids(&root) {
        let output_path = unique_output_path(&format!("python-{example_id}"));
        generate_formatted_python_output(&root, &example_id, &output_path);
        assert_python_310_syntax_compatible(&output_path);
        let rendered = read_python_package_files(&output_path);
        let expected = read_python_package_files(&python_output_path(&root, &example_id));
        assert_eq!(rendered, expected, "snapshot mismatch for {example_id}");
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn python_json_example_generation_matches_checked_in_output() {
    let root = project_root();
    for example_id in ["chat", "kb", "showcase", "temporal"] {
        let output_path = unique_output_path(&format!("python-json-{example_id}"));
        generate_formatted_json_python_output(&root, example_id, &output_path, false);
        assert_python_310_syntax_compatible(&output_path);
        let rendered = read_python_package_files(&output_path);
        let expected =
            read_python_package_files(&python_json_definitions_output_path(&root, example_id));
        assert_eq!(rendered, expected, "snapshot mismatch for {example_id}");
        let package_init = rendered
            .get(&PathBuf::from("__init__.py"))
            .expect("JSON Schema package should include a root __init__.py");
        assert_python_validation_exports(package_init);
        if example_id == "showcase" {
            let all = rendered.values().cloned().collect::<Vec<_>>().join("\n");
            // A default-bearing property materializes on read while its private
            // optional storage retains unset state for wire omission.
            assert!(all.contains("_greeting: str | None"));
            assert!(all.contains("def greeting(self) -> str:"));
            assert!(all.contains("def greeting(self, value: str) -> None:"));
            assert!(all.contains("@greeting.deleter\n    def greeting(self) -> None:"));
            assert!(!all.contains("DEFAULT_GREETING"));
            assert!(!all.contains("DEFAULT_DEBUG"));
            assert!(!all.contains("DEFAULT_RETRIES"));
            // `deprecated` → PEP 702 marker (no runtime warning); `title` → docstring.
            assert!(all.contains(
                "typing_extensions.deprecated(\"This field is deprecated.\", category=None)"
            ));
            assert!(all.contains("Retry budget"));
            // `x-py-name` override (Stage 4): the attribute uses the override
            // while the wire name stays `legacyId`, pinned by the converter body.
            assert!(all.contains("legacy_id_py:"));
            assert!(all.contains("\"legacyId\""));
            // A free-form object inlines as a mapping as a union branch, and as a
            // named model with an explicit `additional_properties` catch-all.
            assert!(all.contains("payload: dict[str, typing.Any] | str | None"));
            assert!(all.contains("class Extras:"));
            assert!(
                all.contains("additional_properties: dict[str, typing.Any] = dataclasses.field(")
            );
            // Each model is a plain dataclass carrying a private transfer type
            // converter, so the default Temporal data converter picks it up. The
            // registration goes through the runtime's `_transfer_type_convertible`
            // shim, which erases the converter's value-type parameter — binding it
            // on the decorated class is circular for a static type checker.
            assert!(all.contains("@dataclasses.dataclass(slots=True, kw_only=True)"));
            assert!(all.contains("@_transfer_type_convertible(_ExtrasTransferTypeConverter)"));
            assert!(all.contains(
                "def _transfer_type_convertible(\n    converter: type[temporalio.converter.TransferTypeConverter[typing.Any, typing.Any]],\n) -> collections.abc.Callable[[type[_ModelT]], type[_ModelT]]:"
            ));
            assert!(
                all.contains(
                    "    return temporalio.converter.transfer_type_convertible(converter)"
                )
            );
            // A tagged union whose branches are written inline: each branch names
            // itself with `x-py-name` and becomes a model of its own.
            assert!(all.contains("class TextNote:"));
            assert!(all.contains("Note: typing.TypeAlias = TextNote | LinkNote"));
            // A named union cannot be decorated, so its conversion is emitted as
            // module-private free functions instead.
            assert!(all.contains("def _note_from_transfer_type("));
            assert!(all.contains("def _note_to_transfer_type("));
            // The lone inline object branch of a property union derives its name
            // from the union it belongs to.
            assert!(all.contains("class ShowcaseDetailObject:"));
            assert!(all.contains("detail: ShowcaseDetailObject | str | None"));
            assert!(all.contains("must have at most 4 properties"));
        }
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn python_json_api_example_generation_matches_checked_in_output() {
    let root = project_root();
    for example_id in ["chat", "kb", "showcase", "temporal"] {
        let output_path = unique_output_path(&format!("python-json-api-{example_id}"));
        generate_formatted_json_python_output(&root, example_id, &output_path, true);
        assert_python_310_syntax_compatible(&output_path);
        let rendered = read_python_package_files(&output_path);
        let expected = read_python_package_files(&python_json_api_output_path(&root, example_id));
        assert_eq!(rendered, expected, "snapshot mismatch for {example_id}");
        let package_init = rendered
            .get(&PathBuf::from("__init__.py"))
            .expect("JSON Schema package should include a root __init__.py");
        assert_python_validation_exports(package_init);
        fs::remove_dir_all(output_path).unwrap();
    }
}

#[test]
fn cli_generates_wit_direct_example_without_descriptors() {
    let root = project_root();
    let output_path = unique_output_path("python-user-service-no-descriptors");
    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            input_path(&root, "user-service").to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_path.join("__init__.py").is_file());
    assert!(output_path.join("models.py").is_file());
    fs::remove_dir_all(output_path).unwrap();
}

#[test]
fn cli_defaults_to_definitions_without_native_api_or_endpoint() {
    let temp_dir = unique_output_path("python-definitions-only");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("input.wit");
    let output_path = temp_dir.join("output");
    fs::write(
        &input_path,
        r#"
package temporal:example@1.0.0;

world system {
  export example-service;
}

interface example-service {
  record request {
    name: string,
  }

  record response {
    message: string,
  }

  example-operation: func(request: request) -> response;
}
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            input_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_path.join("__init__.py").is_file());
    assert!(output_path.join("models.py").is_file());
    assert!(output_path.join("services.py").is_file());
    assert!(!output_path.join("operations/example_operation.py").exists());

    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn cli_generates_python_support_file_from_parameter() {
    let root = project_root();
    let temp_dir = unique_output_path("python-support-file-input");
    fs::create_dir_all(&temp_dir).unwrap();
    let support_path = temp_dir.join("custom_support.py");
    let output_path = temp_dir.join("output");
    fs::write(
        &support_path,
        "def custom_support_hook() -> str:\n    return 'custom'\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nexgen"))
        .args([
            "python",
            input_path(&root, "user-service").to_str().unwrap(),
            "--support-file",
            support_path.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(output_path.join("_support/custom_support.py")).unwrap(),
        "def custom_support_hook() -> str:\n    return 'custom'\n"
    );
    assert!(
        fs::read_to_string(output_path.join("_support/__init__.py"))
            .unwrap()
            .contains("from .custom_support import *")
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_example_suite_type_checks_and_runs() {
    let root = project_root();

    // The advanced project holds the WIT + proto-wire suites and the snapshot-only
    // native-api outputs; the samples project holds the JSON-Schema definitions and
    // their round-trip tests. Both must type-check and pass.
    for example_dir in [python_root(&root), samples_python_root(&root)] {
        let typecheck_status = Command::new("uv")
            .current_dir(&example_dir)
            .args(["run", "basedpyright"])
            .status()
            .unwrap();
        assert!(
            typecheck_status.success(),
            "basedpyright failed in {example_dir:?}"
        );

        let pytest_status = Command::new("uv")
            .current_dir(&example_dir)
            .args(["run", "pytest"])
            .status()
            .unwrap();
        assert!(pytest_status.success(), "pytest failed in {example_dir:?}");
    }
}

/// The generated JSON-Schema runtime must also *run* on the declared floor,
/// `requires-python = ">=3.10"` — not merely parse as 3.10 syntax.
///
/// `assert_python_310_syntax_compatible` checks the AST at
/// `feature_version=(3, 10)`, which is a syntax check only, and the project
/// environments above are whatever interpreter `uv` picked (3.13 here). That left a
/// real class of bug uncovered: before 3.11, `datetime.fromisoformat` parses only
/// the fractional-second widths `isoformat` writes, so an RFC 3339 `.1` raised on
/// 3.10 while passing everywhere else. Every test in the suite was green.
///
/// The environment lives outside the project directory so the checked-in one is
/// untouched and neither `basedpyright` nor `ruff` picks it up (their excludes name
/// `.venv`). It is created on demand in well under a second from the same locked
/// `uv.lock`, so this is one extra resolve, not a second maintained lockfile; `uv`
/// fetches a managed CPython 3.10 if the host has none.
#[test]
fn python_json_samples_run_on_the_declared_python_floor() {
    let root = project_root();
    let floor_environment = root.join("target/python-floor-venv");

    let status = Command::new("uv")
        .current_dir(samples_python_root(&root))
        .env("UV_PROJECT_ENVIRONMENT", &floor_environment)
        .args(["run", "--python", "3.10", "--locked", "pytest"])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "the JSON-Schema sample suite failed on Python 3.10, the declared floor"
    );
}

#[test]
fn python_request_models_are_bidirectional_wire_models() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("Python package should include models.py");
    let rendered = generate_python_to_string(
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );

    assert!(!rendered.contains("SignalWithStartWorkflowRequest.from_proto"));
    assert!(!rendered.contains(
        "proto: temporalio.api.workflowservice.v1.SignalWithStartWorkflowExecutionRequest,\n    ) -> SignalWithStartWorkflowRequest:"
    ));
    assert!(rendered.contains("class SignalWithStartWorkflowRequest:"));
    assert!(rendered.contains(
        "@dataclasses.dataclass(slots=True, kw_only=True)\nclass SignalWithStartWorkflowRequest:\n    \"\"\"\n    .. warning::\n        This API is experimental and subject to change.\n    \"\"\"\n    workflow: str | collections.abc.Callable[..., collections.abc.Awaitable[object]]\n    args: list[typing.Any] | None = None\n    id: str\n    task_queue: str\n    signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]]\n    signal_args: list[typing.Any] | None = None\n    execution_timeout: datetime.timedelta | None = None"
    ));
    assert!(rendered.contains("temporalio.common.WorkflowIDReusePolicy.ALLOW_DUPLICATE"));
    assert!(rendered.contains("args: list[typing.Any] | None = None"));
    assert!(rendered.contains("signal_args: list[typing.Any] | None = None"));
    assert!(!rendered.contains("args: tuple[typing.Any, ...] | None = None"));
    assert!(!rendered.contains("signal_args: tuple[typing.Any, ...] | None = None"));
    assert!(!rendered.contains("(typing.TypedDict, total=False):"));
    assert!(!rendered.contains("typing.Unpack["));
    assert!(!rendered.contains("namespace: str | None = None"));
    assert!(!rendered.contains("namespace: str | None"));
    assert!(rendered.contains("namespace: str = ("));
    assert!(rendered.contains("dataclasses.field(default_factory=workflow_namespace)"));
    assert!(rendered.contains("message.namespace = value.namespace"));
    assert!(rendered.contains("result = await handle"));
    assert!(rendered.contains(
        "from temporalio.workflow import (\n        create_nexus_client,\n        get_external_workflow_handle,\n    )"
    ));
    assert!(rendered.contains("return get_external_workflow_handle("));
    assert!(rendered.contains("run_id=result.run_id"));
    assert!(rendered.contains("async def _signal_with_start_workflow("));
    assert!(rendered.contains("request: SignalWithStartWorkflowRequest"));
    assert!(rendered.contains(
        "if typing.TYPE_CHECKING:\n    from temporalio.workflow import ExternalWorkflowHandle"
    ));
    assert!(rendered.contains(") -> ExternalWorkflowHandle[object]:"));
    assert!(rendered.contains("    nexus_client = create_nexus_client("));
    assert!(rendered.contains("async def signal_with_start_workflow("));
    assert!(rendered.contains("@typing.overload"));
    assert!(rendered.contains("workflow: str,"));
    assert!(rendered.contains("*args: object,"));
    assert!(rendered.contains("id: str,"));
    assert!(rendered.contains("args: list[typing.Any] | None = ...,"));
    assert!(rendered.contains(
        "workflow: collections.abc.Callable[[SelfType, typing_extensions.Unpack[WorkflowArgs]], collections.abc.Awaitable[WorkflowResult]],"
    ));
    assert!(rendered.contains("WorkflowArgs = typing_extensions.TypeVarTuple(\"WorkflowArgs\")"));
    assert!(rendered.contains("WorkflowResult = typing.TypeVar(\"WorkflowResult\")"));
    assert!(rendered.contains("SelfType = typing.TypeVar(\"SelfType\")"));
    assert!(rendered.contains(") -> ExternalWorkflowHandle[SelfType]:"));
    assert!(rendered.contains("*args: typing_extensions.Unpack[WorkflowArgs],"));
    assert!(rendered.contains("args: list[typing.Any],"));
    assert!(!rendered.contains("tuple[FirstWorkflowArg"));
    assert!(rendered.contains(
        "signal: collections.abc.Callable[[SelfType, SignalArg], None | collections.abc.Awaitable[None]],"
    ));
    assert!(rendered.contains("SignalArg = typing.TypeVar(\"SignalArg\")"));
    assert!(rendered.contains("signal_args: SignalArg,"));
    assert!(rendered.contains("signal_args: list[typing.Any],"));
    assert!(rendered.contains(
        "async def signal_with_start_workflow(\n    workflow: str | collections.abc.Callable[..., collections.abc.Awaitable[object]],\n    *positional_args: object,\n    args: list[typing.Any] | None = None,\n    id: str,\n    task_queue: str,\n    signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]],\n    signal_args: object | list[typing.Any] | None = None,"
    ));
    assert!(rendered.contains(
        "signal: str | collections.abc.Callable[..., None | collections.abc.Awaitable[None]],"
    ));
    assert!(rendered.contains("args: list[typing.Any] | None = None,"));
    assert_eq!(
        rendered
            .matches("Signal a workflow, starting it first if needed.")
            .count(),
        1
    );
    assert!(rendered.contains(
        "\"\"\"Signal a workflow, starting it first if needed.\n\n    .. warning::\n        This API is experimental and subject to change.\n\n    Args:\n        workflow: Workflow type name or callable identifying the workflow to start.\n        positional_args: Positional arguments for workflow. Cannot be set if args is\n            set.\n        args: List-form arguments for workflow. Cannot be set if positional_args are\n            set. For typed workflow callables, list contents are not statically\n            typechecked; pass workflow arguments positionally for precise typechecking.\n        id: Unique identifier for the workflow execution.\n        task_queue: Task queue to run the workflow on.\n        signal: Signal name or callable to send with the start request.\n        signal_args: Argument value, or list of argument values, for signal. For typed\n            single-argument signals, scalar signal_args values are statically\n            typechecked. List-form signal_args values are not precisely typechecked. To\n            pass a single signal argument that is itself a list, wrap it in another\n            list; otherwise the list is interpreted as multiple signal arguments."
    ));
    assert!(rendered.contains(
        "cron_schedule: Cron schedule for recurring workflow executions. See\n            https://docs.temporal.io/cron-job."
    ));
    assert!(rendered.contains(
        "static_summary: Single-line fixed summary for the workflow execution that may\n            appear in UI and CLI. This can be in single-line Temporal Markdown format."
    ));
    assert!(rendered.contains(
        "\n\n    Returns:\n        A workflow handle to the started workflow.\n    \"\"\""
    ));
    assert!(rendered.contains(
        "id_reuse_policy: temporalio.common.WorkflowIDReusePolicy = (\n        temporalio.common.WorkflowIDReusePolicy.ALLOW_DUPLICATE\n    ),"
    ));
    assert!(
        rendered.contains(
            "id_conflict_policy: temporalio.common.WorkflowIDConflictPolicy | None = None,"
        )
    );
    assert!(rendered.contains("static_summary: str | None = None,"));
    assert!(rendered.contains("static_details: str | None = None,"));
    assert!(!rendered.contains("identity: str | None"));
    assert!(!rendered.contains("user_metadata_static_summary:"));
    assert!(!rendered.contains("user_metadata_static_details:"));
    assert!(rendered.contains("request = SignalWithStartWorkflowRequest("));
    assert!(rendered.contains("workflow=workflow,"));
    assert!(!rendered.contains("def _nexus_is_function_args_list("));
    assert!(!rendered.contains("def _nexus_normalize_function_args("));
    assert!(!rendered.contains("_nexus_arg_unset = object()"));
    assert!(rendered.contains("if positional_args and args is not None:"));
    assert!(
        rendered.contains("raise TypeError(\"cannot specify both positional arguments and args\")")
    );
    assert!(rendered.contains("normalized_args: list[typing.Any] | None = ("));
    assert!(rendered.contains("list(positional_args)"));
    assert!(rendered.contains("else args"));
    assert!(rendered.contains(
        "normalized_signal_args: list[typing.Any] | None\n    if signal_args is None:\n        normalized_signal_args = None\n    elif isinstance(signal_args, list):\n        normalized_signal_args = typing.cast(list[typing.Any], signal_args)\n    else:\n        normalized_signal_args = [signal_args]"
    ));
    assert!(rendered.contains("user_metadata = ("));
    assert!(rendered.contains("if static_summary is None and static_details is None"));
    assert!(rendered.contains("static_summary=static_summary,"));
    assert!(rendered.contains("static_details=static_details,"));
    assert!(rendered.contains("args=normalized_args,"));
    assert!(rendered.contains("id=id,"));
    assert!(rendered.contains("signal_args=normalized_signal_args,"));
    assert!(rendered.contains("user_metadata=user_metadata,"));
    assert!(rendered.contains("return await _signal_with_start_workflow(request)"));
    assert!(models.contains("payloads_to_proto(value.args)"));
    assert!(models.contains("def from_transfer_type("));
    assert!(models.contains("@temporalio.converter.transfer_type_convertible("));
    assert!(!models.contains("def from_proto("));
    assert!(models.contains("from ._support import ("));
    assert!(models.contains("retry_policy_to_proto,"));
    assert!(
        rendered.contains("from ._support import signal_with_start_workflow_serialization_context")
    );
    assert!(rendered.contains(
        "): _NexusOperationInfo(\n        operation=_services.WorkflowService.signal_with_start_workflow,\n        serialization_context=signal_with_start_workflow_serialization_context,\n    ),"
    ));
    assert!(rendered.contains(
        "handle = await nexus_client.start_operation(\n        operation=\"SignalWithStartWorkflowExecution\",\n        input=request,\n        output_type=SignalWithStartWorkflowResponse,\n    )"
    ));
    assert!(rendered.contains(
        "class SignalWithStartWorkflowModelRequest(typing.Protocol):\n    namespace: str\n    id: str"
    ));
    assert!(rendered.contains(
        "def signal_with_start_workflow_serialization_context(\n    request: SignalWithStartWorkflowModelRequest,\n) -> temporalio_converter.WorkflowSerializationContext:"
    ));
    assert!(rendered.contains(
        "return temporalio_converter.WorkflowSerializationContext(\n        namespace=request.namespace,\n        workflow_id=request.id,\n    )"
    ));

    let type_roundtrip_rendered = generate_python_to_string(
        &example_input_paths(&root, TYPE_ROUNDTRIP_EXAMPLE_ID),
        &[descriptor_path(&root)],
    );
    assert!(type_roundtrip_rendered.contains("async def activity_options_operation("));
    assert!(type_roundtrip_rendered.contains("task_queue: str | None = None,"));
    assert!(type_roundtrip_rendered.contains("retry_policy: temporalio.common.RetryPolicy,"));
    assert!(type_roundtrip_rendered.contains("request = ActivityOptions("));
}

#[test]
fn python_standalone_proto_oneof_models_are_exported_and_converted() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, "proto-oneof"),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("standalone Python package should include models.py");
    let package_init = package
        .get(&PathBuf::from("__init__.py"))
        .expect("standalone Python package should include __init__.py");

    assert!(models.contains("class Outcome(typing.Generic[OutputT]):"));
    assert!(models.contains("\"Outcome[typing.Any]\""));
    assert!(models.contains("OutcomeValue = ("));
    assert!(models.contains("    value: OutcomeValue[OutputT]\n"));
    assert!(!models.contains("value: OutcomeValue[OutputT] | None"));
    assert!(!models.contains("class Failure:"));
    assert!(!models.contains("class Payloads:"));
    assert!(models.contains("_oneof_value_case = value.WhichOneof(\"value\")"));
    assert!(models.contains(
        "if _oneof_value_case is None:\n            raise ValueError(\"missing required field Outcome.value\")"
    ));
    assert!(models.contains(
        "_oneof_value = (\"success\", payloads_from_proto(value.success, [output_type])[0])"
    ));
    assert!(models.contains("_oneof_value = (\"failure\", failure_from_proto(value.failure))"));
    assert!(models.contains("if value.value[0] == \"success\":"));
    assert!(models.contains(
        "if value.value is None:\n            raise ValueError(\"missing required field Outcome.value\")"
    ));
    assert!(models.contains("message.success.CopyFrom(payloads_to_proto([value.value[1]]))"));
    assert!(models.contains("elif value.value[0] == \"failure\":"));
    assert!(models.contains("message.failure.CopyFrom(failure_to_proto(value.value[1]))"));
    assert!(models.contains("raise ValueError(f\"unknown protobuf oneof tag Outcome.value:"));
    assert!(models.contains("class PauseActivityRequest:"));
    assert!(models.contains("namespace: str"));
    assert!(models.contains("execution: WorkflowExecution | None = None"));
    assert!(models.contains("identity: str"));
    assert!(models.contains("activity: ActivitySelection | None = None"));
    assert!(models.contains("reason: str"));
    assert!(models.contains("request_id: str"));
    assert!(models.contains("class WorkflowExecution:"));
    assert!(models.contains("_oneof_activity_case = value.WhichOneof(\"activity\")"));
    assert!(
        models.contains("if _oneof_activity_case is None:\n            _oneof_activity = None")
    );
    assert!(package_init.contains("ActivitySelection,"));
    assert!(package_init.contains("PauseActivityRequest,"));
}

#[test]
fn python_proto_generics_propagate_payload_type_hints() {
    let root = project_root();
    let package = generate_python_package_files(
        &example_input_paths(&root, "proto-generic-python"),
        &[descriptor_path(&root)],
    );
    let models = package
        .get(&PathBuf::from("models.py"))
        .expect("proto generic models should include models.py");
    let support = package
        .get(&PathBuf::from("_support/temporal_model_converters.py"))
        .expect("proto generic models should include the Temporal converter support module");

    assert!(models.contains("class PayloadBackedEnvelope(typing.Generic[OutputT, ContextT]):"));
    assert!(models.contains("\"PayloadBackedEnvelope[typing.Any, typing.Any]\""));
    assert!(models.contains("\"PayloadBackedOutput[typing.Any]\""));
    assert!(models.contains("\"PayloadBackedContext[typing.Any]\""));
    assert!(models.contains(
        "output_type, context_type = typing.get_args(type_hint) or (typing.Any, typing.Any)"
    ));
    assert!(models.contains(
        "_PayloadBackedOutputTransferTypeConverter().from_transfer_type(value.provider, PayloadBackedOutput[output_type])"
    ));
    assert!(models.contains(
        "_PayloadBackedContextTransferTypeConverter().from_transfer_type(value.scaler, PayloadBackedContext[context_type])"
    ));
    assert!(models.contains("output_type, = typing.get_args(type_hint) or (typing.Any,)"));
    assert!(models.contains("payload_from_proto(value.details, output_type)"));
    assert!(models.contains("context_type, = typing.get_args(type_hint) or (typing.Any,)"));
    assert!(models.contains("payload_from_proto(value.details, context_type)"));
    assert!(support.contains("type_hint: type[typing.Any] | None = None,"));
    assert!(support.contains("converter.from_payload(_clone_payload(proto), type_hint)"));
}

#[test]
fn python_rejects_support_namespace() {
    let root = project_root();
    let spec = nexgen::parser::load_api_spec_from_wit_for_language_with_inputs(
        nexgen::language::Language::Python,
        &example_input_paths(&root, PRIMARY_EXAMPLE_ID),
    )
    .unwrap();
    let descriptors = nexgen::descriptors::DescriptorIndex::load(&descriptor_path(&root)).unwrap();
    let err = generate_source(
        nexgen::language::Language::Python,
        spec.clone(),
        &descriptors,
        &SupportFiles {
            fragments: vec![SupportFragmentSpec {
                path: "support.py".to_string(),
                contents: String::new(),
                namespace: Some("example.support".to_string()),
            }],
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("support namespace"));
}

/// An inline **structured** object `oneOf` branch on a property: the branch is
/// named `<Union>Object` and emitted as a module-level dataclass, which the
/// union's dispatcher selects for the object member of the union.
/// See `specs/json-schema/features/oneOf.md` ("Object branches").
#[test]
fn python_json_names_inline_object_union_branch() {
    let temp_dir = unique_output_path("py-json-inline-branch");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("detail.yaml");
    fs::write(&input_path, INLINE_OBJECT_BRANCH_SCHEMA).unwrap();
    let output_path = temp_dir.join("detail");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    assert!(rendered.contains("payload: DetailPayloadObject | str | None"));
    assert!(rendered.contains("class DetailPayloadObject:"));
    assert!(rendered.contains("class _DetailPayloadObjectTransferTypeConverter("));
    assert!(rendered.contains("text: str"));
    // The branch model is part of the module surface, like any named definition.
    let exports = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    assert!(exports.contains("DetailPayloadObject"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Every constraint a **non-object** branch declares is enforced by the union's
/// dispatcher rather than by the annotation, so the field annotation is the plain
/// union of the branch types while the bound, the `pattern`, and the
/// `uniqueItems` check all live in the converter body.
/// See `specs/json-schema/features/oneOf.md` ("Validator mapping").
#[test]
fn python_json_validates_non_object_union_branch_constraints() {
    let temp_dir = unique_output_path("py-json-branch-constraints");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bc.yaml");
    fs::write(&input_path, BRANCH_CONSTRAINT_SCHEMA).unwrap();
    let output_path = temp_dir.join("bc");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    // The string branch's `minLength`/`pattern` and the integer branch's
    // `minimum` leave no residue on the annotation: it is the plain branch union.
    assert!(rendered.contains("value: str | int | None"));
    // Same for the array branch's `minItems`/`uniqueItems`; a closed value set
    // still narrows to a `typing.Literal`.
    assert!(rendered.contains("list[float] | typing.Literal[\"auto\", \"manual\"] | None"));
    // The branch checks themselves live in the converter body: a `pattern` lowers
    // to a `.search` against a module-level compiled regex const, `uniqueItems` to
    // a runtime helper imported from the definitions module.
    assert!(
        rendered.contains("_PATTERN_F242E3A159C2422C = re.compile(\"^[a-z]+\\\\Z\", re.ASCII)")
    );
    assert!(rendered.contains("if _PATTERN_F242E3A159C2422C.search(value) is None:"));
    assert!(rendered.contains("_check_unique_items("));
    // The array branch's element schema is part of the serialize contract too:
    // its unconstrained `number` still contributes the uniform finiteness guard
    // at the indexed path.
    assert!(
        rendered.contains("for item_index_8, item_element_8 in enumerate(value.list_or_name):")
    );
    assert!(rendered.contains(
        "Violation(path=f'listOrName[{item_index_8}]', reason=f\"must be a finite number, got {item_element_8}\")"
    ));
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_json_enforces_remaining_scalar_and_typed_extra_contracts() {
    let temp_dir = unique_output_path("py-json-conformance");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("contract.yaml");
    fs::write(&input_path, PYTHON_CONFORMANCE_SCHEMA).unwrap();
    let output_path = temp_dir.join("contract_package");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();
    assert!(rendered.contains("additional_properties: dict[str, Extra]"));
    assert!(rendered.contains("fixed_day: datetime.date | None"));
    assert!(rendered.contains("day_choice: datetime.date | None"));
    assert!(rendered.contains("fixed_blob: bytes | None"));
    assert_python_script_succeeds(
        PYTHON_CONFORMANCE_RUNTIME_CHECK,
        &[temp_dir.to_str().unwrap(), "contract_package"],
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_json_services_render_one_sided_io_names_and_deprecation() {
    let temp_dir = unique_output_path("py-json-services");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("contract.nexusrpc.yaml");
    fs::write(
        &input_path,
        r##"nexusrpc: "1.0.0"
services:
  LegacyService:
    deprecated: true
    x-py-name: RenamedService
    operations:
      fetch:
        deprecated: true
        x-py-name: fetch_one
        output: { $ref: "#/$defs/Output" }
      submit:
        input: { $ref: "#/$defs/Input" }
$defs:
  Input:
    type: object
    additionalProperties: false
  Output:
    type: object
    additionalProperties: false
"##,
    )
    .unwrap();
    let output_path = temp_dir.join("contract_package");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: true,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let services = fs::read_to_string(output_path.join("services.py")).unwrap();
    assert!(services.contains("import typing_extensions"), "{services}");
    assert!(services.contains(
        "@typing_extensions.deprecated(\"This service is deprecated.\", category=None)\n@service(name=\"LegacyService\")\nclass RenamedService:"
    ));
    assert!(services.contains("fetch_one: typing.Annotated[Operation["));
    assert!(services.contains(
        "], typing_extensions.deprecated(\"This operation is deprecated.\", category=None)] = Operation(name=\"Fetch\", input_type=type(None), output_type=Output)"
    ));
    assert!(
        services.contains("        None,\n        Output,"),
        "{services}"
    );
    assert!(
        services.contains("        Input,\n        None,"),
        "{services}"
    );
    assert!(!services.contains("\"\"\"\n\n\"\"\""), "{services}");

    assert_python_script_succeeds(
        "import importlib, sys; sys.path.insert(0, sys.argv[1]); importlib.import_module(sys.argv[2] + '.services')",
        &[temp_dir.to_str().unwrap(), "contract_package"],
    );

    fs::remove_dir_all(temp_dir).unwrap();
}

/// A union that converts through a `_<base>_to_transfer_type` runs its checks
/// *inside* that function, ahead of the dispatch, so the unguarded last branch is
/// only ever reached by a value some branch admits. The enclosing member emits no
/// check of its own and re-paths what the function raises.
/// See `specs/json-schema/features/oneOf.md` ("Serialize-side (P12)").
#[test]
fn python_json_union_serializer_validates_before_dispatching() {
    let temp_dir = unique_output_path("py-json-union-dispatch");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bag.yaml");
    fs::write(&input_path, UNION_DISPATCH_FALLTHROUGH_SCHEMA).unwrap();
    let output_path = temp_dir.join("bag_package");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    // The checks precede the dispatch, and the raise separates them: the last
    // branch's converter is unreachable for a value no branch admits.
    let dispatch = rendered
        .split_once("def _bag_pick_to_transfer_type(value: Circle | Square) -> typing.Any:\n")
        .expect("no serialize function for the `pick` union")
        .1;
    assert!(dispatch.starts_with(concat!(
        "    violations: list[Violation] = []\n",
        "    candidate = typing.cast(\"object\", value)\n",
        "    if not (isinstance(candidate, Circle) or isinstance(candidate, Square)):\n",
        "        violations.append(Violation(path=\"\", reason=\"expected one of: Circle, Square\"))\n",
        "    if violations:\n",
        "        raise ValidationError(violations)\n",
        "    if isinstance(value, Circle):\n",
    )),
    "the `pick` dispatch is not preceded by the no-branch-matched test:\n{dispatch}");

    // The enclosing member holds no copy of that test; it only re-paths.
    let member = rendered
        .split_once("        if value.pick is not None:\n")
        .expect("no serialize block for the `pick` member")
        .1;
    assert!(
        member.starts_with(concat!(
            "            try:\n",
            "                out[\"pick\"] = _bag_pick_to_transfer_type(value.pick)\n",
            "            except ValidationError as error:\n",
            "                _collect(violations, \"pick\", error)\n",
        )),
        "the `pick` member repeats the union's checks:\n{member}"
    );

    assert_python_script_succeeds(
        UNION_DISPATCH_FALLTHROUGH_RUNTIME_CHECK,
        &[temp_dir.to_str().unwrap(), "bag_package"],
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A union in an element position: the loader names it, so Python emits an
/// ordinary union alias and the converter dispatches the branch per element. An
/// optional field whose *elements* are nullable still needs its own `| None` —
/// the element's `None` is not the field's.
/// See `specs/json-schema/features/oneOf.md` ("Unions in element positions").
#[test]
fn python_json_annotates_element_position_unions() {
    let temp_dir = unique_output_path("py-json-element-union");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("bag.yaml");
    fs::write(&input_path, ELEMENT_UNION_SCHEMA).unwrap();
    let output_path = temp_dir.join("bag");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    assert!(rendered.contains("BagSegmentsItem: typing.TypeAlias = str | int"));
    assert!(rendered.contains("segments: list[BagSegmentsItem] | None"));
    assert!(rendered.contains("choices: list[Choice] | None"));
    assert!(rendered.contains("slots: list[str | None] | None"));
    let exports = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    assert!(exports.contains("BagSegmentsItem"));
    fs::remove_dir_all(temp_dir).unwrap();
}

/// The entry file of a two-file closure. `get`'s output is the model the *other*
/// file declares, and `FindOutput.page` `$ref`s it from a property, so both
/// cross-module reference shapes are covered.
const CROSS_MODULE_ENTRY_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Pages:
    fqn: example.pages.v1.Pages
    operations:
      get:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "content/page.json" }
      find:
        input: { $ref: "#/$defs/GetInput" }
        output: { $ref: "#/$defs/FindOutput" }
$defs:
  GetInput:
    type: object
    additionalProperties: false
    properties:
      id: { type: string }
  FindOutput:
    type: object
    additionalProperties: false
    properties:
      page: { $ref: "content/page.json" }
"##;

/// The referenced file. Its model carries the name override the *consuming*
/// module has to resolve through.
const CROSS_MODULE_PAGE_SCHEMA: &str = r##"$schema: https://json-schema.org/draft/2020-12/schema
type: object
additionalProperties: false
x-py-name: RenamedPage
properties:
  title: { type: string }
"##;

/// Writes the two-file cross-module closure into `dir` and returns the input
/// directory to generate from.
fn write_cross_module_closure(dir: &Path) -> PathBuf {
    let input_dir = dir.join("input");
    fs::create_dir_all(input_dir.join("content")).unwrap();
    fs::write(
        input_dir.join("kb.nexusrpc.yaml"),
        CROSS_MODULE_ENTRY_SCHEMA,
    )
    .unwrap();
    fs::write(
        input_dir.join("content/page.json"),
        CROSS_MODULE_PAGE_SCHEMA,
    )
    .unwrap();
    input_dir
}

/// An `x-py-name` override on a model in *another* input file moves every
/// reference the consuming module emits: the operation's `Operation[...]`
/// parameter, the relative model imports, and the annotation of a cross-module
/// `$ref` property. The override is declared in the referenced file, so only the
/// tree-wide name manifest can resolve it (P14/P15).
#[test]
fn python_json_cross_module_py_name_override_moves_every_reference() {
    let temp_dir = unique_output_path("py-json-cross-module-override");
    let input_dir = write_cross_module_closure(&temp_dir);
    let output_path = temp_dir.join("output");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_dir],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let declaring = fs::read_to_string(output_path.join("content/page/models.py")).unwrap();
    assert!(declaring.contains("class RenamedPage:"));

    let services = fs::read_to_string(output_path.join("kb/services.py")).unwrap();
    for expected in [
        "from ..content.page.models import RenamedPage",
        "        RenamedPage,\n",
    ] {
        assert!(services.contains(expected), "{expected}\n{services}");
    }

    let models = fs::read_to_string(output_path.join("kb/models.py")).unwrap();
    for expected in [
        "from ..content.page.models import RenamedPage",
        "    page: RenamedPage | None",
    ] {
        assert!(models.contains(expected), "{expected}\n{models}");
    }
    // Nothing names the pre-override identifier.
    for stale in ["import Page", " Page,", ": Page"] {
        assert!(!services.contains(stale), "{stale}\n{services}");
        assert!(!models.contains(stale), "{stale}\n{models}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

/// The package barrel (`__init__.py`) re-exports every module by name, so two
/// modules declaring the same type name produce `from .a import Page` followed by
/// `from .b import Page`. Python raises nothing: the second binding silently wins
/// and `__all__` lists the name once, so `from pkg import Page` quietly resolves to
/// the wrong model. That silent incorrectness is what P7 forbids, so the generator
/// rejects at load. See `specs/json-schema/PRINCIPLES.md` §15.
#[test]
fn python_json_rejects_same_type_name_in_two_modules() {
    let temp_dir = unique_output_path("py-json-barrel-collision");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("a")).unwrap();
    fs::create_dir_all(input_dir.join("b")).unwrap();
    fs::write(
        input_dir.join("a/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"title":{"type":"string"}}}"#,
    )
    .unwrap();
    fs::write(
        input_dir.join("b/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"count":{"type":"integer"}}}"#,
    )
    .unwrap();

    let request = |output: &str| GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_dir.clone()],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: temp_dir.join(output),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    };

    let error = generate_to_file(&request("out"))
        .expect_err("two modules declaring `Page` collide in the package barrel")
        .to_string();
    // The diagnostic names both modules — the bare type name appears twice and
    // would otherwise read as one declaration seen twice.
    assert!(error.contains("collision"), "{error}");
    assert!(error.contains("a/page#Page"), "{error}");
    assert!(error.contains("b/page#Page"), "{error}");
    assert!(error.contains("x-py-name"), "{error}");

    // The documented escape hatch resolves it, and the barrel then re-exports both.
    fs::write(
        input_dir.join("b/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","x-py-name":"BPage","additionalProperties":false,"properties":{"count":{"type":"integer"}}}"#,
    )
    .unwrap();
    let output_path = temp_dir.join("out-renamed");
    generate_to_file(&request("out-renamed")).expect("the override resolves the collision");
    let barrel = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    for expected in [
        "from .a import Page",
        "from .b import BPage",
        "\"BPage\",",
        "\"Page\",",
    ] {
        assert!(barrel.contains(expected), "{expected}\n{barrel}");
    }
    fs::remove_dir_all(temp_dir).unwrap();
}

/// Re-emitting a `$ref`d type into the referencing service's module produced two
/// `class Page` in different modules, which the package barrel then imported
/// twice — `from .a import Page` followed by `from .svc import Page`, silently
/// binding one copy and dropping the other (P7). It happened whenever the service
/// module declared no types of its own, because reachability pruning read "this
/// module owns nothing" as "this front end does not scope by module".
#[test]
fn python_json_service_module_without_own_types_does_not_reemit_refs() {
    let temp_dir = unique_output_path("py-json-service-only-module");
    let input_dir = temp_dir.join("input");
    fs::create_dir_all(input_dir.join("a")).unwrap();
    fs::write(
        input_dir.join("a/page.json"),
        r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"properties":{"title":{"type":"string"}},"required":["title"]}"#,
    )
    .unwrap();
    fs::write(
        input_dir.join("svc.nexusrpc.yaml"),
        r#"$schema: https://json-schema.org/draft/2020-12/schema
nexusrpc: "1.0.0"
services:
  Svc:
    fqn: example.v1.Svc
    operations:
      one:
        input: { $ref: "a/page.json" }
"#,
    )
    .unwrap();

    let output_path = temp_dir.join("out");
    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_dir],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();

    let rendered = read_python_package_files(&output_path)
        .into_values()
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        rendered.matches("class Page:").count(),
        1,
        "`Page` must be declared once\n{rendered}"
    );
    // The root barrel binds each name exactly once.
    let barrel = fs::read_to_string(output_path.join("__init__.py")).unwrap();
    assert_eq!(barrel.matches("import Page").count(), 1, "{barrel}");
    fs::remove_dir_all(temp_dir).unwrap();
}

/// A declared property named after one of the converter's own identifiers must not
/// shadow it: the parse body holds each property's value in a `<member>_value` slot
/// local, so the shadow is structurally impossible rather than merely unlisted.
///
/// Rendered output is asserted for the mechanism, and the generated package is then
/// **run**, because the failure this guards against is silent: `violations:
/// list[Violation] = []` rebound by a `violations` property's local discarded every
/// collected violation and returned an invalid payload as a model.
/// See `specs/json-schema/PRINCIPLES.md` (P15) and
/// `specs/json-schema/features/properties.md`.
#[test]
fn python_json_property_names_never_shadow_converter_locals() {
    let temp_dir = unique_output_path("py-json-shadowed-names");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("shadow.yaml");
    fs::write(&input_path, SHADOWED_NAME_SCHEMA).unwrap();
    let output_path = temp_dir.join("shadow_package");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    // The accumulator, the decoded mapping and the emitted mapping keep their own
    // names, unshadowed.
    assert!(rendered.contains("violations: list[Violation] = []"));
    assert!(rendered.contains("raw = typing.cast(\"dict[str, typing.Any]\", value)"));
    assert!(rendered.contains("out: dict[str, typing.Any] = {}"));
    // Every property is held in a `_value` slot instead, and its temporaries hang
    // off that slot rather than off the bare member identifier.
    for member in [
        "violations",
        "raw",
        "out",
        "len",
        "int",
        "str",
        "bool",
        "dict",
        "isinstance",
        "typing",
        "math",
        "re",
        "key",
        "value",
        "self",
        "type_hint",
    ] {
        assert!(
            rendered.contains(&format!("{member}_value")),
            "no `_value` slot for the `{member}` property"
        );
    }
    // No property is ever assigned to its bare identifier, which is what shadowed.
    for shadowing in [
        "\n        violations = ",
        "\n        raw = raw[",
        "\n        len = ",
        "\n        int = ",
        "\n        str = ",
        "\n        dict = ",
        "\n        isinstance = ",
        "\n        typing = ",
        "\n        math = ",
        "\n        re = ",
        "\n        self = ",
    ] {
        assert!(
            !rendered.contains(shadowing),
            "a property rebound the converter's own `{}`",
            shadowing.trim().trim_end_matches(" =").trim()
        );
    }
    // The synthesized catch-all frozenset carries the *wire* names, not the locals.
    assert!(rendered.contains("_SHADOW_DECLARED: frozenset[str] = frozenset({\"violations\","));
    assert!(rendered.contains("\"typeHint\"}"));

    assert_python_script_succeeds(
        SHADOWED_NAME_RUNTIME_CHECK,
        &[temp_dir.to_str().unwrap(), "shadow_package"],
    );
    fs::remove_dir_all(temp_dir).unwrap();
}

#[test]
fn python_json_model_properties_use_union_none_and_defaults_preserve_presence() {
    let temp_dir = unique_output_path("py-json-dataclass-default");
    fs::create_dir_all(&temp_dir).unwrap();
    let input_path = temp_dir.join("model.yaml");
    fs::write(&input_path, PYTHON_DATACLASS_DEFAULT_SCHEMA).unwrap();
    let output_path = temp_dir.join("default_package");

    generate_to_file(&GenerateRequest {
        language: nexgen::language::Language::Python,
        input_paths: vec![input_path],
        support_paths: Vec::new(),
        descriptor_paths: Vec::new(),
        output_path: output_path.clone(),
        format: false,
        generate_native_api: false,
        java_package_name: None,
        ts_date_time_types: Default::default(),
    })
    .unwrap();
    let rendered = fs::read_to_string(output_path.join("models.py")).unwrap();

    assert!(rendered.contains("required_plain: str"));
    assert!(rendered.contains("required_nullable: int | None"));
    assert!(rendered.contains("optional_plain: bool | None = None"));
    assert!(rendered.contains("optional_nullable: str | None = None"));
    assert!(rendered.contains("nullable_items: list[str | None] | None = None"));
    assert!(rendered.contains("_salutation: typing.Annotated[str, typing_extensions.deprecated"));
    assert!(rendered.contains("def salutation(self) -> typing.Annotated[str,"));
    assert!(rendered.contains("value: typing.Annotated["));
    assert!(rendered.contains("@salutation.deleter\n    def salutation(self) -> None:"));
    assert!(rendered.contains("if value._salutation is not None:"));
    assert!(!rendered.contains("DEFAULT_SALUTATION"));
    assert!(!rendered.contains("reportDeprecated=false"));
    assert!(!rendered.contains("reportPropertyTypeMismatch=false"));
    assert!(!rendered.contains("reportDeprecated"));
    assert!(!rendered.contains("reportPropertyTypeMismatch"));
    // Converter/helper annotations use the same compact union style.
    assert!(rendered.contains("optional_plain_value: bool | None = None"));
    assert!(rendered.contains("nullable_items_value: list[str | None] | None = None"));

    assert_python_script_succeeds(
        PYTHON_DATACLASS_DEFAULT_RUNTIME_CHECK,
        &[temp_dir.to_str().unwrap(), "default_package"],
    );
    fs::remove_dir_all(temp_dir).unwrap();
}
