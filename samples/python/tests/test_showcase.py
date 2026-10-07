import dataclasses
import json
import math
import typing

import pytest
import temporalio.exceptions

from showcase import (
    Address,
    Attributes,
    Choices,
    Circle,
    ContactPy,
    Extras,
    ItemRules,
    Labels,
    LinkNote,
    Metrics,
    Settings,
    Showcase,
    ShowcaseDetailObject,
    ShowcaseLedgerValue,
    ShowcaseLocation,
    ShowcaseRowsItem,
    Square,
    TextNote,
    Violation,
    Widget,
)

from tests.json_converter_helper import (
    canonical_json_bytes,
    converter_for,
    decode_fixture,
    encode_bytes,
    roundtrip_fixture,
    violation_pairs,
)

SUITE = "showcase"


def test_validation_types_are_exported_from_the_package() -> None:
    violation = Violation(path="name", reason="required")
    error = temporalio.exceptions.ApplicationError(
        "Payload validation failed",
        [violation],
        type="PayloadValidationError",
        non_retryable=True,
    )
    assert violation_pairs(error) == [("name", "required")]


def test_closed_items_precede_array_level_violations() -> None:
    expected = [
        ("values[0]", 'must equal "fixed"'),
        ("values[1]", 'must equal "fixed"'),
        ("values", "must have at least 3 items, got 2"),
        ("values", "duplicate items: element at index 1 equals index 0"),
    ]
    converter = converter_for(ItemRules)
    with pytest.raises(temporalio.exceptions.ApplicationError) as parsed:
        _ = converter.from_transfer_type({"values": ["bad", "bad"]}, ItemRules)
    assert violation_pairs(parsed.value) == expected

    with pytest.raises(temporalio.exceptions.ApplicationError) as serialized:
        invalid_values = typing.cast(list[typing.Literal["fixed"]], ["bad", "bad"])
        _ = converter.to_transfer_type(ItemRules(values=invalid_values))
    assert violation_pairs(serialized.value) == expected


# The ten required members of Showcase; every negative payload starts here so the
# only violations reported are the ones under test. Mirrors the `base` object the
# Go and TypeScript suites use.
BASE: dict[str, typing.Any] = {
    "kind": "showcase",
    "revision": 1,
    "enabled": True,
    "status": "active",
    "tier": 1,
    "scale": 1.5,
    "name": "w",
    "count": 1,
    "active": True,
    "category": "tools",
}


def expect_roundtrip(name: str, model_type: type[typing.Any]) -> typing.Any:
    """Decode through the default converter and compare the re-emitted JSON value.

    The only members that may differ are the explicit `null`s
    ``COLLAPSED_NULL_MEMBERS`` declares on an optional+nullable member, which
    collapse. Everything else round-trips by JSON value — including a key the
    fixture omits on a member carrying a schema `default`, which is advisory and
    never injected.
    """
    return roundtrip_fixture(model_type, SUITE, name)


def expect_showcase(name: str) -> Showcase:
    return typing.cast(Showcase, expect_roundtrip(name, Showcase))


def parse(raw: dict[str, typing.Any]) -> Showcase:
    return converter_for(Showcase).from_transfer_type(raw, Showcase)


def parse_violations(raw: dict[str, typing.Any]) -> list[tuple[str, str]]:
    """The ``(path, reason)`` pairs one bad Showcase payload produces."""
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = parse(raw)
    return violation_pairs(excinfo.value)


def wire_with(member: str, literal: str) -> str:
    """``BASE`` as wire *text*, with ``member`` set to a raw JSON ``literal``.

    Some wire values exist only as text: Python's ``json.loads`` accepts the
    ``Infinity``/``-Infinity``/``NaN`` literals its dialect adds, decodes ``1e400``
    to ``inf``, and decodes an integer literal of any length to an unbounded
    ``int``. Splicing the literal in and parsing it with ``json.loads`` is exactly
    what the SDK's ``JSONPlainPayloadConverter`` does to an incoming payload, so
    these values genuinely reach the converter from untrusted bytes.
    """
    members = [f"{json.dumps(key)}: {json.dumps(value)}" for key, value in BASE.items()]
    members.append(f"{json.dumps(member)}: {literal}")
    return "{" + ", ".join(members) + "}"


def parse_wire_violations(member: str, literal: str) -> list[tuple[str, str]]:
    """The violations raw wire text carrying ``literal`` at ``member`` produces."""
    raw = typing.cast("dict[str, typing.Any]", json.loads(wire_with(member, literal)))
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = parse(raw)
    return violation_pairs(excinfo.value)


def serialize_violations(**replacements: typing.Any) -> list[tuple[str, str]]:
    """The violations serializing a ``BASE`` model with ``replacements`` produces.

    In-memory construction is unchecked (Python §1), so a value past a bound only
    surfaces here — and it must surface *before* any wire form exists (P12), which
    is why the assertion is on the raised violations rather than on output.
    """
    model = dataclasses.replace(parse(BASE), **replacements)
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Showcase).to_transfer_type(model)
    return violation_pairs(excinfo.value)


def test_const_and_enum_value_sets() -> None:
    # `kind`/`revision`/`enabled` are required consts; `status`/`tier`/`scale` are
    # required closed value sets. All six must be on the wire.
    minimal = parse(BASE)
    assert minimal.kind == "showcase"
    assert minimal.revision == 1
    assert minimal.enabled is True
    assert minimal.status == "active"
    assert minimal.tier == 1
    assert minimal.scale == 1.5
    # Public properties materialize defaults without populating the private
    # presence-bearing fields, so the values still omit on the way back out.
    assert minimal.retries == 3
    assert minimal.greeting == "hello"
    assert minimal.debug is False
    assert converter_for(Showcase).to_transfer_type(minimal) == BASE

    # A `const` member, unlike a `default`, DOES carry its value as the dataclass
    # default — it is the only admissible value, not a suggestion — so a
    # hand-constructed model needs only the non-const required members.
    constructed = Showcase(
        status="active",
        tier=1,
        scale=1.5,
        name="w",
        count=1,
        active=True,
        category="tools",
    )
    assert constructed.kind == "showcase"
    assert constructed.revision == 1
    assert constructed.enabled is True
    assert converter_for(Showcase).to_transfer_type(constructed) == BASE

    # Wrong const values.
    assert parse_violations({**BASE, "kind": "nope"}) == [
        ("kind", 'must equal "showcase"')
    ]
    assert parse_violations({**BASE, "revision": 2}) == [("revision", "must equal 1")]
    assert parse_violations({**BASE, "enabled": False}) == [
        ("enabled", "must equal true")
    ]

    # Out-of-set enum values, named with the admissible set and the offending value.
    assert parse_violations({**BASE, "status": "archived"}) == [
        ("status", 'must be one of ["active", "inactive", "pending"], got "archived"')
    ]
    assert parse_violations({**BASE, "tier": 9}) == [
        ("tier", "must be one of [1, 2, 3], got 9")
    ]
    assert parse_violations({**BASE, "scale": 3.5}) == [
        ("scale", "must be one of [1.5, 2.5], got 3.5")
    ]

    # Valid alternative members are accepted.
    ok = parse({**BASE, "status": "pending", "tier": 3, "scale": 2.5})
    assert ok.status == "pending"
    assert ok.tier == 3
    assert ok.scale == 2.5


def test_nullability_states() -> None:
    # required+nullable (`category`): absent is a violation, explicit null is the
    # value, and the null is emitted back.
    assert parse_violations(
        {key: value for key, value in BASE.items() if key != "category"}
    ) == [("category", "required")]
    with_null = parse({**BASE, "category": None})
    assert with_null.category is None
    assert converter_for(Showcase).to_transfer_type(with_null)["category"] is None

    # optional, non-nullable (`nickname`, and the default-bearing `greeting`):
    # an explicit null is a violation, and both are reported at once (P11).
    assert parse_violations({**BASE, "nickname": None, "greeting": None}) == [
        ("nickname", "explicit null not allowed"),
        ("greeting", "explicit null not allowed"),
    ]

    # optional+nullable (`middleName`): absent and explicit null COLLAPSE to the
    # same in-memory state, and both re-serialize as omitted. This matches Go and
    # Java (samples/go/tests/json_schema_showcase_test.go verifies
    # showcase-nulls.json by field checks for exactly this reason); TypeScript is
    # the only target that still round-trips the explicit null.
    from_absent = parse(BASE)
    from_null = parse({**BASE, "middleName": None})
    assert from_absent == from_null
    assert from_null.middle_name is None
    assert "middleName" not in converter_for(Showcase).to_transfer_type(from_null)

    # A closed object rejects an unknown member.
    assert parse_violations({**BASE, "nope": 1}) == [("nope", "unknown field")]

    # A non-object payload is a single structural violation at the root.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Showcase).from_transfer_type(7, Showcase)
    assert violation_pairs(excinfo.value) == [("", "expected object")]


def test_labels_typed_map_and_settings_closed_object() -> None:
    labels = converter_for(Labels).from_transfer_type(
        {"env": "prod", "team": "core"}, Labels
    )
    assert labels.additional_properties == {"env": "prod", "team": "core"}
    assert converter_for(Labels).to_transfer_type(labels) == {
        "env": "prod",
        "team": "core",
    }
    # A map-shaped model is constructed through its explicit catch-all member.
    assert converter_for(Labels).to_transfer_type(
        Labels(additional_properties={"env": "prod"})
    ) == {"env": "prod"}

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Labels).from_transfer_type({"env": 42}, Labels)
    assert violation_pairs(excinfo.value) == [("env", "expected string")]

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Settings).from_transfer_type(
            {"theme": "dark", "unknown": 1}, Settings
        )
    assert violation_pairs(excinfo.value) == [("unknown", "unknown field")]


def test_canonical_wire_fixtures_roundtrip_through_the_default_converter() -> None:
    minimal = expect_showcase("showcase-minimal.json")
    assert minimal.kind == "showcase"
    assert minimal.count == 3
    assert minimal.active is True
    assert minimal.category == "tools"
    # Scalar defaults materialize through properties while their unset private
    # values remain omitted on re-serialize (byte-identity asserted above).
    assert minimal.retries == 3
    assert minimal.greeting == "hello"
    assert minimal.debug is False

    full = expect_showcase("showcase-full.json")
    assert full.retries == 5
    assert full.middle_name == "Q"
    assert full.tags == ["a", "b"]
    assert full.aliases == ["alpha", "beta"]
    assert full.roles == ["admin", "user"]
    assert full.address is not None
    assert full.address.street == "1 Main St"
    assert full.address.zip == 90210
    assert full.address.additional_properties == {"region": "west"}
    assert full.labels is not None
    assert full.labels.additional_properties == {"env": "prod", "team": "core"}
    assert full.settings is not None
    assert full.settings.font_size == 14

    # showcase-nulls.json carries `middleName: null` on an optional+nullable
    # member, which collapses and is therefore dropped on re-serialize.
    nulls = expect_showcase("showcase-nulls.json")
    assert nulls.middle_name is None
    # `category` is required+nullable, so ITS explicit null does survive.
    assert nulls.category is None
    assert nulls.active is False
    assert nulls.count == 0

    address = typing.cast(Address, expect_roundtrip("address-open.json", Address))
    assert address.street == "1 Main St"
    assert address.additional_properties == {"x-extra": 7}

    labels = typing.cast(Labels, expect_roundtrip("labels.json", Labels))
    assert labels.additional_properties == {"env": "prod", "team": "core"}

    settings = typing.cast(Settings, expect_roundtrip("settings.json", Settings))
    assert settings.theme == "dark"
    assert settings.font_size == 14


def test_numeric_constraints_roundtrip_and_reject() -> None:
    metrics = expect_showcase("showcase-metrics.json")
    assert metrics.priority == 5
    assert metrics.level == 2
    assert metrics.ratio == 15.0
    assert metrics.step == 9

    assert parse_violations({**BASE, "priority": 99}) == [
        ("priority", "must be <= 10, got 99")
    ]
    assert parse_violations({**BASE, "level": 0}) == [("level", "must be > 0, got 0")]
    assert parse_violations({**BASE, "step": 7}) == [
        ("step", "must be a multiple of 3, got 7")
    ]
    assert parse_violations({**BASE, "ratio": 7}) == [
        ("ratio", "must be a multiple of 5, got 7")
    ]
    # P11: one member can produce SEVERAL violations — `ratio` is both below
    # `minimum` and off the `multipleOf` grid, and both are reported.
    assert parse_violations({**BASE, "ratio": 3}) == [
        ("ratio", "must be >= 5, got 3"),
        ("ratio", "must be a multiple of 5, got 3"),
    ]


def test_integer_semantics() -> None:
    # An integral JSON number is an integer.
    assert parse({**BASE, "count": 3.0}).count == 3
    # A boolean, a fractional number, and a value past the +/-(2**53-1) cap all
    # report the single "expected integer" reason.
    for bad in (True, 1.5, 2**53):
        assert parse_violations({**BASE, "count": bad}) == [
            ("count", "expected integer")
        ]
    # A plain integer member normalizes its wire form too, on bytes.
    assert encode_bytes(parse({**BASE, "count": 3.0})) == canonical_json_bytes(
        {**BASE, "count": 3}
    )


def test_integral_closed_value_sets_normalize_to_the_integer_wire_form() -> None:
    """An integer `const`/`enum` routes the wire value through the spec-integer
    parse before the membership comparison, so `1.0` becomes an `int` and re-emits
    as `1` — matching Go's `parseIntegerField` and Java's `SpecNumbers.specLong`.

    Asserted on **bytes**: `1 == 1.0` in Python, so a parsed comparison passes
    either way and cannot see a closed set that kept its wire `float`.
    """
    model = parse({**BASE, "revision": 1.0, "tier": 2.0})
    # `is int` rather than `== 1`: a `float` would satisfy the equality.
    assert type(model.revision) is int
    assert type(model.tier) is int
    assert encode_bytes(model) == canonical_json_bytes({**BASE, "tier": 2})

    # Routing through the spec-integer parse also reinstates the two checks a
    # closed numeric set used to bypass entirely: the fractional reject...
    assert parse_violations({**BASE, "revision": 1.5}) == [
        ("revision", "expected integer")
    ]
    assert parse_violations({**BASE, "tier": 2.5}) == [("tier", "expected integer")]
    # ...and the +/-(2**53-1) cap, which a value inside the closed set can exceed
    # only by being outside it — so the cap must be reported before membership.
    assert parse_violations({**BASE, "revision": 2**53 + 1}) == [
        ("revision", "expected integer")
    ]
    assert parse_violations({**BASE, "tier": -(2**53) - 1}) == [
        ("tier", "expected integer")
    ]

    # A **float**-valued closed set has no `Literal` form (PEP 586) and keeps the
    # wire value as it arrived, so `2.5` stays `2.5` rather than becoming `2`.
    scaled = parse({**BASE, "scale": 2.5})
    assert scaled.scale == 2.5
    assert encode_bytes(scaled) == canonical_json_bytes({**BASE, "scale": 2.5})


def test_non_finite_numbers_are_rejected_in_both_directions() -> None:
    """`Infinity`/`-Infinity`/`NaN` are reachable from the wire — Python's
    `json.loads` accepts the literals its dialect adds — and every one of them is a
    violation rather than a crash or a round-trip.

    Bytes Go's `json.Unmarshal`, `JSON.parse` and Jackson all reject must never
    become a model here, and must never be emitted either (P1/P12).
    """
    # `ratio` carries `multipleOf`, whose `math.fmod(inf, 5)` raised `ValueError`
    # (and `OverflowError` for an out-of-binary64 integer literal) — escaping the
    # aggregated `ApplicationError` entirely (P11).
    for literal, rendered in [
        ("Infinity", "inf"),
        ("-Infinity", "-inf"),
        ("NaN", "nan"),
        ("1e400", "inf"),
    ]:
        assert parse_wire_violations("ratio", literal) == [
            ("ratio", f"must be a finite number, got {rendered}")
        ]

    # A 401-digit integer literal decodes to an unbounded Python `int`, which is
    # past binary64 without ever being a `float`.
    digits = "9" * 401
    assert parse_wire_violations("ratio", digits) == [
        ("ratio", f"must be a finite number, got {digits}")
    ]

    # A `number` with **no** other constraint was the worse case: it parsed and
    # re-serialized `inf` verbatim. `Circle.radius` is one, reached through a union.
    assert parse_wire_violations("shape", '{"kind": "circle", "radius": Infinity}') == [
        ("shape.radius", "must be a finite number, got inf")
    ]

    # The serialize direction: an unchecked dataclass holding `inf` fails before a
    # byte is written, rather than emitting a value this module's own parser rejects.
    assert serialize_violations(ratio=float("inf")) == [
        ("ratio", "must be a finite number, got inf")
    ]
    assert serialize_violations(ratio=float("-inf")) == [
        ("ratio", "must be a finite number, got -inf")
    ]
    assert serialize_violations(ratio=float("nan")) == [
        ("ratio", "must be a finite number, got nan")
    ]
    # A finite value on the boundary still serializes.
    assert encode_bytes(
        dataclasses.replace(parse(BASE), ratio=5.0)
    ) == canonical_json_bytes({**BASE, "ratio": 5.0})


def test_string_length_constraints_roundtrip_and_reject() -> None:
    # The astral crux: "a😀b" is 3 code points but 6 UTF-8 bytes, so it passes
    # `code` maxLength:5 — lengths are counted in code points, not bytes.
    strings = expect_showcase("showcase-strings.json")
    assert strings.code == "a😀b"
    assert strings.nickname == "buddy"

    assert parse_violations({**BASE, "code": "a"}) == [
        ("code", "must have length >= 2, got 1")
    ]
    assert parse_violations({**BASE, "code": "abcdef"}) == [
        ("code", "must have length <= 5, got 6")
    ]
    # 6 emoji = 6 code points (24 bytes); rejected by code-point count.
    assert parse_violations({**BASE, "code": "😀😀😀😀😀😀"}) == [
        ("code", "must have length <= 5, got 6")
    ]
    # A multi-byte value within the code-point bound is accepted (a byte count of
    # 6 would exceed maxLength:5 — proving code points, not bytes).
    assert parse({**BASE, "code": "a😀b"}).code == "a😀b"


def test_pattern_constraints_roundtrip_and_reject() -> None:
    # sku `^[A-Z]{2,4}$` and phrase `^\S+\s\S+$` round-trip.
    patterns = expect_showcase("showcase-patterns.json")
    assert patterns.sku == "AB"
    assert patterns.phrase == "hello world"

    # The reason names the *lowered* pattern (the loader rewrote `\s`/`\S` to an
    # ASCII class and `$` to Python's `\Z`), so only its head is asserted here.
    for member, value in [
        ("sku", "ab"),  # lowercase, not [A-Z]
        ("sku", "ABCDE"),  # 5 letters, above {2,4}
        ("phrase", "helloworld"),  # no whitespace separator
        # `\s` ASCII-class crux: a NBSP (U+00A0) is not ASCII whitespace, so the
        # normalized `[\t\n\x0B\f\r ]` rejects it — matching Go/TS/Java.
        ("phrase", "hello world"),
        # `$` end-anchor crux: Python `re`'s `$` matches before a trailing `\n`,
        # so the loader rewrote `$` -> `\Z` and a trailing newline is rejected,
        # consistent with Go/TS/Java.
        ("phrase", "hello world\n"),
    ]:
        violations = parse_violations({**BASE, member: value})
        assert [path for path, _ in violations] == [member]
        assert violations[0][1].startswith("must match pattern ")

    # The reason quotes the authored pattern (`$`), not the `\Z` rewrite
    # compiled for matching — the same text Go/TS/Java print.
    assert parse_violations({**BASE, "sku": "ab"}) == [
        ("sku", 'must match pattern ^[A-Z]{2,4}$, got "ab"')
    ]

    ok = parse({**BASE, "sku": "XY", "phrase": "hello world"})
    assert ok.sku == "XY"
    assert ok.phrase == "hello world"


def test_format_constraints_roundtrip_and_reject() -> None:
    # uuid/email/hostname/uri/ipv4 round-trip (string-typed, no materialization).
    formats = expect_showcase("showcase-format.json")
    assert formats.request_id == "de305d54-75b4-431b-adb2-eb6b9e546013"
    assert formats.contact_email == "user@example.com"
    assert formats.host == "api.example.com"
    assert formats.homepage == "https://example.com/path?q=1#frag"
    assert formats.gateway == "192.168.0.1"

    long_host = ".".join(["abc"] * 64)  # > 253 code points
    for member, value, format_name in [
        ("requestId", "not-a-uuid", "uuid"),
        # A single-label email domain (user@localhost) is rejected.
        ("contactEmail", "user@localhost", "email"),
        ("gateway", "256.0.0.1", "ipv4"),
        # A double-`::` IPv6 IP-literal host: the spliced ipv6 grammar rejects it.
        ("homepage", "http://[1::2::3]", "uri"),
        ("host", long_host, "hostname"),
    ]:
        # The offending value is rendered in its JSON form, exactly as Go and
        # TypeScript render it.
        assert parse_violations({**BASE, member: value}) == [
            (member, f'must be a valid {format_name}, got "{value}"')
        ]


def test_array_constraints_roundtrip_and_reject() -> None:
    ok = parse({**BASE, "tags": ["a"], "aliases": ["x", "y"], "roles": ["admin"]})
    assert ok.roles == ["admin"]

    assert parse_violations({**BASE, "tags": []}) == [
        ("tags", "must have at least 1 items, got 0")
    ]
    assert parse_violations({**BASE, "tags": ["a", "b", "c", "d", "e", "f"]}) == [
        ("tags", "must have at most 5 items, got 6")
    ]
    assert parse_violations({**BASE, "aliases": ["x", "x"]}) == [
        ("aliases", "duplicate items: element at index 1 equals index 0")
    ]
    assert parse_violations({**BASE, "roles": ["user"]}) == [
        ("roles", "too few matching items: at least 1, got 0")
    ]
    assert parse_violations({**BASE, "roles": ["admin", "admin", "admin"]}) == [
        ("roles", "too many matching items: at most 2, got 3")
    ]
    assert parse_violations({**BASE, "roles": [1, "admin"]}) == [
        ("roles[0]", "expected string")
    ]


def test_array_element_type_mismatch_names_the_expected_type() -> None:
    """A mistyped element reports the type it failed to be, at its own index.

    Every element kind takes the same parse the value in that position would take
    anywhere else, so a `string` element reads `expected string` — the same reason a
    `string` member reports, and the same one Java's element loop and a union's array
    branch (`measurements[0]`, `expected number`) use. The index in the path is what
    identifies the element; the reason names the type.
    """
    assert parse_violations({**BASE, "tags": [1]}) == [("tags[0]", "expected string")]
    assert parse_violations({**BASE, "aliases": [1, 2]}) == [
        ("aliases[0]", "expected string"),
        ("aliases[1]", "expected string"),
    ]
    assert parse_violations({**BASE, "aliases": [1, 1]}) == [
        ("aliases[0]", "expected string"),
        ("aliases[1]", "expected string"),
        ("aliases", "duplicate items: element at index 1 equals index 0"),
    ]
    assert parse_violations({**BASE, "tags": ["a", None, {}]}) == [
        ("tags[1]", "expected string"),
        ("tags[2]", "expected string"),
    ]


def test_number_roundtrip_preserves_mathematical_values() -> None:
    value = expect_showcase("showcase-number-values.json")
    assert value.number_grid is not None
    numbers = value.number_grid[0]
    assert numbers[:3] == [0.0, 5.0, 1000.0]
    assert numbers[3] == float.fromhex("0x1.fffffffffffffp+1023")
    assert numbers[4] == float.fromhex("0x0.0000000000001p-1022")
    # Signed zero is deliberately not identity-bearing.
    assert numbers[0] == 0.0
    assert math.isfinite(numbers[3]) and math.isfinite(numbers[4])

    # A `number` is narrowed to binary64 on the way in, so Python lands on the
    # same value the other three targets do. Python's `int` is the only unbounded
    # one, so without the narrowing this field held 9007199254740993 exactly
    # while Go, TypeScript and Java rounded it -- one payload reading back as two
    # different numbers, which is the round-trip agreement P1 requires. The
    # conformance manifest pins the four-target half of this
    # (`mathematical-number-equality`).
    past_the_safe_range = parse({**BASE, "score": 9007199254740993})
    assert isinstance(past_the_safe_range.score, float)
    assert past_the_safe_range.score == 9007199254740992.0
    assert encode_bytes(past_the_safe_range) == canonical_json_bytes(
        {**BASE, "score": 9007199254740992.0}
    )


def test_object_constraints_roundtrip_and_reject() -> None:
    attributes = typing.cast(
        Attributes, expect_roundtrip("attributes.json", Attributes)
    )
    assert attributes.additional_properties == {"host": "a", "port": "8080"}
    contact = typing.cast(ContactPy, expect_roundtrip("contact.json", ContactPy))
    assert contact.shipping_street == "1 Main St"
    assert contact.shipping_zip == "90210"

    # minProperties/maxProperties over the distinct wire-key count sit at the
    # object root, so their violation path is empty.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Attributes).from_transfer_type({}, Attributes)
    assert violation_pairs(excinfo.value) == [
        ("", "must have at least 1 properties, got 0")
    ]

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Attributes).from_transfer_type(
            {"a": "1", "b": "2", "c": "3", "d": "4"}, Attributes
        )
    assert violation_pairs(excinfo.value) == [
        ("", "must have at most 3 properties, got 4")
    ]

    # propertyNames maxLength:8 — an over-long key, keyed by the offending key.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Attributes).from_transfer_type(
            {"toolongkey": "1"}, Attributes
        )
    assert violation_pairs(excinfo.value) == [
        (
            "toolongkey",
            'invalid property name "toolongkey": must have length <= 8, got 10',
        )
    ]

    # dependentRequired — a shipping street present without a shipping zip.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(ContactPy).from_transfer_type(
            {"shippingStreet": "1 Main St"}, ContactPy
        )
    assert violation_pairs(excinfo.value) == [
        (
            "shippingZip",
            'property "shippingZip" is required when "shippingStreet" is present',
        )
    ]

    # minProperties:1 on a declared-property object — an empty object.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(ContactPy).from_transfer_type({}, ContactPy)
    assert violation_pairs(excinfo.value) == [
        ("", "must have at least 1 properties, got 0")
    ]

    ok = converter_for(ContactPy).from_transfer_type(
        {"shippingStreet": "1 Main St", "shippingZip": "90210"}, ContactPy
    )
    assert ok.shipping_zip == "90210"


def test_all_of_merged_widget() -> None:
    # Widget is an allOf base-type extension (WidgetBase folded in + an extension
    # branch): a flat standalone object with the union of properties ({id, kind,
    # name, size}) and required ([id, name]), with no allOf residue.
    widget = typing.cast(Widget, expect_roundtrip("widget.json", Widget))
    assert widget.id == "w-1"
    assert widget.kind == "gadget"
    assert widget.name == "Widget One"
    assert widget.size == 15

    converter = converter_for(Widget)

    # `size` carries a bound tightened from two allOf branches to [10, 20].
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter.from_transfer_type(
            {"id": "w-1", "name": "Widget One", "size": 5}, Widget
        )
    assert violation_pairs(excinfo.value) == [("size", "must be >= 10, got 5")]

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter.from_transfer_type(
            {"id": "w-1", "name": "Widget One", "size": 25}, Widget
        )
    assert violation_pairs(excinfo.value) == [("size", "must be <= 20, got 25")]

    # A missing required member contributed by the extension branch is rejected.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter.from_transfer_type({"id": "w-1"}, Widget)
    assert violation_pairs(excinfo.value) == [("name", "required")]

    # A value on the tightened boundary validates.
    ok = converter.from_transfer_type(
        {"id": "w-1", "name": "Widget One", "size": 10}, Widget
    )
    assert ok.size == 10


def test_one_of_sum_types_roundtrip_and_reject() -> None:
    # Disjoint-kind union (str | int): each branch round-trips and is selected by
    # the wire token.
    as_string = expect_showcase("showcase-union-string.json")
    assert as_string.id_or_name == "abc"
    as_int = expect_showcase("showcase-union-int.json")
    assert as_int.id_or_name == 7

    # Discriminated (tagged) union (Circle | Square) selected by `kind`.
    circle = expect_showcase("showcase-shape-circle.json")
    assert isinstance(circle.shape, Circle)
    assert circle.shape.kind == "circle"
    assert circle.shape.radius == 2.5
    square = expect_showcase("showcase-shape-square.json")
    assert isinstance(square.shape, Square)
    assert square.shape.kind == "square"
    assert square.shape.side == 4

    # An unmatchable wire token (boolean) matches no branch of str | int.
    assert parse_violations({**BASE, "idOrName": True}) == [
        ("idOrName", "expected one of: string, integer")
    ]

    # An unknown discriminator value is rejected (closed value set, P13.1).
    assert parse_violations({**BASE, "shape": {"kind": "triangle"}}) == [
        (
            "shape",
            'unknown discriminator kind triangle: expected one of ["circle", "square"]',
        )
    ]


def test_one_of_branch_constraints() -> None:
    """Once the token selects a branch, the value is held to everything that
    branch declares: each union member carries its own constraints."""
    assert parse({**BASE, "idOrName": "abc"}).id_or_name == "abc"
    assert parse({**BASE, "idOrName": 1}).id_or_name == 1
    assert parse_violations({**BASE, "idOrName": "ab"}) == [
        ("idOrName", "must have length >= 3, got 2")
    ]
    assert parse_violations({**BASE, "idOrName": 0}) == [
        ("idOrName", "must be >= 1, got 0")
    ]

    # A closed value set on a branch: an unknown string matches no member.
    assert parse({**BASE, "mode": "manual"}).mode == "manual"
    assert parse({**BASE, "mode": 7}).mode == 7
    assert parse_violations({**BASE, "mode": "turbo"}) == [
        ("mode", 'must be one of ["auto", "manual"], got "turbo"')
    ]
    assert parse_violations({**BASE, "mode": -1}) == [("mode", "must be >= 0, got -1")]

    # The array branch's `minItems`/`uniqueItems` and the string branch's
    # `pattern`, on the same union.
    assert parse({**BASE, "measurements": [1.5, 2.5]}).measurements == [1.5, 2.5]
    assert parse_violations({**BASE, "measurements": []}) == [
        ("measurements", "must have at least 1 items, got 0")
    ]
    assert parse_violations({**BASE, "measurements": [1.5, 1.5]}) == [
        ("measurements", "duplicate items: element at index 1 equals index 0")
    ]
    measurement_violations = parse_violations({**BASE, "measurements": "AUTO"})
    assert [path for path, _ in measurement_violations] == ["measurements"]
    assert measurement_violations[0][1].startswith("must match pattern ")

    # An element union's branch constraints hold per element, at its own index.
    assert parse({**BASE, "segments": ["ab", 0]}).segments == ["ab", 0]
    assert parse_violations({**BASE, "segments": ["a"]}) == [
        ("segments[0]", "must have length >= 2, got 1")
    ]
    assert parse_violations({**BASE, "segments": [-1]}) == [
        ("segments[0]", "must be >= 0, got -1")
    ]


def test_free_form_object_roundtrip_and_reject() -> None:
    # The free-form object in both positions: the inline object branch of the
    # `payload` union (carried structurally as a dict) and the named `Extras`
    # model (carried in its catch-all member). Members are kept verbatim, so a
    # large integer survives untruncated.
    as_object = expect_showcase("showcase-freeform.json")
    assert isinstance(as_object.payload, dict)
    assert as_object.payload["big"] == 9007199254740992
    assert as_object.extras is not None
    assert as_object.extras.additional_properties["note"] == "free-form"

    # The same union's string branch, selected by the wire token.
    as_string = expect_showcase("showcase-freeform-string.json")
    assert as_string.payload == "text"

    # The named free-form model round-trips standalone, nested members included.
    extras = typing.cast(Extras, expect_roundtrip("extras.json", Extras))
    assert extras.additional_properties["nested"] == {"a": 1}
    assert extras.additional_properties["count"] == 9007199254740992

    # maxProperties over the member set is enforced, at the object root.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Extras).from_transfer_type(
            {"a": 1, "b": 2, "c": 3, "d": 4, "e": 5}, Extras
        )
    assert violation_pairs(excinfo.value) == [
        ("", "must have at most 4 properties, got 5")
    ]

    # An unmatchable wire token (boolean) matches no branch of object | string.
    assert parse_violations({**BASE, "payload": True}) == [
        ("payload", "expected one of: object, string")
    ]


def test_inline_object_union_roundtrip_and_reject() -> None:
    # The `note` tagged union's branches are written inline in the schema and
    # named by their `x-py-name` overrides, so each is an ordinary dataclass the
    # union's free functions dispatch to on the `kind` const.
    text = expect_showcase("showcase-note-text.json")
    assert isinstance(text.note, TextNote)
    assert text.note.kind == "text"
    assert text.note.body == "remember the milk"
    # The branch stays open: an unknown member is preserved (P13).
    assert text.note.additional_properties == {"pinned": True}

    link = expect_showcase("showcase-note-link.json")
    assert isinstance(link.note, LinkNote)
    assert link.note.href == "https://example.test/notes/1"

    # The selected branch's own constraints are enforced, at the nested path.
    assert parse_violations({**BASE, "note": {"kind": "text", "body": ""}}) == [
        ("note.body", "must have length >= 1, got 0")
    ]

    # An unknown tag value matches no branch.
    assert parse_violations({**BASE, "note": {"kind": "audio"}}) == [
        ("note", 'unknown discriminator kind audio: expected one of ["text", "link"]')
    ]


def test_property_inline_object_union_roundtrip_and_reject() -> None:
    # `detail`'s union is written inline on the property; its lone structured
    # object branch derives `ShowcaseDetailObject` from the union it belongs to
    # and is an ordinary model.
    object_detail = expect_showcase("showcase-detail-object.json")
    assert isinstance(object_detail.detail, ShowcaseDetailObject)
    assert object_detail.detail.code == "E_LIMIT"
    assert object_detail.detail.hint == "retry later"
    # The branch stays open: an unknown member is preserved (P13).
    assert object_detail.detail.additional_properties == {"retryAfterMs": 250}

    text = expect_showcase("showcase-detail-string.json")
    assert text.detail == "E_LIMIT"

    assert parse_violations({**BASE, "detail": {"code": ""}}) == [
        ("detail.code", "must have length >= 1, got 0")
    ]

    # A value admitted by no branch is rejected, naming the admissible branches.
    assert parse_violations({**BASE, "detail": 7}) == [
        ("detail", "expected one of: ShowcaseDetailObject, string")
    ]


def test_tagged_union_with_scalar_branch_roundtrip_and_reject() -> None:
    # `shapeOrName` composes both selector layers: the JSON token picks
    # object-vs-string, then the `kind` const picks Circle-vs-Square. Both branch
    # models are the ones the `shape` union already uses.
    square = expect_showcase("showcase-shape-or-name-square.json")
    assert isinstance(square.shape_or_name, Square)
    assert square.shape_or_name.side == 4

    named = expect_showcase("showcase-shape-or-name-string.json")
    assert named.shape_or_name == "unit-square"

    # An object with an unknown tag matches no branch — it does not fall back to
    # the string branch.
    assert parse_violations({**BASE, "shapeOrName": {"kind": "triangle"}}) == [
        (
            "shapeOrName",
            'unknown discriminator kind triangle: expected one of ["circle", "square"]',
        )
    ]

    # A value admitted by no branch names all three.
    assert parse_violations({**BASE, "shapeOrName": 7}) == [
        ("shapeOrName", "expected one of: Circle, Square, string")
    ]


def test_array_branch_union_roundtrip_and_reject() -> None:
    # `measurements` is `list[float] | str`: Python carries the array branch
    # structurally (no synthesized variant model).
    values = expect_showcase("showcase-measurements-array.json")
    assert values.measurements == [1.5, 2.5, 3.75]

    preset = expect_showcase("showcase-measurements-string.json")
    assert preset.measurements == "auto"

    # A value admitted by neither branch is rejected, naming both admissible
    # kinds. An array branch has no name to take, so its label is the language's
    # own type spelling (`list[float]`, where TypeScript says `number[]`); scalar
    # branches use the JSON-Schema kind word.
    assert parse_violations({**BASE, "measurements": True}) == [
        ("measurements", "expected one of: list[float], string")
    ]


def test_union_array_branch_types_every_element() -> None:
    """Once the wire token selects the array branch, the branch decodes
    **elementwise** — the same element parse a declared array member runs — so a bad
    element is rejected at its own index.

    The branch used to cast the whole value to `list[float]` and run only
    `minItems`/`uniqueItems`, so any list at all was admitted: `["a", "b"]`
    round-tripped verbatim while Go decodes `[]float64` and Java binds a typed
    list, both rejecting (P1).
    """
    assert parse_violations({**BASE, "measurements": [{"x": 1}]}) == [
        ("measurements[0]", "expected number")
    ]
    assert parse_violations({**BASE, "measurements": ["a"]}) == [
        ("measurements[0]", "expected number")
    ]
    # A `bool` is not a number, which is also what resolves the `True == 1`
    # uniqueness discrepancy at its root: the element is rejected before
    # `_check_unique_items` ever compares it against a `1`.
    assert parse_violations({**BASE, "measurements": [1.0, True]}) == [
        ("measurements[1]", "expected number")
    ]
    # A non-finite element is caught per element too.
    assert parse_wire_violations("measurements", "[Infinity]") == [
        ("measurements[0]", "must be a finite number, got inf")
    ]

    two = parse_violations({**BASE, "measurements": ["a", "b"]})
    assert two == [
        ("measurements[0]", "expected number"),
        ("measurements[1]", "expected number"),
    ]

    # Valid elements still decode, and re-emit as the same mathematical numbers.
    # A `number` element is narrowed to binary64 on the way in, so the integral
    # `2` reads back as `2.0` -- one JSON value under P1 ("a number's lexical
    # spelling" is not part of JSON identity), and the same `double` Go,
    # TypeScript and Java hold. Without the narrowing Python was the only target
    # keeping an unbounded `int` here, so an element past 2 ** 53 round-tripped
    # to a different number than everywhere else.
    values = parse({**BASE, "measurements": [1.5, 2, 3.75]})
    assert values.measurements is not None
    assert values.measurements == [1.5, 2, 3.75]
    assert all(isinstance(m, float) for m in values.measurements)
    assert encode_bytes(values) == canonical_json_bytes(
        {**BASE, "measurements": [1.5, 2.0, 3.75]}
    )


def test_element_position_unions_roundtrip_and_reject() -> None:
    # Unions in positions with no property of their own: an array element at a
    # named union (`shapes`), an array element at an inline union the loader names
    # `ShowcaseSegmentsItem`, and a map member at an inline union named
    # `ChoicesValue`.
    value = expect_showcase("showcase-element-unions.json")
    assert value.shapes is not None
    assert isinstance(value.shapes[0], Circle)
    assert value.shapes[0].radius == 2.5
    assert isinstance(value.shapes[1], Square)
    assert value.shapes[1].side == 4
    assert value.segments == ["alpha", 7]
    # Element nullability is the element's own concern: `list[str | None]`, so an
    # explicit null is a member rather than a violation, and it survives the
    # round-trip (the optional+nullable collapse is a *property* rule).
    assert value.slots == ["first", None, "third"]
    # A map's members live in the explicit catch-all, materialized into their
    # declared member type — here the union each member is routed to.
    assert value.choices is not None
    primary = value.choices.additional_properties["primary"]
    assert isinstance(primary, Circle)
    assert primary.radius == 1

    # An element admitted by no branch is rejected, at its own index.
    assert parse_violations(
        {**BASE, "shapes": [{"kind": "circle", "radius": 1}, True]}
    ) == [("shapes[1]", "expected one of: Circle, Square")]

    assert parse_violations({**BASE, "segments": ["ok", 1.5]}) == [
        ("segments[1]", "expected one of: string, integer")
    ]

    # A map member's violation carries its key under the map's own path.
    assert parse_violations({**BASE, "choices": {"primary": "circle"}}) == [
        ("choices.primary", "expected one of: Circle, Square")
    ]


def test_content_encoding_roundtrip_and_reject() -> None:
    # blob (base64) and urlBlob (base64url) round-trip: a JSON string on the wire,
    # native `bytes` in the model, re-encoded byte-identically. The same bytes
    # (">>>") encode differently per encoding ("Pj4+" vs "Pj4-").
    parsed = expect_showcase("showcase-bytes.json")
    assert parsed.blob == b">>>"
    assert parsed.url_blob == b">>>"

    for member, value, encoding in [
        # A base64 field using the URL-safe alphabet.
        ("blob", "Pj4-", "base64"),
        # A base64 field missing padding.
        ("blob", "aGk", "base64"),
        # A base64url field carrying padding.
        ("urlBlob", "aGk=", "base64url"),
    ]:
        assert parse_violations({**BASE, member: value}) == [
            (member, f'must be {encoding}-encoded, got "{value}"')
        ]


def test_inline_object_shapes_roundtrip_and_reject() -> None:
    # An object written inline in a value position is named after that position
    # and emitted as an ordinary model: a property (`location`, with its own
    # nested `geo`), a nullable property (`audit`), an array element (`rows`), a
    # map and its member (`ledger`), and a free-form bag (`metadata`). The same
    # fixture covers a typed map's member constraints (`quotas`, `tokens`,
    # `nicknames`) and a nested array (`grid`).
    value = expect_showcase("showcase-inline-shapes.json")
    assert value.grid == [[1, 2], [3]]
    assert value.location is not None
    assert value.location.city == "Springfield"
    assert value.location.geo is not None
    assert value.location.geo.lat == 39.8
    assert value.audit is not None
    assert value.audit.by == "alice"
    assert value.rows is not None
    assert value.rows[0].cell == "a1"
    # The member override renamed the member (`ledger_py`); the hoisted types keep
    # their position-derived names.
    assert value.ledger_py is not None
    opening = value.ledger_py.additional_properties["opening"]
    assert isinstance(opening, ShowcaseLedgerValue)
    assert opening.amount == 100
    assert value.metadata is not None
    assert value.metadata.additional_properties == {"source": "import", "batch": 7}
    assert value.quotas is not None
    assert value.quotas.additional_properties == {"cpu": 20, "memory": 100}
    # A null member of a map of nullable members is a member, not a violation.
    assert value.nicknames is not None
    assert value.nicknames.additional_properties == {"short": "al", "none": None}

    # A hoisted shape validates like any other model, at the nested path.
    assert parse_violations({**BASE, "location": {"city": ""}}) == [
        ("location.city", "must have length >= 1, got 0")
    ]
    assert parse_violations({**BASE, "rows": [{"cell": "ok"}, {}]}) == [
        ("rows[1].cell", "required")
    ]
    # A nested array reports the failing element at its own two-dimensional index.
    assert parse_violations({**BASE, "grid": [[1], [2, 1.5]]}) == [
        ("grid[1][1]", "expected integer")
    ]
    # A typed map's member constraints are enforced, keyed by the member under
    # the map's own path.
    assert parse_violations({**BASE, "quotas": {"cpu": 7}}) == [
        ("quotas.cpu", "must be a multiple of 5, got 7")
    ]
    token_violations = parse_violations({**BASE, "tokens": {"primary": "AB"}})
    assert [path for path, _ in token_violations] == ["tokens.primary"]
    assert token_violations[0][1].startswith("must match pattern ")
    assert parse_violations({**BASE, "nicknames": {"tiny": "a"}}) == [
        ("nicknames.tiny", "must have length >= 2, got 1")
    ]
    # The free-form bag's member-count bound rides with the hoisted type.
    assert parse_violations({**BASE, "metadata": {"a": 1, "b": 2, "c": 3, "d": 4}}) == [
        ("metadata", "must have at most 3 properties, got 4")
    ]


def test_recursive_collections_and_non_finite_positions() -> None:
    value = expect_showcase("showcase-recursive-collections.json")
    assert value.number_grid == [[1, 2.5], [3]]
    assert value.addresses is not None
    assert value.addresses[0].street == "1 Main St"
    assert value.address_book is not None
    assert value.address_book.additional_properties["home"].street == "2 Side St"
    assert value.dates is not None
    assert value.dates[0].year == 1
    assert value.date_index is not None
    assert value.date_index.additional_properties["first"].year == 1
    assert value.blobs == [b"hi", b">>>"]
    assert value.blob_index is not None
    assert value.blob_index.additional_properties["hi"] == b"hi"

    violations = parse_violations({**BASE, "slots": ["x"], "links": ["not a uri"]})
    assert [path for path, _ in violations] == ["slots[0]", "links[0]"]

    replacements = [
        ({"score": float("nan")}, "score"),
        ({"metric_or_label": float("inf")}, "metricOrLabel"),
        ({"measurements": [float("-inf")]}, "measurements[0]"),
        ({"number_grid": [[1, float("nan")]]}, "numberGrid[0][1]"),
        (
            {"metrics": Metrics(additional_properties={"cpu": float("inf")})},
            "metrics.cpu",
        ),
    ]
    for replacement, path in replacements:
        invalid = dataclasses.replace(value, **replacement)
        with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
            _ = converter_for(Showcase).to_transfer_type(invalid)
        pairs = violation_pairs(excinfo.value)
        assert pairs[0][0] == path
        assert "finite number" in pairs[0][1]


def test_serialize_rejects_invalid_in_memory_values() -> None:
    """P12: `to_transfer_type` re-runs every check the parse side runs, so an
    in-memory value past a bound is rejected before any wire form is produced."""
    converter = converter_for(Showcase)
    full = decode_fixture(Showcase, SUITE, "showcase-full.json")
    # A valid model still serializes cleanly (no false rejection).
    _ = converter.to_transfer_type(full)

    for replacement, expected in [
        ({"priority": 42}, ("priority", "must be <= 10, got 42")),
        ({"code": "abcdef"}, ("code", "must have length <= 5, got 6")),
        (
            {"aliases": ["dup", "dup"]},
            ("aliases", "duplicate items: element at index 1 equals index 0"),
        ),
        (
            {"status": typing.cast(typing.Any, "archived")},
            (
                "status",
                'must be one of ["active", "inactive", "pending"], got "archived"',
            ),
        ),
        ({"revision": typing.cast(typing.Any, 2)}, ("revision", "must equal 1")),
        ({"count": typing.cast(typing.Any, True)}, ("count", "expected integer")),
        ({"count": typing.cast(typing.Any, 1.5)}, ("count", "expected integer")),
        (
            {"count": 10**400},
            ("count", "exceeds ±(2^53-1) integer cap"),
        ),
        ({"active": typing.cast(typing.Any, 1)}, ("active", "expected boolean")),
        ({"nickname": typing.cast(typing.Any, [])}, ("nickname", "expected string")),
        ({"tags": typing.cast(typing.Any, "ab")}, ("tags", "expected array")),
        ({"dates": typing.cast(typing.Any, [7])}, ("dates[0]", "expected date")),
        ({"name": typing.cast(typing.Any, None)}, ("name", "required")),
    ]:
        with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
            _ = converter.to_transfer_type(dataclasses.replace(full, **replacement))
        assert violation_pairs(excinfo.value) == [expected]

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter.to_transfer_type(
            dataclasses.replace(
                full,
                addresses=typing.cast(typing.Any, 7),
                dates=typing.cast(typing.Any, 7),
            )
        )
    assert violation_pairs(excinfo.value) == [
        ("addresses", "expected array"),
        ("dates", "expected array"),
    ]

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Address).to_transfer_type(typing.cast(typing.Any, 7))
    assert violation_pairs(excinfo.value) == [("", "expected object")]

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Attributes).to_transfer_type(
            Attributes(additional_properties={"host": typing.cast(typing.Any, [])})
        )
    assert violation_pairs(excinfo.value) == [("host", "expected string")]

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Attributes).to_transfer_type(
            Attributes(additional_properties=typing.cast(typing.Any, None))
        )
    assert violation_pairs(excinfo.value) == [("", "expected object")]

    # Object-level checks fire on serialize too.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Attributes).to_transfer_type(
            Attributes(additional_properties={})
        )
    assert violation_pairs(excinfo.value) == [
        ("", "must have at least 1 properties, got 0")
    ]

    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(ContactPy).to_transfer_type(
            ContactPy(shipping_street="1 Main St")
        )
    assert violation_pairs(excinfo.value) == [
        (
            "shippingZip",
            'property "shippingZip" is required when "shippingStreet" is present',
        )
    ]


def test_serialize_aggregates_nested_violations_under_their_own_paths() -> None:
    """P11/P12 on the serialize side: one model with independent failures at
    several depths reports **all** of them, each fully pathed.

    `to_transfer_type` wrapped no nested conversion, so the first nested
    `ApplicationError` propagated raw — discarding both the parent's already
    collected violations and its own path prefix. Every nested conversion now funnels
    through `_collect`, the analogue of Go's `mergeNested`.
    """
    model = dataclasses.replace(
        parse(BASE),
        # A flat member of the parent itself, which a raw nested error would discard.
        name="",
        # A union in an element position, reported at its index.
        segments=["ab", -1],
        # A `$ref` member (hoisted inline object), reported under the member.
        location=ShowcaseLocation(city="", geo=None),
        # An array of `$ref`, reported at index *and* member.
        rows=[ShowcaseRowsItem(cell="a1"), ShowcaseRowsItem(cell="")],
        # A typed map whose member type is a union, reported at key *and* member.
        choices=Choices(
            additional_properties={
                "a": Circle(kind=typing.cast(typing.Any, "nope"), radius=1.0)
            }
        ),
    )
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Showcase).to_transfer_type(model)
    assert violation_pairs(excinfo.value) == [
        ("name", "must have length >= 1, got 0"),
        ("segments[1]", "must be >= 0, got -1"),
        ("location.city", "must have length >= 1, got 0"),
        ("rows[1].cell", "must have length >= 1, got 0"),
        ("choices.a.kind", 'must equal "circle"'),
    ]

    # The nested shapes serialize cleanly when valid — the aggregation above is not
    # a blanket rejection of nesting.
    ok = dataclasses.replace(
        parse(BASE),
        location=ShowcaseLocation(city="Springfield", geo=None),
        rows=[ShowcaseRowsItem(cell="a1")],
    )
    assert encode_bytes(ok) == canonical_json_bytes(
        {**BASE, "location": {"city": "Springfield"}, "rows": [{"cell": "a1"}]}
    )


def test_serialize_rejects_a_value_matching_no_union_branch() -> None:
    """A union's serialize dispatch rejects a value in **no** branch (P12).

    The dispatch falls through to its last branch unguarded once a value has
    matched, which is deliberate; a value matching nothing used to be emitted
    verbatim — bytes every parser, including this one, rejects.
    """
    # A `float` in a `str | int` union: past static typing only through a cast, and
    # exactly what an untyped caller or a `typing.Any` boundary produces.
    assert serialize_violations(id_or_name=typing.cast(typing.Any, 1.5)) == [
        ("idOrName", "expected one of: string, integer")
    ]
    assert serialize_violations(mode=typing.cast(typing.Any, 1.5)) == [
        ("mode", "expected one of: string, integer")
    ]
    # The same on a union mixing object and scalar branches, which names all three.
    assert serialize_violations(shape_or_name=typing.cast(typing.Any, 1.5)) == [
        ("shapeOrName", "expected one of: Circle, Square, string")
    ]
    # ...and on one whose branches are an array and a string.
    assert serialize_violations(measurements=typing.cast(typing.Any, 1.5)) == [
        ("measurements", "expected one of: list[float], string")
    ]
    # A no-branch value aggregates with an unrelated failure rather than short-
    # circuiting it (P11).
    assert serialize_violations(name="", id_or_name=typing.cast(typing.Any, 1.5)) == [
        ("name", "must have length >= 1, got 0"),
        ("idOrName", "expected one of: string, integer"),
    ]
    # Each branch's own value still serializes.
    for value in ("abc", 7):
        assert encode_bytes(
            dataclasses.replace(parse(BASE), id_or_name=value)
        ) == canonical_json_bytes({**BASE, "idOrName": value})
