//! The asserted string `format` subset (JSON Schema 2020-12 §7). Each supported
//! format lowers to a **generator-owned** check: a pinned, portable, RE2-safe
//! regex (proven compilable by the [[pattern]] gate, `crate::json_schema::pattern`) plus —
//! where a regex alone is insufficient — a shared **length guard**. The
//! generators emit the same regex the loader validates literals against, so a
//! value accepted (or rejected) by one language is accepted (or rejected) by all
//! (P1). See `specs/json-schema/features/format.md` for the authoritative rules and
//! the `specs/json-schema/corpora/format_{conformance,email,hostname,uri}/`
//! corpora that pinned every pattern and length below.
//!
//! Two families are asserted here. The **string-shaped** formats — `uuid`,
//! `ipv4`, `ipv6`, `hostname`, `email`, `uri`, `uri-reference` — keep a
//! `string` field and get the pinned regex (+ length guard). The **temporal**
//! formats — `date-time`, `date`, `time`, `duration` — are *materialized* into
//! a language-native typed field (Go `time.Time`, Java `OffsetDateTime`, Python
//! `datetime`, …) and asserted with a **narrowed** grammar (leap second `:60`
//! rejected; clock offsets limited to `-14:00..+14:00`; `duration` time-only;
//! calendar year floor 0001); their wire form is
//! produced by a generator-owned serializer, so a literal is canonicalized
//! through [`canonicalize`] rather than echoed verbatim. Every other standard
//! format is **deferred** (rejected "not yet supported"); anything else is an
//! **unknown** format (rejected with a fix-it listing the supported names).

/// The RFC 3986 dotted-quad octet (`0-255`, no leading zeros), shared by `ipv4`
/// and the IPv4-tail of `ipv6`. Pinned verbatim from
/// `specs/json-schema/corpora/format_conformance/`.
const OCTET: &str = "(25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9][0-9]|[0-9])";

/// An RFC 4291 IPv6 16-bit group (1-4 hex digits).
const H16: &str = "[0-9a-fA-F]{1,4}";

/// The **string-shaped** formats, in canonical order: each lowers to a pinned
/// regex (plus an optional length guard) over a field that stays a `string`.
/// This is the set [`check_for`] answers for — it is *not* the fix-it list,
/// which must also name the materialized temporal formats
/// ([`SUPPORTED_FORMATS`]).
pub const STRING_FORMATS: [&str; 7] = [
    "uuid",
    "ipv4",
    "ipv6",
    "hostname",
    "email",
    "uri",
    "uri-reference",
];

/// Every format the generator asserts, in canonical order — the string-shaped
/// set followed by the materialized temporal set. This is the list the
/// unknown-format fix-it prints, so it must stay complete: quoting only
/// [`STRING_FORMATS`] hid `date-time` from the user who typed `datetime`.
pub const SUPPORTED_FORMATS: [&str; 11] = [
    "uuid",
    "ipv4",
    "ipv6",
    "hostname",
    "email",
    "uri",
    "uri-reference",
    "date-time",
    "date",
    "time",
    "duration",
];

/// The RFC-3339 temporal formats. These are **materialized** as idiomatic
/// native typed model fields (Go `time.Time`, Java `OffsetDateTime`, Python
/// `datetime`, …) rather than a bare `string`, and asserted with a **narrowed**
/// grammar (leap second `:60` rejected; clock offsets limited to ±14 hours;
/// `duration` is time-only). See
/// `specs/json-schema/features/format.md` (Materialization) and `TemporalKind`.
pub const TEMPORAL_FORMATS: [&str; 4] = ["date-time", "date", "time", "duration"];

/// Whether every string accepted by `narrower` is also accepted by `wider`.
///
/// This is the format component's ownership table for intersections used by
/// `allOf`. Keep it beside the asserted format definitions: adding or changing
/// a format's accepted set must update this relation and its tests together,
/// rather than teaching the generic schema merger format semantics.
pub(crate) fn accepted_set_is_contained_by(narrower: &str, wider: &str) -> bool {
    matches!(
        (narrower, wider),
        ("uri", "uri-reference")
            | ("hostname", "uri-reference")
            | ("uuid", "hostname")
            | ("ipv4", "hostname")
            | ("date", "hostname")
            | ("duration", "uri-reference")
            | ("uuid", "uri-reference")
            | ("ipv4", "uri-reference")
            | ("date", "uri-reference")
    )
}

/// The four materialized temporal formats. Each carries a language-native typed
/// field; the wire is produced by re-serializing it through a generator-owned
/// serializer. The asserted grammar is the **narrowed** materialized grammar
/// (see the `materialized_pattern` docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalKind {
    /// RFC 3339 `date-time`, **offset required**, `:60` rejected.
    DateTime,
    /// RFC 3339 full-date `YYYY-MM-DD`.
    Date,
    /// RFC 3339 `partial-time` with optional offset, `:60` rejected.
    Time,
    /// ISO 8601 duration narrowed to **time-only** `PT…H…M…S`.
    Duration,
}

impl TemporalKind {
    /// The `TemporalKind` for a format name, or `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "date-time" => Some(Self::DateTime),
            "date" => Some(Self::Date),
            "time" => Some(Self::Time),
            "duration" => Some(Self::Duration),
            _ => None,
        }
    }

    /// The canonical format name.
    pub fn name(self) -> &'static str {
        match self {
            Self::DateTime => "date-time",
            Self::Date => "date",
            Self::Time => "time",
            Self::Duration => "duration",
        }
    }

    /// The pinned, anchored (`^…$`) **materialized** regex for this kind — the
    /// narrowed grammar (leap second `:60` excluded by the `[0-5][0-9]` seconds
    /// group; `date-time` offset required; materialized offsets limited to
    /// `-14:00..+14:00`; `duration` time-only). `T`/`Z`
    /// separators are accepted in either case. Emitted (with the per-target
    /// end-anchor rewrite) into each generator's parse adapter, where the wire
    /// string's `:60` / offset / precision are still observable.
    pub fn pattern(self) -> &'static str {
        materialized_pattern(self)
    }
}

/// The pinned materialized regex for a temporal kind. See `TemporalKind::pattern`.
///
/// **Fractional seconds are deliberately unbounded** (`(\.[0-9]+)?`). RFC 3339
/// places no limit on the number of digits, and the contract is *accept every
/// width and truncate to each target's genuine capacity* — Python `datetime` to
/// microseconds, Go / Java / TS `Temporal` to nanoseconds — which is exactly the
/// bounded loss P1 exception (b) permits, since each target truncates at **its
/// own** capacity limit. `samples/python/tests/test_temporal.py:201-227`
/// (`test_sub_second_precision_is_accepted_at_every_width`) pins this with an
/// explicit ten-digit case and spells the reasoning out.
///
/// Do **not** narrow this to `{1,9}` to make a target that rejects long
/// fractions agree: a uniform nine-digit cap satisfies exception (b) for *no*
/// target at ten or more digits (nine is not Python's limit, and the others can
/// simply truncate), and it silently turns an accepted value into a load
/// rejection. When a target diverges here, the target is what to fix — see
/// `new:java-rejects-12-digit-fraction`, closed by teaching Java to truncate
/// like the other three.
pub fn materialized_pattern(kind: TemporalKind) -> &'static str {
    match kind {
        TemporalKind::Date => "^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])$",
        TemporalKind::Time => {
            "^([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]+)?([Zz]|[+-]((0[0-9]|1[0-3]):[0-5][0-9]|14:00))?$"
        }
        TemporalKind::DateTime => {
            "^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])[Tt]([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9](\\.[0-9]+)?([Zz]|[+-]((0[0-9]|1[0-3]):[0-5][0-9]|14:00))$"
        }
        TemporalKind::Duration => {
            "^PT(?:[0-9]+H(?:[0-9]+M(?:[0-9]+S)?)?|[0-9]+M(?:[0-9]+S)?|[0-9]+S)$"
        }
    }
}

/// The Gregorian leap-year rule.
pub fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days in a (1-based) month of `year`, or `None` for an out-of-range month.
pub fn days_in_month(year: i64, month: u32) -> Option<u32> {
    Some(match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => return None,
    })
}

/// The `time.Duration` (Go int64-nanosecond) capacity — the uniform cross-language
/// overflow cap for a materialized `duration` (P1: identical accept/reject in
/// every target). A time-only duration whose total nanoseconds exceed this is a
/// load/parse `Violation`.
pub const MAX_DURATION_NANOS: i128 = i64::MAX as i128;

/// The date/date-time calendar predicate over `YYYY-MM-DD…`: month `01–12` and
/// day within the month's Gregorian length. The regex has already guaranteed the
/// digit shape and the month/day *ranges*; this adds day-in-month + leap-year.
fn valid_calendar_prefix(value: &str) -> bool {
    // value starts with `YYYY-MM-DD`; slice fixed ASCII positions.
    let bytes = value.as_bytes();
    if bytes.len() < 10 {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        value[0..4].parse::<i64>(),
        value[5..7].parse::<u32>(),
        value[8..10].parse::<u32>(),
    ) else {
        return false;
    };
    // Year zero cannot be represented by every target's materialized native
    // date type (notably Python's datetime), so the shared wire contract starts
    // at Gregorian year 0001.
    if year < 1 {
        return false;
    }
    match days_in_month(year, month) {
        Some(max) => (1..=max).contains(&day),
        None => false,
    }
}

/// The total-nanoseconds of a materialized (time-only) `duration`, or `None` if
/// it overflows the uniform `MAX_DURATION_NANOS` cap. The input must already have
/// matched the materialized duration regex.
pub fn duration_total_nanos(value: &str) -> Option<i128> {
    // Strip leading "PT".
    let body = value.strip_prefix("PT")?;
    let mut total_seconds: i128 = 0;
    let mut number = String::new();
    for ch in body.chars() {
        if ch.is_ascii_digit() {
            number.push(ch);
            continue;
        }
        let magnitude: i128 = number.parse().ok()?;
        number.clear();
        let unit_seconds: i128 = match ch {
            'H' => 3600,
            'M' => 60,
            'S' => 1,
            _ => return None,
        };
        total_seconds = total_seconds.checked_add(magnitude.checked_mul(unit_seconds)?)?;
        if total_seconds > MAX_DURATION_NANOS {
            return None;
        }
    }
    let total_nanos = total_seconds.checked_mul(1_000_000_000)?;
    if total_nanos > MAX_DURATION_NANOS {
        return None;
    }
    Some(total_nanos)
}

/// The **canonical wire string** for a materialized temporal value — the exact
/// bytes the generator's own serializer produces in every target, and therefore
/// the only form a `const`/`enum`/`default` literal may be compared against
/// (decision **D10**: `uniqueItems` and `const`/`enum` over materialized values
/// compare the canonical wire string, in both directions, in all four
/// languages).
///
/// Returns `None` when `value` is not valid under the materialized grammar for
/// `kind` (same verdict as [`is_valid_materialized`]), so a caller can validate
/// and canonicalize in one step.
///
/// The rules, per `specs/json-schema/features/format.md` ("Serialized form"):
/// - `date` — already canonical (the pinned regex fixes every field width).
/// - `time` / `date-time` — the RFC 3339 §5.6 case-insensitive `t`/`z`
///   separators are **uppercased**, the zero offsets `+00:00` / `-00:00` fold to
///   `Z`, and fractional seconds are kept at the value's own precision with
///   trailing zeros trimmed (no fractional part at all when it is zero).
/// - `duration` — value-preserving non-canonical inputs are recomposed
///   (`PT90M` → `PT1H30M`, `PT3600S` → `PT1H`).
///
/// Examples: `2021-06-15t12:30:45z` → `2021-06-15T12:30:45Z`,
/// `2021-06-15T12:30:45.120+00:00` → `2021-06-15T12:30:45.12Z`,
/// `12:30:45.000-00:00` → `12:30:45Z`.
pub fn canonicalize(kind: TemporalKind, value: &str) -> Option<String> {
    if !is_valid_materialized(kind, value) {
        return None;
    }
    Some(match kind {
        TemporalKind::Date => value.to_string(),
        TemporalKind::Time | TemporalKind::DateTime => canonicalize_clock(value),
        TemporalKind::Duration => canonicalize_duration(value)?,
    })
}

/// [`canonicalize`] keyed by the `format` name. Returns `None` for a format that
/// is not a materialized temporal (there is nothing to canonicalize — a
/// string-shaped format's literal is already its own wire form) as well as for
/// an invalid value; callers that want "the canonical wire string, or the
/// literal unchanged" write
/// `canonicalize_for_format(name, v).unwrap_or_else(|| v.to_string())`.
pub fn canonicalize_for_format(format: &str, value: &str) -> Option<String> {
    TemporalKind::from_name(format).and_then(|kind| canonicalize(kind, value))
}

/// The `time` / `date-time` half of [`canonicalize`]. The input has already
/// matched the pinned materialized regex, so the only letters it can contain are
/// the `t`/`T` separator and the `z`/`Z` offset, and any offset is exactly
/// `±HH:MM`.
fn canonicalize_clock(value: &str) -> String {
    // 1. Split the offset off the end and fold the zero offsets to `Z`.
    let (body, offset) =
        if let Some(body) = value.strip_suffix('Z').or_else(|| value.strip_suffix('z')) {
            (body, "Z")
        } else if value.len() >= 6 && value.is_char_boundary(value.len() - 6) {
            let tail = &value[value.len() - 6..];
            let bytes = tail.as_bytes();
            if (bytes[0] == b'+' || bytes[0] == b'-') && bytes[3] == b':' {
                let folded = if &tail[1..] == "00:00" { "Z" } else { tail };
                (&value[..value.len() - 6], folded)
            } else {
                (value, "")
            }
        } else {
            (value, "")
        };

    // 2. Trim trailing zeros from the fractional seconds, dropping an all-zero
    //    fraction entirely.
    let body = match body.split_once('.') {
        Some((whole, fraction)) => {
            let trimmed = fraction.trim_end_matches('0');
            if trimmed.is_empty() {
                whole.to_string()
            } else {
                format!("{whole}.{trimmed}")
            }
        }
        None => body.to_string(),
    };

    // 3. Uppercase the `date-time` separator (the grammar has no other letters).
    format!("{}{offset}", body.replace('t', "T"))
}

/// The generator-owned canonical serialization of a materialized (time-only)
/// `duration`: decompose the total seconds into `PT…H…M…S`, omitting zero
/// components; the whole-zero duration is `PT0S`. Non-canonical inputs
/// canonicalize (`PT90M` → `PT1H30M`, `PT3600S` → `PT1H`). Returns `None` on
/// overflow. Prefer [`canonicalize`], which covers all four temporal kinds.
pub fn canonicalize_duration(value: &str) -> Option<String> {
    let total_nanos = duration_total_nanos(value)?;
    let total_seconds = (total_nanos / 1_000_000_000) as i128;
    Some(format_duration_seconds(total_seconds))
}

/// Formats a whole-second total into the canonical `PT…H…M…S` form.
pub fn format_duration_seconds(total_seconds: i128) -> String {
    if total_seconds == 0 {
        return "PT0S".to_string();
    }
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    let mut out = String::from("PT");
    if hours != 0 {
        out.push_str(&format!("{hours}H"));
    }
    if minutes != 0 {
        out.push_str(&format!("{minutes}M"));
    }
    if seconds != 0 {
        out.push_str(&format!("{seconds}S"));
    }
    out
}

/// The runtime-equivalent verdict for a value under a **materialized** temporal
/// format: the pinned narrowed regex plus the calendar predicate (`date` /
/// `date-time`) or the overflow guard (`duration`). Used at load to validate
/// `const`/`default`/`enum` literals and as the oracle for regression tests.
pub fn is_valid_materialized(kind: TemporalKind, value: &str) -> bool {
    let matched = regex::Regex::new(materialized_pattern(kind))
        .expect("pinned materialized pattern compiles")
        .is_match(value);
    if !matched {
        return false;
    }
    match kind {
        TemporalKind::Date | TemporalKind::DateTime => valid_calendar_prefix(value),
        TemporalKind::Time => true,
        TemporalKind::Duration => duration_total_nanos(value).is_some(),
    }
}

/// Standard 2020-12 formats we do not (yet) assert: they need IDNA/Unicode
/// handling or are niche. Rejected at load as "not yet supported" (distinct from
/// an unknown/typo format).
pub const DEFERRED_FORMATS: [&str; 8] = [
    "idn-email",
    "idn-hostname",
    "iri",
    "iri-reference",
    "uri-template",
    "json-pointer",
    "relative-json-pointer",
    "regex",
];

/// The pinned, generator-owned check for a supported string format: the anchored
/// (`^…$`) RE2-safe regex plus an optional total-length guard (in Unicode code
/// points). The regex is emitted verbatim to Go/JS and with the [[pattern]]
/// per-target end-anchor rewrite (`$`→`\Z`/`\z`) to Python/Java.
#[derive(Debug, Clone)]
pub struct FormatCheck {
    /// The canonical format name (for the `must be a valid <name>` reason).
    pub name: &'static str,
    /// The anchored `^…$` pinned regex (pre per-target end-anchor rewrite).
    pub pattern: String,
    /// Total-length guard in code points, run **before** the regex where present
    /// (`hostname` ≤253, `email` ≤254; the email order neutralizes a Java
    /// matcher StackOverflow hazard on adversarial inputs).
    pub max_code_points: Option<usize>,
}

/// The load-time classification of a `format` value.
pub enum FormatClass {
    /// An asserted string format lowering to a pinned check.
    Supported(FormatCheck),
    /// A recognized RFC-3339 temporal format, **materialized** into a native
    /// typed field with the narrowed grammar.
    Temporal(TemporalKind),
    /// A recognized-but-deferred standard format.
    Deferred,
    /// An unrecognized / non-standard format name.
    Unknown,
}

fn ipv6_body() -> String {
    let v4 = format!("({OCTET}\\.{OCTET}\\.{OCTET}\\.{OCTET})");
    let ls32 = format!("({H16}:{H16}|{v4})");
    format!(
        "({H16}:){{6}}{ls32}|\
         ::({H16}:){{5}}{ls32}|\
         ({H16})?::({H16}:){{4}}{ls32}|\
         (({H16}:){{0,1}}{H16})?::({H16}:){{3}}{ls32}|\
         (({H16}:){{0,2}}{H16})?::({H16}:){{2}}{ls32}|\
         (({H16}:){{0,3}}{H16})?::({H16}:){ls32}|\
         (({H16}:){{0,4}}{H16})?::{ls32}|\
         (({H16}:){{0,5}}{H16})?::{H16}|\
         (({H16}:){{0,6}}{H16})?::"
    )
}

/// The pinned URI (`uri`, scheme required) / URI-reference RFC 3986 ASCII body,
/// pinned verbatim from
/// `specs/json-schema/corpora/format_uri/pinned_body{,_uriref}.body`. The
/// IP-literal host `[…]` splices the `ipv6` grammar so `http://[1::2::3]`
/// rejects. Wrapped in `^(?:…)$` for anchored matching.
fn uri_body(reference: bool) -> &'static str {
    if reference {
        include_str!("../../specs/json-schema/corpora/format_uri/pinned_body_uriref.body")
    } else {
        include_str!("../../specs/json-schema/corpora/format_uri/pinned_body.body")
    }
}

/// Returns the pinned check for a supported string format, or `None`.
pub fn check_for(name: &str) -> Option<FormatCheck> {
    let anchored = |body: String| format!("^(?:{body})$");
    match name {
        "uuid" => Some(FormatCheck {
            name: "uuid",
            pattern:
                "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
                    .to_string(),
            max_code_points: None,
        }),
        "ipv4" => Some(FormatCheck {
            name: "ipv4",
            pattern: format!("^{OCTET}\\.{OCTET}\\.{OCTET}\\.{OCTET}$"),
            max_code_points: None,
        }),
        "ipv6" => Some(FormatCheck {
            name: "ipv6",
            pattern: anchored(ipv6_body()),
            max_code_points: None,
        }),
        "hostname" => Some(FormatCheck {
            name: "hostname",
            pattern: "^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*$"
                .to_string(),
            max_code_points: Some(253),
        }),
        "email" => Some(FormatCheck {
            name: "email",
            pattern: "^[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+(?:\\.[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+)*@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+$"
                .to_string(),
            max_code_points: Some(254),
        }),
        "uri" => Some(FormatCheck {
            name: "uri",
            pattern: anchored(uri_body(false).trim().to_string()),
            max_code_points: None,
        }),
        "uri-reference" => Some(FormatCheck {
            name: "uri-reference",
            pattern: anchored(uri_body(true).trim().to_string()),
            max_code_points: None,
        }),
        _ => None,
    }
}

/// Classifies a `format` name for the load gate.
pub fn classify(name: &str) -> FormatClass {
    if let Some(check) = check_for(name) {
        FormatClass::Supported(check)
    } else if let Some(kind) = TemporalKind::from_name(name) {
        FormatClass::Temporal(kind)
    } else if DEFERRED_FORMATS.contains(&name) {
        FormatClass::Deferred
    } else {
        FormatClass::Unknown
    }
}

/// Runtime-equivalent verdict for a value under a supported format: the shared
/// predicate the generators emit (length guard first, then the pinned regex).
/// Used at load to validate `const`/`default`/`enum` literals and as the oracle
/// for the corpus regression tests. Returns `true` for any non-supported format
/// (the load gate rejects those separately).
pub fn is_valid(name: &str, value: &str) -> bool {
    if let Some(kind) = TemporalKind::from_name(name) {
        return is_valid_materialized(kind, value);
    }
    let Some(check) = check_for(name) else {
        return true;
    };
    if let Some(max) = check.max_code_points
        && value.chars().count() > max
    {
        return false;
    }
    // The load gate proves the pinned pattern compiles; recompiling here (load
    // path only) is fine.
    regex::Regex::new(&check.pattern)
        .expect("pinned format pattern compiles")
        .is_match(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_table_is_directional() {
        assert!(accepted_set_is_contained_by("uuid", "hostname"));
        assert!(accepted_set_is_contained_by("uuid", "uri-reference"));
        assert!(!accepted_set_is_contained_by("hostname", "uuid"));
        assert!(!accepted_set_is_contained_by("email", "uri-reference"));
    }

    #[test]
    fn pinned_patterns_pass_the_pattern_gate() {
        for name in STRING_FORMATS {
            let check = check_for(name).expect("supported");
            crate::json_schema::pattern::gate_and_normalize(&check.pattern).unwrap_or_else(
                |error| panic!("{name} pinned pattern rejected by gate: {error:?}"),
            );
        }
        // The materialized temporal grammars are emitted through the same gate.
        for name in TEMPORAL_FORMATS {
            let kind = TemporalKind::from_name(name).expect("temporal");
            crate::json_schema::pattern::gate_and_normalize(kind.pattern()).unwrap_or_else(
                |error| panic!("{name} pinned pattern rejected by gate: {error:?}"),
            );
        }
    }

    /// `09#10`: the unknown-format fix-it prints `SUPPORTED_FORMATS`, so the
    /// constant must name every format the loader actually accepts — otherwise
    /// `format: datetime` is rejected with a list that hides `date-time`.
    #[test]
    fn supported_formats_names_every_accepted_format() {
        for name in STRING_FORMATS {
            assert!(SUPPORTED_FORMATS.contains(&name), "{name}");
        }
        for name in TEMPORAL_FORMATS {
            assert!(SUPPORTED_FORMATS.contains(&name), "{name}");
        }
        for name in SUPPORTED_FORMATS {
            assert!(
                !matches!(classify(name), FormatClass::Unknown | FormatClass::Deferred),
                "{name} is advertised but not accepted"
            );
        }
    }

    #[test]
    fn uuid_accepts_canonical_rejects_malformed() {
        assert!(is_valid("uuid", "de305d54-75b4-431b-adb2-eb6b9e546013"));
        assert!(!is_valid("uuid", "not-a-uuid"));
        assert!(!is_valid("uuid", "de305d54-75b4-431b-adb2-eb6b9e54601")); // 11 in last
    }

    #[test]
    fn ipv4_rejects_out_of_range_and_leading_zero() {
        assert!(is_valid("ipv4", "192.168.0.1"));
        assert!(!is_valid("ipv4", "256.0.0.1"));
        assert!(!is_valid("ipv4", "01.2.3.4"));
    }

    #[test]
    fn ipv6_rejects_double_colon() {
        assert!(is_valid("ipv6", "2001:db8::1"));
        assert!(is_valid("ipv6", "::1"));
        assert!(is_valid("ipv6", "::ffff:192.0.2.1"));
        assert!(!is_valid("ipv6", "1::2::3"));
        assert!(!is_valid("ipv6", "zzzz"));
    }

    #[test]
    fn email_rejects_single_label_domain_and_enforces_length() {
        assert!(is_valid("email", "user@example.com"));
        assert!(!is_valid("email", "user@localhost"));
        let huge = format!("{}@example.com", "a".repeat(300));
        assert!(!is_valid("email", &huge));
    }

    #[test]
    fn hostname_enforces_length_and_rejects_trailing_dot() {
        assert!(is_valid("hostname", "example.com"));
        assert!(!is_valid("hostname", "example.com."));
        let long_label = "a".repeat(64);
        assert!(!is_valid("hostname", &long_label));
        let over = (0..64).map(|_| "abc").collect::<Vec<_>>().join(".");
        assert!(over.chars().count() > 253);
        assert!(!is_valid("hostname", &over));
    }

    #[test]
    fn uri_splices_ipv6_and_requires_scheme() {
        assert!(is_valid("uri", "https://example.com/path?q=1#frag"));
        assert!(is_valid("uri", "http://[2001:db8::1]/"));
        assert!(is_valid("uri", "http://[::1]"));
        assert!(!is_valid("uri", "http://[1::2::3]"));
        assert!(!is_valid("uri", "//example.com/no-scheme"));
        assert!(!is_valid("uri", "http://example.com/%2"));
    }

    #[test]
    fn uri_reference_allows_relative() {
        assert!(is_valid("uri-reference", "/relative/path"));
        assert!(is_valid("uri-reference", "https://example.com"));
        assert!(!is_valid("uri-reference", "http://[1::2::3]"));
    }

    #[test]
    fn classify_partitions_names() {
        assert!(matches!(classify("uuid"), FormatClass::Supported(_)));
        assert!(matches!(
            classify("date-time"),
            FormatClass::Temporal(TemporalKind::DateTime)
        ));
        assert!(matches!(
            classify("duration"),
            FormatClass::Temporal(TemporalKind::Duration)
        ));
        assert!(matches!(classify("iri"), FormatClass::Deferred));
        assert!(matches!(classify("phone"), FormatClass::Unknown));
        assert!(matches!(classify("datetime"), FormatClass::Unknown));
    }

    /// The materialized temporal grammar agrees with `format_conformance`
    /// row for row. There is no exception list: every invalid temporal row,
    /// including leap seconds and offsets outside the materialized domain,
    /// declares `expect_valid: false`, so the assertion reads the corpus
    /// verbatim.
    #[test]
    fn materialized_temporal_matches_the_conformance_corpus() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/format_conformance/corpus.json"
        ))
        .expect("corpus parses");
        for pair in corpus["pairs"].as_array().expect("pairs") {
            let format = pair["format"].as_str().unwrap_or_default();
            let Some(kind) = TemporalKind::from_name(format) else {
                continue;
            };
            let value = pair["value"].as_str().expect("value string");
            let expect = pair["expect_valid"].as_bool().expect("expect_valid");
            assert_eq!(
                is_valid_materialized(kind, value),
                expect,
                "materialized {format} {value:?}"
            );
        }
    }

    #[test]
    fn materialized_duration_time_only_and_canonicalizes() {
        // Time-only accepted.
        assert!(is_valid_materialized(TemporalKind::Duration, "PT1H30M15S"));
        assert!(is_valid_materialized(TemporalKind::Duration, "PT0S"));
        assert!(is_valid_materialized(TemporalKind::Duration, "PT90M"));
        // Calendar components rejected (narrowed).
        assert!(!is_valid_materialized(TemporalKind::Duration, "P1Y"));
        assert!(!is_valid_materialized(TemporalKind::Duration, "P4W"));
        assert!(!is_valid_materialized(TemporalKind::Duration, "P1D"));
        assert!(!is_valid_materialized(TemporalKind::Duration, "P1YT1H"));
        // Canonicalization.
        assert_eq!(canonicalize_duration("PT90M").as_deref(), Some("PT1H30M"));
        assert_eq!(canonicalize_duration("PT3600S").as_deref(), Some("PT1H"));
        assert_eq!(
            canonicalize_duration("PT1H30M15S").as_deref(),
            Some("PT1H30M15S")
        );
        assert_eq!(canonicalize_duration("PT0S").as_deref(), Some("PT0S"));
        // Overflow.
        assert!(!is_valid_materialized(
            TemporalKind::Duration,
            "PT999999999999H"
        ));
    }

    /// `09#2` / `09#7` / decision **D10**: one canonicalization entry point
    /// covering all four temporal kinds, so the loader can store the literal in
    /// its serialized form and every emitter can compare the same wire string.
    #[test]
    fn canonicalize_covers_every_temporal_kind() {
        use TemporalKind::*;
        let cases = [
            // `date` is already canonical.
            (Date, "2021-06-15", "2021-06-15"),
            // Case folding (RFC 3339 §5.6 accepts lowercase `t`/`z`).
            (DateTime, "2021-06-15t12:30:45z", "2021-06-15T12:30:45Z"),
            (Time, "12:30:45z", "12:30:45Z"),
            // Zero offsets fold to `Z`; a real offset is preserved.
            (
                DateTime,
                "2021-06-15T12:30:45+00:00",
                "2021-06-15T12:30:45Z",
            ),
            (
                DateTime,
                "2021-06-15T12:30:45-00:00",
                "2021-06-15T12:30:45Z",
            ),
            (
                DateTime,
                "2021-06-15T12:30:45-05:00",
                "2021-06-15T12:30:45-05:00",
            ),
            (Time, "12:30:45+00:00", "12:30:45Z"),
            (Time, "12:30:45-00:00", "12:30:45Z"),
            (Time, "12:30:45+02:00", "12:30:45+02:00"),
            // Fractional seconds keep their own precision, trailing zeros
            // trimmed; an all-zero fraction disappears.
            (
                DateTime,
                "2021-06-15T12:30:45.120Z",
                "2021-06-15T12:30:45.12Z",
            ),
            (DateTime, "2021-06-15T12:30:45.000Z", "2021-06-15T12:30:45Z"),
            (
                DateTime,
                "2021-06-15t12:30:45.5-03:00",
                "2021-06-15T12:30:45.5-03:00",
            ),
            (Time, "12:30:45.250", "12:30:45.25"),
            (Time, "12:30:45.000-00:00", "12:30:45Z"),
            (Time, "12:30:45", "12:30:45"),
            // `duration` recomposes.
            (Duration, "PT90M", "PT1H30M"),
            (Duration, "PT3600S", "PT1H"),
            (Duration, "PT0S", "PT0S"),
        ];
        for (kind, input, expected) in cases {
            assert_eq!(
                canonicalize(kind, input).as_deref(),
                Some(expected),
                "canonicalize({kind:?}, {input:?})"
            );
            // Canonicalization is idempotent and its output is itself valid.
            assert!(is_valid_materialized(kind, expected), "{expected:?}");
            assert_eq!(canonicalize(kind, expected).as_deref(), Some(expected));
        }
        // An invalid value canonicalizes to `None` rather than to garbage.
        assert_eq!(canonicalize(TemporalKind::Date, "2021-02-30"), None);
        assert_eq!(canonicalize(TemporalKind::DateTime, "2021-06-15"), None);
        assert_eq!(canonicalize(TemporalKind::Duration, "P1D"), None);
        // The by-name wrapper only answers for the materialized temporals.
        assert_eq!(
            canonicalize_for_format("date-time", "2021-06-15t12:30:45z").as_deref(),
            Some("2021-06-15T12:30:45Z")
        );
        assert_eq!(canonicalize_for_format("uuid", "not-a-uuid"), None);
    }

    /// A `format_materialize_clock` row is a valid, round-trippable wire unless
    /// it explicitly declares otherwise. Keeping the exception in the data — not
    /// in a `wire.contains(":60")` guard here — is what lets the cross-runtime
    /// harness and this test read one source of truth.
    fn clock_row_is_valid(row: &serde_json::Value) -> bool {
        row["expect_valid"].as_bool().unwrap_or(true)
    }

    /// Every wire form in the clock corpus canonicalizes to something the
    /// materialized grammar still accepts, and canonicalization is a fixed point.
    #[test]
    fn canonicalize_is_idempotent_over_the_clock_corpus() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/format_materialize_clock/corpus.json"
        ))
        .expect("corpus parses");
        for (key, kind) in [
            ("date-time", TemporalKind::DateTime),
            ("date", TemporalKind::Date),
            ("time", TemporalKind::Time),
        ] {
            for row in corpus[key].as_array().expect("rows") {
                let wire = row["wire"].as_str().expect("wire");
                let Some(canonical) = canonicalize(kind, wire) else {
                    assert!(
                        !clock_row_is_valid(row),
                        "{key} {wire:?} is declared valid but does not canonicalize"
                    );
                    continue;
                };
                assert_eq!(
                    canonicalize(kind, &canonical).as_deref(),
                    Some(canonical.as_str()),
                    "{key} {wire:?} is not a canonicalization fixed point"
                );
                assert!(!canonical.contains('t') && !canonical.contains('z'));
                assert!(!canonical.ends_with("+00:00") && !canonical.ends_with("-00:00"));
            }
        }
    }

    /// The fractional-seconds contract is **accept every width**, not a cap.
    /// RFC 3339 sets no limit and each target truncates at its own genuine
    /// capacity (Python microseconds, Go / Java / TS nanoseconds) — the bounded
    /// loss P1 exception (b) allows. `samples/python/tests/test_temporal.py`
    /// (`test_sub_second_precision_is_accepted_at_every_width`) pins the runtime
    /// half with an explicit ten-digit case; this pins the grammar half, so
    /// narrowing the pattern to `{1,9}` to make a rejecting target agree fails
    /// here rather than in a sample suite.
    #[test]
    fn materialized_fraction_is_accepted_at_every_width() {
        for kind in [TemporalKind::DateTime, TemporalKind::Time] {
            let render = |fraction: &str| match kind {
                TemporalKind::DateTime => format!("2021-01-15T12:30:45{fraction}Z"),
                _ => format!("12:30:45{fraction}Z"),
            };
            // No fraction at all.
            assert!(is_valid_materialized(kind, &render("")), "{kind:?} bare");
            // Every width, including past every target's native resolution:
            // 7+ exceeds Python's microseconds, 10+ exceeds nanoseconds. Both
            // are accepted and truncated per target, never rejected.
            for digits in [1, 2, 3, 6, 7, 9, 10, 12, 20] {
                let fraction = format!(".{}", "1".repeat(digits));
                assert!(
                    is_valid_materialized(kind, &render(&fraction)),
                    "{kind:?} {digits} fractional digits must be accepted \
                     (the contract is truncate-per-target, not reject)"
                );
            }
            // A lone `.` carries no digits and is not a fraction.
            assert!(!is_valid_materialized(kind, &render(".")));
        }
        // The corpus row that pins the beyond-nanosecond case.
        assert!(is_valid_materialized(
            TemporalKind::DateTime,
            "2021-01-15T12:30:45.123456789012Z"
        ));
    }

    #[test]
    fn materialized_calendar_starts_at_year_one() {
        assert!(is_valid_materialized(TemporalKind::Date, "0001-01-01"));
        assert!(!is_valid_materialized(TemporalKind::Date, "0000-01-01"));
        assert!(is_valid_materialized(
            TemporalKind::DateTime,
            "0001-01-01T00:00:00Z"
        ));
        assert!(!is_valid_materialized(
            TemporalKind::DateTime,
            "0000-01-01T00:00:00Z"
        ));
    }

    #[test]
    fn materialized_offsets_are_limited_to_fourteen_hours() {
        for kind in [TemporalKind::DateTime, TemporalKind::Time] {
            let render = |offset: &str| match kind {
                TemporalKind::DateTime => format!("2021-01-15T12:30:45{offset}"),
                _ => format!("12:30:45{offset}"),
            };
            for offset in ["+14:00", "-14:00", "+13:59", "-13:59"] {
                assert!(
                    is_valid_materialized(kind, &render(offset)),
                    "{kind:?} must accept {offset}"
                );
            }
            for offset in [
                "+14:01", "-14:01", "+15:00", "-15:00", "+18:00", "-18:00", "+23:59", "-23:59",
            ] {
                assert!(
                    !is_valid_materialized(kind, &render(offset)),
                    "{kind:?} must reject {offset}"
                );
            }
        }
    }

    #[test]
    fn materialized_clock_roundtrip_values_are_valid() {
        // Every wire in the clock corpus must pass the materialized check so the
        // parse adapter never rejects a round-trip case — unless the row declares
        // `expect_valid: false`, for example for leap seconds or offsets outside
        // the materialized domain. An absent field means valid.
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/format_materialize_clock/corpus.json"
        ))
        .expect("corpus parses");
        for (key, kind) in [
            ("date-time", TemporalKind::DateTime),
            ("date", TemporalKind::Date),
            ("time", TemporalKind::Time),
        ] {
            for row in corpus[key].as_array().expect("rows") {
                let wire = row["wire"].as_str().expect("wire");
                let expect = clock_row_is_valid(row);
                assert_eq!(
                    is_valid_materialized(kind, wire),
                    expect,
                    "clock {key} {wire:?}"
                );
            }
        }
    }

    /// Drive the conformance corpora through the shared predicate as a
    /// regression against every corpus accept/reject pair.
    #[test]
    fn conformance_corpora_agree() {
        // format_conformance: uuid / ipv4 / ipv6 (skip temporal rows).
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/format_conformance/corpus.json"
        ))
        .expect("corpus parses");
        for pair in corpus["pairs"].as_array().expect("pairs") {
            let format = pair["format"].as_str().unwrap_or_default();
            if !STRING_FORMATS.contains(&format) {
                continue;
            }
            let value = pair["value"].as_str().expect("value string");
            let expect = pair["expect_valid"].as_bool().expect("expect_valid");
            assert_eq!(
                is_valid(format, value),
                expect,
                "format_conformance {format} {value:?}"
            );
        }
    }

    #[test]
    fn hostname_corpus_agrees() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/format_hostname/corpus.json"
        ))
        .expect("corpus parses");
        for case in corpus["cases"].as_array().expect("cases") {
            let instance = case["instance"].as_str().expect("instance string");
            let expect = case["valid"].as_bool().expect("valid");
            assert_eq!(
                is_valid("hostname", instance),
                expect,
                "hostname {instance:?}"
            );
        }
    }

    #[test]
    fn uri_corpus_all_uri() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/format_uri/corpus.json"
        ))
        .expect("corpus parses");
        for pair in corpus["pairs"].as_array().expect("pairs") {
            let value = pair["value"].as_str().expect("value string");
            let expect = pair["expect"].as_bool().expect("expect");
            assert_eq!(is_valid("uri", value), expect, "uri {value:?}");
        }
    }

    #[test]
    fn email_corpus_agrees() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/format_email/corpus.json"
        ))
        .expect("corpus parses");
        for pair in corpus["pairs"].as_array().expect("pairs") {
            let instance = pair["instance"].as_str().expect("instance string");
            let expect = pair["expect_valid"].as_bool().expect("expect_valid");
            assert_eq!(is_valid("email", instance), expect, "email {instance:?}");
        }
    }
}
