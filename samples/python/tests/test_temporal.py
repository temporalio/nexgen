import dataclasses
import datetime
import typing

import pytest
import temporalio.exceptions

from temporal import Temporal
from temporal._definitions import (
    _TEMPORAL_FRACTION_DIGITS,
    _TEMPORAL_MAX_DURATION_SECONDS,
    _temporal_isoformat,
)

from tests.json_converter_helper import (
    canonical_json_bytes,
    converter_for,
    decode_fixture,
    encode_bytes,
    load_fixture,
    roundtrip_fixture,
    violation_pairs,
)

SUITE = "temporal"

#: The four required members, so a negative payload reports only what is under test.
BASE: dict[str, typing.Any] = {
    "createdAt": "2021-06-15T12:30:45Z",
    "birthday": "2000-01-01",
    "alarm": "09:00:00",
    "timeout": "PT0S",
}


def decode(name: str) -> Temporal:
    return decode_fixture(Temporal, SUITE, name)


def parse(**overrides: typing.Any) -> Temporal:
    return converter_for(Temporal).from_transfer_type({**BASE, **overrides}, Temporal)


def parse_violations(**overrides: typing.Any) -> list[tuple[str, str]]:
    """The ``(path, reason)`` pairs one bad Temporal payload produces.

    Every value under test here used to escape as a bare ``ValueError`` from a
    ``datetime`` parser rather than as an aggregated ``ApplicationError`` (P11), so
    the assertion is as much that ``ApplicationError`` is what surfaces as it is
    about the reason text.
    """
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = parse(**overrides)
    return violation_pairs(excinfo.value)


def serialize_violations(**replacements: typing.Any) -> list[tuple[str, str]]:
    """The violations serializing a ``BASE`` model with ``replacements`` produces."""
    model = dataclasses.replace(parse(), **replacements)
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Temporal).to_transfer_type(model)
    return violation_pairs(excinfo.value)


def unrepresentable(format_name: str, value: object, detail: str) -> str:
    """The reason a materialized value the wire grammar cannot spell is reported
    under: the format, the offending value, and *why* it cannot be written."""
    return f'must be a valid {format_name}, got "{value}": {detail}'


def test_temporal_roundtrip_full() -> None:
    # Materialized temporals become native datetime/date/time/timedelta and
    # re-serialize (generator-owned) byte-identically for microsecond precision.
    model = roundtrip_fixture(Temporal, SUITE, "temporal-full.json")
    assert model.created_at.utcoffset() == datetime.timedelta(hours=2)
    assert model.created_at.microsecond == 123456
    assert model.timeout == datetime.timedelta(minutes=90)
    assert model.deleted_at is not None


def test_temporal_roundtrip_minimal() -> None:
    _ = roundtrip_fixture(Temporal, SUITE, "temporal-minimal.json")


def test_temporal_canonicalization() -> None:
    # Non-canonical input normalizes on re-serialize (uppercase T/Z, +00:00 -> Z,
    # PT90M -> PT1H30M). This is the one fixture whose re-emitted bytes differ from
    # its own by design, so its expectation is spelled out rather than derived
    # (`NON_CANONICAL_FIXTURES`).
    model = decode("temporal-canonicalize.json")
    assert encode_bytes(model) == canonical_json_bytes(
        {
            "createdAt": "2021-06-15T12:30:45Z",
            "birthday": "2021-02-28",
            "alarm": "12:30:45Z",
            "timeout": "PT1H30M",
        }
    )


def test_temporal_nulls_collapse_on_roundtrip() -> None:
    # `deletedAt`/`archivedOn` are optional+nullable. A dataclass has no presence
    # channel, so absent and explicit `null` are the same in-memory state (None)
    # and both re-serialize as OMITTED. Python now matches Go and Java here (see
    # samples/go/tests/json_schema_temporal_test.go, TestJSONSchemaTemporalNulls);
    # only TypeScript still preserves the explicit null.
    model = decode("temporal-nulls.json")
    assert model.deleted_at is None
    assert model.archived_on is None
    assert model.timeout == datetime.timedelta(0)

    wire = typing.cast(
        "dict[str, typing.Any]", load_fixture(SUITE, "temporal-nulls.json")
    )
    assert wire["deletedAt"] is None
    assert wire["archivedOn"] is None
    # The explicit nulls are gone from the re-encoded wire; every other byte survives.
    assert encode_bytes(model) == canonical_json_bytes(
        {
            key: value
            for key, value in wire.items()
            if key not in ("deletedAt", "archivedOn")
        }
    )


def test_temporal_absent_and_explicit_null_are_indistinguishable() -> None:
    # The collapse, stated directly: the two payloads produce equal models.
    assert parse() == parse(deletedAt=None, archivedOn=None)


def test_missing_required_members_aggregate() -> None:
    # P11: one bad payload surfaces every violation it contains, in declared
    # property order — the aggregation pydantic used to provide for free.
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Temporal).from_transfer_type({}, Temporal)
    assert violation_pairs(excinfo.value) == [
        ("createdAt", "required"),
        ("birthday", "required"),
        ("alarm", "required"),
        ("timeout", "required"),
    ]


def test_non_object_payload_is_a_single_structural_violation() -> None:
    with pytest.raises(temporalio.exceptions.ApplicationError) as excinfo:
        _ = converter_for(Temporal).from_transfer_type("nope", Temporal)
    assert violation_pairs(excinfo.value) == [("", "expected object")]


def test_unknown_member_is_rejected() -> None:
    assert parse_violations(nope=1) == [("nope", "unknown field")]


@pytest.mark.parametrize(
    "field,value,format_name",
    [
        ("createdAt", "2021-12-31T23:59:60Z", "date-time"),  # leap second
        ("timeout", "P1Y", "duration"),  # calendar duration
        ("birthday", "2021-02-29", "date"),  # invalid calendar date
        ("createdAt", "2021-06-15T12:30:45", "date-time"),  # missing offset
        ("createdAt", "2021-06-15T12:30:45+14:01", "date-time"),
        ("createdAt", "2021-06-15T12:30:45-18:00", "date-time"),
        ("createdAt", "2021-06-15T12:30:45-23:59", "date-time"),
        ("alarm", "09:00:00+14:01", "time"),
        ("alarm", "09:00:00+18:00", "time"),
        ("alarm", "09:00:00-23:59", "time"),
    ],
)
def test_temporal_materialized_narrowing_rejects(
    field: str, value: str, format_name: str
) -> None:
    # The reason names the format and the offending value, rendered in its JSON
    # form exactly as Go and TypeScript render it.
    assert parse_violations(**{field: value}) == [
        (field, f'must be a valid {format_name}, got "{value}"')
    ]


def test_year_zero_is_a_violation_rather_than_a_value_error() -> None:
    """`datetime.MINYEAR` is 1, now the shared cross-language calendar floor.

    Year 0000 is rejected as an aggregated `ApplicationError` (P11), and the reason
    names the limit rather than escaping as a native `ValueError`.
    """
    limit = f"year 0000 is not representable (datetime.MINYEAR is {datetime.MINYEAR})"
    assert parse_violations(createdAt="0000-01-01T00:00:00Z") == [
        ("createdAt", f'must be a valid date-time, got "0000-01-01T00:00:00Z": {limit}')
    ]
    assert parse_violations(birthday="0000-12-31") == [
        ("birthday", f'must be a valid date, got "0000-12-31": {limit}')
    ]
    # Aggregation still holds — the year-0000 reject is a violation like any other,
    # not an early exit.
    assert parse_violations(
        createdAt="0000-01-01T00:00:00Z", birthday="0000-01-01"
    ) == [
        (
            "createdAt",
            f'must be a valid date-time, got "0000-01-01T00:00:00Z": {limit}',
        ),
        ("birthday", f'must be a valid date, got "0000-01-01": {limit}'),
    ]
    # Year 0001 is the first representable year and is accepted.
    assert parse(createdAt="0001-01-01T00:00:00Z").created_at.year == 1


@pytest.mark.parametrize(
    "fraction,microsecond,emitted",
    [
        # RFC 3339 allows any number of fractional digits. `isoformat` writes only
        # 3 or 6, and before 3.11 `fromisoformat` parses only what `isoformat`
        # writes -- so every width below except `.123456` used to raise on the
        # declared 3.10 floor while every other target accepted it.
        (".1", 100000, ".1"),
        (".12", 120000, ".12"),
        (".123", 123000, ".123"),
        (".12345", 123450, ".12345"),
        (".123456", 123456, ".123456"),
        # Past `datetime`'s own microsecond resolution the extra digits are dropped
        # -- the bounded loss P1 exception (b) allows, mirroring Go's truncation at
        # nanoseconds -- rather than the value being rejected.
        (".1234567", 123456, ".123456"),
        (".1234567890", 123456, ".123456"),
        # A fraction of all zeros carries no sub-second component at all, so the
        # canonical form drops it.
        (".000", 0, ""),
    ],
)
def test_sub_second_precision_is_accepted_at_every_width(
    fraction: str, microsecond: int, emitted: str
) -> None:
    """Every fractional-second width the wire grammar admits parses, and re-emits
    canonically — `.1` still writes as `.1`, not as `.100000`."""
    model = parse(
        createdAt=f"2021-06-15T12:30:45{fraction}Z",
        alarm=f"09:00:00{fraction}Z",
    )
    assert model.created_at.microsecond == microsecond
    assert model.alarm.microsecond == microsecond
    assert encode_bytes(model) == canonical_json_bytes(
        {
            **BASE,
            "createdAt": f"2021-06-15T12:30:45{emitted}Z",
            "alarm": f"09:00:00{emitted}Z",
        }
    )


def test_temporal_isoformat_pads_the_fraction_to_datetime_resolution() -> None:
    """The interpreter-independent statement of the fix above.

    `test_sub_second_precision_is_accepted_at_every_width` only *fails* on an
    interpreter whose `fromisoformat` is picky about the fraction width — 3.10, the
    declared floor. From 3.11 on, `fromisoformat` accepts every width itself, so on
    a newer interpreter that test passes with or without the normalization and
    proves nothing. This asserts the normalization directly, so the guard holds on
    every interpreter: the fraction handed to `fromisoformat` is always exactly
    `_TEMPORAL_FRACTION_DIGITS` wide, which is the one width every supported
    version parses.
    """
    assert _TEMPORAL_FRACTION_DIGITS == 6
    for wire, normalized in [
        ("2021-06-15T12:30:45.1Z", "2021-06-15T12:30:45.100000+00:00"),
        ("2021-06-15T12:30:45.12Z", "2021-06-15T12:30:45.120000+00:00"),
        ("2021-06-15T12:30:45.1234567890Z", "2021-06-15T12:30:45.123456+00:00"),
        ("2021-06-15t12:30:45.1-05:00", "2021-06-15T12:30:45.100000-05:00"),
        # No fraction and no `Z` are both left alone.
        ("2021-06-15T12:30:45+02:00", "2021-06-15T12:30:45+02:00"),
    ]:
        assert _temporal_isoformat(wire) == normalized
        # The point of the padding: this is the spelling every supported
        # interpreter's `fromisoformat` accepts, 3.10 included.
        _ = datetime.datetime.fromisoformat(normalized)

    for wire, normalized in [
        ("09:00:00.1Z", "09:00:00.100000+00:00"),
        ("09:00:00.1234567890z", "09:00:00.123456+00:00"),
        ("09:00:00", "09:00:00"),
    ]:
        assert _temporal_isoformat(wire) == normalized
        _ = datetime.time.fromisoformat(normalized)


def test_oversized_duration_components_are_violations_rather_than_crashes() -> None:
    """CPython refuses `int()` on a string of more than 4300 digits, so a long
    numeric component crashed the parse. The magnitude is now bounded by digit count
    before any conversion is attempted."""
    huge = "9" * 5000
    assert parse_violations(timeout=f"PT{huge}S") == [
        ("timeout", f'must be a valid duration, got "PT{huge}S"')
    ]
    # Leading zeros are stripped before the digit count, matching TypeScript's
    # `Number()`, so a padded but in-range value is still accepted.
    assert parse(timeout="PT" + "0" * 5000 + "30S").timeout == datetime.timedelta(
        seconds=30
    )
    # The cap itself: one second over is rejected, the cap exactly is accepted.
    assert parse_violations(timeout=f"PT{_TEMPORAL_MAX_DURATION_SECONDS + 1}S") == [
        (
            "timeout",
            f'must be a valid duration, got "PT{_TEMPORAL_MAX_DURATION_SECONDS + 1}S"',
        )
    ]
    assert parse(
        timeout=f"PT{_TEMPORAL_MAX_DURATION_SECONDS}S"
    ).timeout == datetime.timedelta(seconds=_TEMPORAL_MAX_DURATION_SECONDS)
    # A multi-component duration overflows on the sum, not on one component.
    summed = f"PT{_TEMPORAL_MAX_DURATION_SECONDS // 3600}H59M59S"
    assert parse_violations(timeout=summed) == [
        ("timeout", f'must be a valid duration, got "{summed}"')
    ]


def test_serialize_rejects_temporal_values_the_wire_form_cannot_carry() -> None:
    """P12 on the serialize side: a dataclass is constructed unchecked, so a value
    the narrowed wire grammar cannot spell reaches serialize.

    Without these predicates the converter emitted wire bytes its own parser
    rejects — the exact asymmetry P12 exists to forbid. Each violation is reported
    under the field's own name, so a caller reads it like any other.
    """
    naive = datetime.datetime(2021, 6, 15, 12, 30, 45)
    negative = datetime.timedelta(seconds=-1)
    fractional = datetime.timedelta(milliseconds=500)
    over_cap = datetime.timedelta(seconds=_TEMPORAL_MAX_DURATION_SECONDS + 1)
    # A UTC offset finer than the minute the wire form spells would be silently lost.
    sub_minute = datetime.timezone(datetime.timedelta(seconds=30))
    offset_datetime = datetime.datetime(2021, 6, 15, 12, 30, 45, tzinfo=sub_minute)
    offset_time = datetime.time(9, 0, tzinfo=sub_minute)

    naive_reason = unrepresentable(
        "date-time", naive, "a naive datetime carries no UTC offset"
    )
    negative_reason = unrepresentable(
        "duration", negative, "a duration cannot be negative"
    )

    # A naive datetime has no offset the required wire form could carry.
    assert serialize_violations(created_at=naive) == [("createdAt", naive_reason)]
    # The wire duration grammar is unsigned...
    assert serialize_violations(timeout=negative) == [("timeout", negative_reason)]
    # ...whole-second...
    assert serialize_violations(timeout=fractional) == [
        (
            "timeout",
            unrepresentable(
                "duration", fractional, "a duration cannot carry a fraction of a second"
            ),
        )
    ]
    # ...and capped.
    assert serialize_violations(timeout=over_cap) == [
        (
            "timeout",
            unrepresentable(
                "duration",
                over_cap,
                f"a duration cannot exceed {_TEMPORAL_MAX_DURATION_SECONDS} seconds",
            ),
        )
    ]
    # The sub-minute offset, on a date-time and on a time alike.
    sub_minute_detail = "the UTC offset 0:00:30 is not a whole number of minutes"
    assert serialize_violations(created_at=offset_datetime) == [
        ("createdAt", unrepresentable("date-time", offset_datetime, sub_minute_detail))
    ]
    assert serialize_violations(alarm=offset_time) == [
        ("alarm", unrepresentable("time", offset_time, sub_minute_detail))
    ]

    # Both materialized clock formats use the same inclusive +/-14:00 domain.
    for seconds in (
        14 * 3600 + 60,
        -(14 * 3600 + 60),
        18 * 3600,
        -18 * 3600,
        23 * 3600 + 59 * 60,
        -(23 * 3600 + 59 * 60),
    ):
        zone = datetime.timezone(datetime.timedelta(seconds=seconds))
        for field, value in (
            ("created_at", datetime.datetime(2021, 6, 15, 12, 30, 45, tzinfo=zone)),
            ("alarm", datetime.time(9, 0, tzinfo=zone)),
        ):
            violations = serialize_violations(**{field: value})
            assert violations[0][0] == (
                "createdAt" if field == "created_at" else "alarm"
            )
            assert violations[0][1].endswith(
                "the UTC offset is outside -14:00 through +14:00"
            )

    for seconds, suffix in ((14 * 3600, "+14:00"), (-14 * 3600, "-14:00")):
        zone = datetime.timezone(datetime.timedelta(seconds=seconds))
        model = dataclasses.replace(
            parse(),
            created_at=datetime.datetime(2021, 6, 15, 12, 30, 45, tzinfo=zone),
            alarm=datetime.time(9, 0, tzinfo=zone),
        )
        wire = typing.cast(
            "dict[str, typing.Any]", converter_for(Temporal).to_transfer_type(model)
        )
        assert wire["createdAt"] == f"2021-06-15T12:30:45{suffix}"
        assert wire["alarm"] == f"09:00:00{suffix}"

    # Independent failures at different members aggregate into one error (P11).
    assert serialize_violations(created_at=naive, timeout=negative) == [
        ("createdAt", naive_reason),
        ("timeout", negative_reason),
    ]

    # A `date` needs no predicate: every `datetime.date` writes a valid wire date.
    assert encode_bytes(
        dataclasses.replace(parse(), birthday=datetime.date(1, 1, 1))
    ) == canonical_json_bytes({**BASE, "birthday": "0001-01-01"})
