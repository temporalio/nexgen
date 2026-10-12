# `format`

Source: JSON Schema 2020-12, Validation, §7 "Vocabularies for Semantic
Content With 'format'".

Annotates a string with a named semantic shape (`uuid`, `date-time`,
`ipv4`, …). In 2020-12 `format` is an **annotation by default** — the
`format-annotation` vocabulary is required and merely *collects* the value;
assertion is the *opt-in* `format-assertion` vocabulary. We deliberately
**opt into assertion behavior for a curated, provably-portable subset** and
**reject every other format at load**, because an accepted-but-unenforced
`format` is exactly the "looks constrained, silently isn't" footgun **P10**
(validation is enforced, not advisory) forbids. We do **not** delegate to
any target's native format validator — those are the single most divergent
corner of JSON Schema across implementations, which is *why* the spec made
assertion optional. Each supported format lowers to a **generator-owned**
check: a pinned portable regex inside the [[pattern]] RE2-safe subset, plus —
where a regex alone is insufficient — a shared **length guard** or **calendar
predicate**, all plain arithmetic.

Beyond validation, the temporal formats (`date-time`, `date`, `time`,
`duration`) are **materialized** as idiomatic typed model fields (Go
`time.Time`, Java `OffsetDateTime`, Python `datetime`, …) rather than a bare
`string` — see **Materialization (type mapping)**. That is the one place
`format` departs from a pure assertion: the field carries a language-native
value, and the wire is produced by **re-serializing** it — with **no
truncation** (offset and sub-second precision preserved to each type's
resolution), `date-time` and `time` being the two formats whose round-trip may
lose information, and only at a target type's genuine limit. The shipped rules
are executed by `tests/json_schema_corpus_runtime.rs` across Go, Java, Python,
and default TypeScript; format rows additionally run the `typescript-date` and
`typescript-temporal` profiles. Where a rule is gate-checked, the Rust gate
consumes the same pinned data.
Prospective targets are design analysis, not part of this executable claim.

## Spec summary

Verbatim (2020-12 validation, §7.1 / §7.2):

> The value of this keyword is called a format attribute. It MUST be a
> string.

> [Format attributes] generally … only [apply to specific instance types].
> If the type of the instance to validate is not in this set, validation
> for this format attribute and instance SHOULD succeed.

> [Format-Annotation, required] … the "format" keyword … MUST … be
> collected as an annotation … The implementation MUST provide options to
> enable and disable such [validation] evaluation and MUST be disabled by
> default.

> [Format-Assertion, optional] … implementations MUST provide full
> validation support for all of the formats defined by this specification …
> When the Format-Assertion vocabulary is specified, implementations MUST
> fail upon encountering unknown formats.

Distilled:
- Value MUST be a **string** naming a format.
- Two vocabularies: **`format-annotation`** (required, default) collects
  `format` as a **pure annotation** and does **not** validate;
  **`format-assertion`** (optional, opt-in) **validates** and **fails on
  unknown formats**.
- A format generally targets **one instance type** (the standard set all
  target `string`); on any other type it is a **no-op** per the spec.
- Standard formats (§7.3): `date-time`, `date`, `time`, `duration`
  (§7.3.1); `email`, `idn-email` (§7.3.2); `hostname`, `idn-hostname`
  (§7.3.3); `ipv4`, `ipv6` (§7.3.4); `uri`, `uri-reference`, `iri`,
  `iri-reference`, `uuid` (§7.3.5); `uri-template` (§7.3.6);
  `json-pointer`, `relative-json-pointer` (§7.3.7); `regex` (§7.3.8).

## Support decision

**Support:** partial — a **curated portable subset**, each **asserted**
(we adopt `format-assertion` semantics for it) by lowering to a
generator-owned check. Everything outside the subset — including the
spec-default annotation-only fallthrough — is **rejected at load**
(deferred, *not* a categorical **P6** exclusion).

The opt-in is meant to be made **explicit**: the `*.nexusrpc.yaml` IDE-support
schema is to declare the `format-assertion` vocabulary in its `$vocabulary`
(mapped to `true`), so tooling reads that these formats are *validated*, not
merely annotated — the assertion behavior self-documenting rather than
implicit. **Status: unimplemented** — no such IDE-support schema artifact is
published. Assertion semantics are enforced by the generated code regardless;
what is missing is the machine-readable declaration of it.

**Asserted (v1)** — grouped by the shape of the owned check:

- **Pinned regex only** (the syntax fully captures validity):
  - `uuid` — RFC 4122 8-4-4-4-12 hex.
  - `ipv4` — dotted-quad, each octet `0–255`, no leading zeros.
  - `ipv6` — RFC 4291 (full, `::`-compressed, and IPv4-tail forms).
  - `uri`, `uri-reference` — RFC 3986, ASCII-only, at high fidelity
    (`json-schema/corpora/format_uri/`, 72 pairs, 7/7 agree, including the
    IP-literal tightening below). The IP-literal host `[…]` is validated
    semantically by splicing in the pinned `ipv6` grammar (below).
- **Pinned regex + a length guard** (RE2 has no total-length lookahead, so
  a cheap `code_point_count` check rides alongside the regex in the shared
  `Validate`):
  - `hostname` — RFC 1123 LDH labels; each label ≤63, **total ≤253**
    (`json-schema/corpora/format_hostname/`, 41 pairs, 7/7 agree).
  - `email` — a well-defined **ASCII dot-atom** subset of RFC 5321 (no
    quoted locals, comments, IP-literals, or IDN); **total ≤254**, the
    guard runs *before* the regex to neutralize a Java matcher hazard
    (`json-schema/corpora/format_email/`, 56 pairs, 7/7 agree).
- **Pinned regex (syntax) + a shared calendar/range predicate, and
  materialized** (below): `date`, `date-time`, `time`, `duration` — RFC 3339
  profile; the pattern pins the offset's numeric range and the predicate
  enforces the year floor, day-in-month and the Gregorian leap-year rule
  (`json-schema/corpora/format_conformance/`, 136 pairs, 7/7 agree).

**Deferred (rejected at load, "not yet supported"):** `idn-email`,
`idn-hostname`, `iri`, `iri-reference` (all need **IDNA / Unicode** handling
that diverges across engines — WHATWG punycodes, Ruby ASCII-rejects; the
asserted set is deliberately ASCII-only, the portable line); `uri-template`
(RFC 6570 templating grammar), `json-pointer`, `relative-json-pointer`, and
`regex` (niche; `regex` would additionally mean running the [[pattern]] gate
over the *value*). Each is deferred, *not* a categorical **P6** exclusion.

**Unknown / non-standard format** (`format: "phone"`, a typo, a custom
string) → **reject** with a fix-it listing the supported names. An
unrecognized format is the ambiguity **P7.1** rejects loudly, and
`format-assertion` itself mandates failing on unknown formats. Two names get a
**named in-subset alternative** rather than the bare list, because they are the
common spellings of a supported concept: `byte` names
`contentEncoding: "base64"` ([[contentEncoding]]), and `int32`/`int64` name
[[type]] `integer` with the matching bounds. `binary` deliberately gets no such
hint — it means raw octets, so pointing an author at base64 would silently
change the wire.

**`format` on a non-string [[type]]** (`{type:"integer", format:"uuid"}`)
→ **reject** (**P7.1**). The spec would make it a vacuous no-op; a
statically meaningless keyword is a load reject here, exactly as
[[pattern]] / the count keywords treat a type mismatch.

Grounding ([[PRINCIPLES.md]]): **P1** (identical cross-language accept /
reject **and** identical wire bytes — guaranteed by owning the check and the
generator-owned serializer, proven by the corpora, never by a native validator;
`date-time` and `time` are the bounded exceptions where round-trip may be lossy,
below),
**P2** (a typed field is the idiomatic, hand-written-feeling shape — the
motivation for materialization), **P4** (only each stdlib's regex engine and
temporal types — no new dependency), **P10** (enforced at the boundary),
**P11** (aggregated), **P12** (see Validator mapping). The curated line is
the **P1** line, mirroring [[pattern]]'s "portable subset accepted, hazardous
form rejected, deferred not excluded".

**Materialization narrows two grammars, node-scoped.** Materializing a
temporal into a native type means the native type must be able to *hold* the
value, which the full RFC 3339 / ISO 8601 grammar does not always allow. So a
**materialized** node asserts a **narrower** grammar than a **string-opt-out**
node (below):
- **Leap second `:60` is rejected** on a materialized `date-time` / `time`
  node. No stdlib temporal type can store `:60`, and native parsers **split**
  on it: Go / Java / Python / .NET *reject*, while **JS `Temporal` and Ruby
  silently clamp** `:60`→`:59` (corruption — `Temporal` clamps even with
  `{overflow:'reject'}`) (`json-schema/corpora/format_materialize_clock/`).
  Since the owned validator rejects `:60` before any native parse runs, no target ever
  reaches the clamp — but the split is exactly why we reject uniformly rather
  than delegate. A **string-opt-out** node keeps accepting `:60` (the current
  pure-assertion contract).
- **`duration` is narrowed to a time-only duration** — `PT`-forms of hours /
  minutes / seconds only. The calendar components (years, months, weeks,
  days) are **rejected** on a materialized node, because no stdlib
  fixed-duration type (`time.Duration`, `timedelta`, `java.time.Duration`,
  `TimeSpan`) can represent calendar-variable years/months without a
  reference date. A **string-opt-out** node keeps the full duration grammar.
- **`date-time` / `time` offsets are narrowed to `-14:00..+14:00` inclusive**
  — at hour 14, only minute 00 is admitted. This is the single materialized
  offset domain in every target. Real-world UTC offsets span
  `-12:00..+14:00`, and `±14:00` is the narrowest range that every target's
  idiomatic native carrier can hold (.NET `DateTimeOffset` caps at `±14:00`,
  Java `ZoneOffset` at `±18:00`, while Go `time.Location`, Python `tzinfo`,
  and `Temporal.ZonedDateTime` are wider). A wider offset cannot be silently
  renormalized into range, because readers must round-trip the authored
  offset. A future string opt-out keeps RFC 3339's wider numeric offset
  grammar.

All three narrowings are strictly *more* restrictive (no previously-rejected value
becomes accepted) and are the price of the idiomatic typed field.

**RFC 3339 edge decisions (pinned, temporal formats).** All targets follow
these because we own the check:
- **`date-time` offset is required** (`Z` or `±HH:MM`); a bare local
  `date-time` is invalid. `-00:00` is accepted. **`time` offset is optional**
  (RFC 3339 `partial-time`); an offset, when present, is range-checked.
- **Offset range** is enforced by the pinned pattern's own alternation:
  `00:00–13:59` plus `14:00`, with either sign. Thus `±14:00` is accepted,
  while `±14:01`, `±15:00`, `±18:00`, and malformed minutes such as `+01:60` are
  rejected. (The serialize-side range check over a native offset is separate
  — below.)
- **Case** — `T` / `Z` separators are accepted in either case (RFC 3339
  §5.6). Materialized nodes **uppercase on the parse path** before the native
  parse (Go / Python / Ruby native parsers reject lowercase; safe because the
  grammar has no other letters).
- **Calendar validity** (`date`, and the date half of `date-time`) enforces
  year `0001–9999`, month `01–12`, day within the month's length, and the
  Gregorian leap-year rule. Year `0000` is rejected uniformly: Python's native
  temporal types cannot represent it, so accepting it elsewhere would violate
  P1.
- **Leap second** — see the narrowing above: **rejected** on materialized
  nodes, **accepted** on string-opt-out nodes.

**Edge decisions for the string-shaped formats** (pinned, corpus-proven):
- **`hostname`**: a **trailing dot** is **rejected** (note `ajv` accepts it);
  an **all-numeric label / TLD** is **accepted** (RFC 1123's note is not
  RE2-expressible; documented residual risk); `xn--` A-labels pass as LDH
  (Punycode is `idn-hostname`, deferred).
- **`email`**: ASCII dot-atom local, single `@`, `hostname`-style domain of
  **≥2 labels** (`user@localhost` rejected). Quoted locals, comments,
  IP-literals, trailing dots, Unicode rejected. The **≤254 guard precedes the
  regex**: `java.util.regex` matches the nested dot-atom quantifier
  recursively and throws `StackOverflowError` on multi-thousand-char runs;
  the cap keeps every engine safe (RE2 engines are already linear/immune).
- **`uri` / `uri-reference`**: RFC 3986 faithful for scheme, percent-encoding
  (`%HEXDIG HEXDIG` only), the authority/path split, and ASCII-only
  enforcement. The IP-literal host `[…]` is validated *semantically* — the
  pinned `ipv6` grammar is spliced into the authority production, so
  `http://[1::2::3]` (double `::`) is **rejected** like a bare `ipv6` would be
  (`IPvFuture` literals stay structural).

## Materialization (type mapping)

The temporal formats carry a **typed model field**; the rest stay `string`.
The typed value is **authoritative** (authority model B): the parse path turns
the validated wire string into it, and the serialize path re-emits it through a
**generator-owned per-language serializer**. That serializer applies **no
truncation** — it preserves the original UTC offset and the full sub-second
precision to the extent the target's native type can hold them, regardless of
which languages the generation targets. Where a language lacks a suitable native
type, the field stays a `string` **holding that same serialized form**, so it
agrees byte-for-byte with the materializing languages.

**`date-time` round-trip is a deliberate, bounded exception** — the sibling of
the Go/Java/Python optional+nullable collapse (a wire distinction the in-memory
model cannot carry). Loss happens **only at a native type's genuine capacity limit**,
never as an artificial common-denominator floor, so a value round-trips
**byte-identically across every target whose type can hold its offset and
precision** and diverges only where a chosen type cannot. `date` and `duration`
round-trip losslessly. **`date-time` and `time` are the two formats whose
round-trip may lose information** across ordinary targets — which target, and
how much, is tabulated below.

For `time` the licensed losses are exactly the two that are a **materialized
native type's** resolution: Python's `datetime.time` is microsecond and Go's
`time.Time` is nanosecond, so finer input is truncated there. **Java and
TypeScript may not lose a digit of `time`**: both hold it in a string carrier —
Java in a `String`, TypeScript in a `string` in every mode — which materializes
nothing and has no capacity limit of its own, so [[PRINCIPLES.md]] P1's
exception **(b)** does not reach it. Routing a `String`-carried value through
`java.time` for validation and rebuilding the string from the result imports
that intermediary's nanosecond limit into a carrier that can hold more, which is
the artificial common-denominator floor (b) forbids: a target that *can*
represent a value must not be made to drop it.

| Format | Go | Java | Python | TS (per `--date-time-types`) | Wire form |
|---|---|---|---|---|---|
| `date-time` | `time.Time` | `OffsetDateTime` | `datetime` (aware) | `string` / `Date` / `Temporal.ZonedDateTime` | materialized RFC 3339 subset, **original offset & sub-second precision preserved** (per-target loss below) |
| `date` | `time.Time`† | `LocalDate` | `date` | `string` / `Temporal.PlainDate` | `YYYY-MM-DD` (lossless) |
| `time` | `time.Time`† | `String` (validated and canonicalized) | `time` (aware / naive) | `string` (all modes) | `HH:MM:SS[.f…]` + **offset preserved when present** |
| `duration` | `time.Duration` | `Duration` | `timedelta` | `string` / `Temporal.Duration` | `PTnHnMnS` (time-only; omit zero components; `PT0S` for zero) |
| `uuid` / `ipv4` / `ipv6` / `hostname` / `email` / `uri` / `uri-reference` | `string` | `String` | `str` | `string` | verbatim (no materialization) |

† Go has no date-only / time-of-day type; `time.Time` carries a phantom date /
time-of-day the serializer ignores (an offset still rides in its zone).

**The TS column** lists the `string` default and the `--date-time-types=temporal`
type; `date-time` additionally offers `Date` under `--date-time-types=date`
(no other format has a built-in `Date` analog). TS `date` never uses `Date` —
a `Date` is a UTC **instant** that misreads a plain date under local
`getHours()`, so its proper typed form is `Temporal.PlainDate`.

**Serialized form** (all materializing targets, generator-owned): **RFC 3339**,
original offset preserved (`+00:00` / `-00:00` → `Z`), `T` / `Z` uppercased,
fractional seconds at the value's own precision with trailing zeros trimmed (no
fractional part when zero). `Temporal.ZonedDateTime` preserves the offset too
(serialized with `.toString({ timeZoneName: 'never' })`, then the same
`+00:00`→`Z` normalization). The **one** target that cannot carry an offset is
TS `date-time` under `--date-time-types=date`, whose `Date.toISOString()`
folds to a UTC instant (always 3 digits). The serializer is **owned, not
native**, because the stdlib emitters disagree
(`json-schema/corpora/format_materialize_clock/`): Java `OffsetDateTime.toString` pads to
fixed 3/6/9-digit groups (no trailing-zero trim), and Python `isoformat` emits
`.500000` and `+00:00` rather than `.5` / `Z` — only Go `RFC3339Nano` and
Temporal's `.toString({ fractionalSecondDigits: 'auto' })` already match the
rules above.

**`date-time` per-target round-trip fidelity:**

| Target | In-memory type | Round-trip |
|---|---|---|
| Go | `time.Time` | offset + **nanosecond** preserved; **sub-nanosecond truncated** (the type's resolution) |
| Java | `OffsetDateTime` | offset + **nanosecond** preserved; **sub-nanosecond truncated** (the type's resolution) |
| Python | `datetime` (aware) | offset preserved, **sub-microsecond truncated** (the type's resolution) |
| TS `--date-time-types=string` (default) | `string` | serialized form stored — lossless at every width |
| TS `--date-time-types=date` | `Date` | UTC instant at **millisecond** — offset folded to UTC, sub-ms lost |
| TS `--date-time-types=temporal` | `Temporal.ZonedDateTime` | offset + **nanosecond** preserved; **sub-nanosecond truncated** |

So `2021-06-15T12:30:45.123456789+02:00` keeps its offset and nanoseconds in Go
/ Java / TS `string` / TS `Temporal.ZonedDateTime`, loses the trailing `789` in
Python, and folds to `…Z` (also to ms) only in the legacy TS `date` mode. Any
input up to **microsecond** precision with any offset round-trips
**byte-identically across Go / Java / Python / TS `string` / TS `temporal`**.
(Prospective .NET adds a third resolution tier: `DateTimeOffset` is 100-ns and
**rounds** rather than truncates — `.123456789`→`.1234568` — so it would need
its own row if promoted to a model target.)

**`time` preserves the offset** and keeps sub-second precision at its carrier's
resolution — every accepted digit for the two string carriers, the native
resolution for the two native ones. RFC 3339 keeps the `time` offset
**optional**, and an offset-less value stays offset-less. Go's phantom
`time.Time` and Python's aware/naive `time` carry either form. Java and
TypeScript keep the canonicalized wire string because neither `java.time` nor
Temporal has one time-of-day type that can carry both offset-bearing and
offset-less values without a wrapper. **`duration` canonicalizes**
value-preserving non-canonical inputs
(`PT90M` → `PT1H30M`, `PT3600S` → `PT1H`) **byte-identically across languages**.

**JS temporal representation (`--date-time-types`).** JS is the only target
with more than one reasonable in-memory shape for a temporal — a lossy legacy
`Date`, the new `Temporal` API, or a plain `string` — so **one** generator-wide
flag (a CLI option, and the equivalent API option per **P16**) selects the TS
type for **all four** temporal formats at once. It affects **only** the TS
output; Go / Java / Python are unchanged, and choosing a lossy TS shape does
**not** pull the other targets down (no truncation propagates across languages):

| `--date-time-types` | `date-time` | `date` | `time` | `duration` |
|---|---|---|---|---|
| `string` (default) | `string` | `string` | `string` | `string` |
| `date` | `Date` | `string` | `string` | `string` |
| `temporal` | `Temporal.ZonedDateTime` | `Temporal.PlainDate` | `string`* | `Temporal.Duration` |

\* `time` has no `Temporal` type that carries an offset, so it stays a
`string` even in `temporal` mode (see below).

- **`string`** (default) — every temporal is a `string` holding the
  generator-serialized form with every accepted fractional digit preserved.
  It agrees with Go and Java through nanosecond precision and is strictly more
  faithful for finer input. **Lossless**, maximally portable (no runtime feature), and
  free of `Date` footguns. Still **materialized** nodes (model B — the narrowed
  grammar rejects `:60`), *not* the verbatim model-A opt-out below.
- **`date`** — only `date-time` gains a native type, the legacy `Date`: a UTC
  instant at **millisecond** resolution that folds the offset and drops sub-ms
  precision. `date` / `time` / `duration` have no `Date` analog and stay
  `string`. The loss is confined to the TS `date-time` output.
- **`temporal`** — the idiomatic TC39 **Temporal** type for each format,
  lossless to each type's own resolution: `date-time` →
  `Temporal.ZonedDateTime` (offset + nanosecond preserved, matching Go /
  Java), `date` → `PlainDate`, `duration` →
  `Duration`. `time` stays a `string`, because Temporal has no offset-bearing
  time-only type and `PlainTime` would drop the offset — keeping the offset
  wins. The trade-off is portability — it needs a Temporal-capable runtime or
  polyfill (newer Node / browsers), so it is **less portable** than the default.

This flag is orthogonal to the string opt-out below: it selects the *type* of
**materialized** TS fields, whereas the opt-out drops materialization entirely
(in every language).

**String opt-out (authority model A).** A node may opt out of materialization
and keep a **verbatim `string`** in *every* language (byte-exact round-trip,
offset and precision preserved, `:60` and calendar durations accepted), with
an optional derived accessor (`asDateTime()` / `AsOffsetDateTime()` /
`.as_datetime()`) that parses on demand. Use it where **byte-exact** fidelity
or the *wider* grammar is contractually significant — it keeps the incidental
formatting the materialized form normalizes away (case, `+00:00` vs `Z`,
trailing fractional zeros), the sub-microsecond digits Python would otherwise
drop, and the pre-narrowing grammar (`:60`, calendar durations). The opt-out is
per-node (and available as a generator-wide mode).

> **Status: unimplemented.** There is no opt-out keyword, no generator-wide
> mode, and no derived accessor; every temporal node materializes. Every
> statement about opt-out nodes in this spec (the wider grammars above, the
> string-shaped validator row, the accepted-matrix row) describes the intended
> design, not current behavior. This is load-bearing beyond `format`:
> **P1's bounded exception (b) is conditioned on the loss being recoverable
> through a per-field `string` opt-out**, so until this ships, Python's
> sub-microsecond truncation and the legacy TS `date` fold have no recovery
> path.

**Doc comment.** The materialized field's doc comment names the format and its
round-trip behavior (`// format: date-time — offset & precision preserved;
round-trip may lose precision beyond this type's resolution`) so any loss is
visible in the generated source (**P2**). The one TS mode whose loss is not
merely its type's resolution is legacy `--date-time-types=date`, whose comment
names the UTC-instant fold and millisecond resolution; `temporal`
(`ZonedDateTime`) keeps the offset and nanoseconds, like the default.

> **Status: unimplemented.** Generated field comments do not yet include this
> format-specific round-trip text.

## Validator mapping

Per **P10** / **P11**. For a **string-shaped format** (`uuid`, `ipv4`,
`ipv6`, `hostname`, `email`, `uri`, `uri-reference`, and any opt-out
temporal) the check is a single predicate over the decoded `string`,
identical in both directions (shared `Validate`, **P12**): the pinned regex
compiled **once** ([[pattern]]'s machinery — the ASCII-class rule and the
per-target end-anchor `$`→`\Z`/`\z` normalization apply), plus the length
guard for `hostname` / `email`. Pinned patterns (written `^…$`; **emitted**
with the normalized anchors):

| Format | Pinned pattern / source | Auxiliary check |
|---|---|---|
| `uuid` | `^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$` | — |
| `ipv4` | `^(?:(25[0-5]\|2[0-4][0-9]\|1[0-9][0-9]\|[1-9][0-9]\|[0-9])\.){3}(25[0-5]\|2[0-4][0-9]\|1[0-9][0-9]\|[1-9][0-9]\|[0-9])$` | — |
| `ipv6` | RFC 4291 — see `json-schema/corpora/format_conformance/` (authoritative form) | — |
| `hostname` | `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*$` | length ≤253 |
| `email` | ASCII dot-atom — see `json-schema/corpora/format_email/` | length ≤254 (runs **first**) |
| `uri` / `uri-reference` | RFC 3986 ASCII — see `json-schema/corpora/format_uri/pinned_body.body` | IP-literal host: pinned `ipv6` grammar |

For a **materialized temporal**, the pinned regex + calendar/range predicate
run in the **parse adapter over the wire string** (that is where `:60`,
offset, and precision are still observable) — a value that fails is one
aggregated `Violation`; a value that passes is then **uppercased and parsed
into the native construct** — offset and sub-second precision **preserved** to
the type's resolution (no truncation), except legacy TS `date` mode (`Date`
folded to a UTC instant), per the table. Pinned temporal patterns (the
materialized, `:60`-rejecting grammar):

| Format | Pinned pattern (materialized node) |
|---|---|
| `date` | `^[0-9]{4}-(0[1-9]\|1[0-2])-(0[1-9]\|[12][0-9]\|3[01])$` + calendar predicate (year `0001–9999`) |
| `time` | `^([01][0-9]\|2[0-3]):[0-5][0-9]:[0-5][0-9](\.[0-9]+)?([Zz]\|[+-]((0[0-9]\|1[0-3]):[0-5][0-9]\|14:00))?$` (offset optional; no `\|60`) |
| `date-time` | full-date `[Tt]` full-time, **offset required**, no `\|60`, offset `00:00–13:59` or `14:00` + calendar + range |
| `duration` | `^PT(?:[0-9]+H(?:[0-9]+M(?:[0-9]+S)?)?\|[0-9]+M(?:[0-9]+S)?\|[0-9]+S)$` (time-only) |

*(A string-opt-out temporal node keeps the wider grammar: `time` / `date-time`
add back the `|60` seconds alternative; `duration` uses the full
`PnYnMnDTnHnMnS` / `PnW` grammar.)*

| Language | Strategy (materialized temporal) |
|---|---|
| Go | Parse adapter: run the pinned regex + `validRFC3339(...)` over the wire string, pushing a `Violation` on failure; else `t, _ := time.Parse(time.RFC3339Nano, strings.ToUpper(s))` → store `t` **as parsed** (offset and nanoseconds retained; no `UTC()`, no truncation) for `date-time`, or `time.Parse("2006-01-02", s)` (`date`); `duration` parses the `PT…` components into a `time.Duration`. Encode adapter: `t.Format(time.RFC3339Nano)` (offset preserved, `Z` for zero offset, trailing-zero fractional trimmed). `regexp.MustCompile` compiled once at init. |
| TypeScript | Parse adapter: pinned regex (`/…/u`) + calendar/range check, then per `--date-time-types`: **`string`** (default) store the generator-serialized string for every temporal; **`date`** `new Date(s)` for `date-time` (others string); **`temporal`** `Temporal.ZonedDateTime.from` (`date-time`, in the wire's offset zone) / `PlainDate.from` (`date`) / `Duration.from` (`duration`), with `time` staying a string. Encode adapter: **`string`** emit the stored string; **`date`** `date-time` → `.toISOString()` (UTC, ms, 3 digits); **`temporal`** `ZonedDateTime.toString({ timeZoneName: 'never' })` (offset kept, then `+00:00`→`Z`), `PlainDate.toString()`, and for `Duration` a generator-owned formatter over `value.total({unit: 'seconds'})` so component-equivalent values canonicalize. |
| Python | Parse: the `_parse_date_time` / `_parse_date` / `_parse_time` / `_parse_duration` runtime helpers, called from the transfer type converter (PRINCIPLES Python §3) — regex + calendar over the wire string, then `datetime.fromisoformat(s.upper())` **retaining the parsed offset** (`date-time`; `datetime`'s native microsecond resolution truncates any finer input — the one Python-side loss), `date.fromisoformat(s)` (`date`), `time.fromisoformat(s)` **retaining any offset** as an aware `time` (`time`), or parse `PT…` into a `timedelta` (`duration`). Each appends a `Violation` and returns `None` on failure so the rest of the object still validates. Encode: the matching generator-owned `_format_*` helper (offset preserved, fractional trimmed). The dataclass field is the plain `datetime.datetime` / `date` / `time` / `timedelta`, so no library's own coercion is in the path (native coercions typically accept a missing offset and normalize differently). |
| Java | The per-POJO collecting deserializer (PRINCIPLES Java §5): regex + calendar over the `String`, then generator-owned `parseDateTimeLiteral` applies case normalization and fraction truncation before `OffsetDateTime.parse`, **retaining the offset and up to nanoseconds** (`date-time`; defaults use the same helper), `LocalDate.parse` (`date`), generator-owned string arithmetic for `time` that retains every accepted fractional digit and offset, or checked component parsing plus `Duration.ofSeconds` for the `PT…` form. The `Serializer` emits the **generator-owned** string (offset preserved, fractional trimmed) — **not** `Duration.toString()` for `.NET` parity and **not** the BCL serializer (`.NET XmlConvert` rolls `PT24H`→`P1D`). |

There is **no common truncation floor**, and the resolution is a property of the
*carrier*, not of the target:

* A **string carrier retains every accepted digit** — TypeScript's `string` in
  every mode, and Java's `String` for `time`. It has no capacity limit, so a
  validation step that borrows an intermediary's limit is a defect rather than a
  bounded loss.
* A **materialized native carrier retains its type's genuine resolution** — Go
  `time.Time`, Java `OffsetDateTime` and TypeScript `Temporal` are nanosecond;
  Python `datetime`/`time` are microsecond; the legacy TypeScript `Date` mode is
  millisecond.

Only the second is a licensed loss (P1 exception **(b)**), and only at the
type's real limit. The
generator-owned serializer trims trailing-zero fractional digits so equal values
agree byte-for-byte wherever the carriers are equally capable. Under
`--date-time-types`, `Temporal.ZonedDateTime` keeps the offset (byte-identical
with Go/Java); only the legacy TS `date` mode is a UTC-instant fold.

**Informative `reason` strings.** The `Violation` `reason` names the
**format and the offending value** (`must be a valid date-time, got "…"`),
per the [[maximum]] / [[pattern]] convention.

**Why compile-once.** As [[pattern]]: the pinned pattern is a package-level
compiled constant; the load gate proves it compiles, so the emitted
`MustCompile` / `Pattern.compile` is unconditional.

### Serialize-side (P12)

- **String-shaped formats:** the predicate **re-runs before emit** over the
  decoded string, so an in-memory value set to a non-UUID (etc.) fails
  serialize with the same aggregated primitive — real teeth in the
  statically-typed targets. The check is direction-agnostic.
- **Materialized temporals:** the model field is a native type, so most wire
  grammar checks no longer apply — but **no native temporal type is a subset
  of the wire grammar**, so the serializer owes one explicit check per way a
  constructible in-memory value cannot be spelled. Each failure is an
  aggregated `Violation` pushed **before** formatting; emitting a string the
  generated parser would itself reject is a P1 break, not a nicety:
  - **Calendar floor** (`date` / `date-time`): Go, Java and Temporal can
    construct a native year-zero value while Python cannot — year `< 1` is a
    violation.
  - **Calendar ceiling** (`date` / `date-time`): the wire form carries a
    four-digit year, so year `> 9999` is a violation.
  - **Offset granularity** (`date-time` / `time`): RFC 3339 spells an offset
    in whole minutes, while Go's `time.FixedZone("", 30)` and Java's
    `ZoneOffset.ofTotalSeconds(30)` carry seconds — a sub-minute offset is a
    violation, not something to round or drop.
  - **Offset magnitude** (`date-time` / `time`): the materialized grammar is
    limited to `-14:00..+14:00`; a constructible Go/Python/Java value outside
    that domain is a violation (Java `OffsetDateTime` itself admits up to
    `±18:00`, so it is checked explicitly), while TypeScript re-runs the pinned
    regex over the serialized `date-time` and Java/TypeScript string-carried
    `time` re-runs the pinned regex.
  - **Duration sign** (`duration`): the pinned grammar has no sign, but
    `time.Duration`, `java.time.Duration`, `timedelta` and
    `Temporal.Duration` are all signed. A negative duration is a violation —
    `-90 * time.Minute` must not emit `"PT-1H-30M"`.
  - **Duration resolution** (`duration`): the grammar's seconds component is
    integral, while the native types are nanosecond- or
    microsecond-resolution. A duration carrying a fraction of a second
    (`500 * time.Millisecond`) is a violation, never a silent fold to
    `"PT0S"`.
  - **Missing offset** (`date-time`, Python): a naive `datetime` cannot satisfy
    the offset-required wire grammar.
  - **Invalid native date** (`date-time`, TypeScript `date` mode):
    `new Date(NaN)` is constructible but cannot be serialized.
  - **Duration units** (`duration`): the grammar is time-only, so a native
    value carrying calendar components (`Temporal.Duration` with `years` /
    `months` / `weeks`) is a violation. A native `days` component is **not**:
    absent a reference date it denotes exactly 24 h, so it converts and emits
    as `PT24H`. Only the three that cannot be converted without a calendar are
    violations — unlike a *wire* `P1D`, which the narrowed grammar rejects
    because the wire form must be unambiguous.
  - **Duration magnitude** (`duration`, Python / Java / TypeScript): a native
    value above the shared wire cap of 9,223,372,036 seconds is a violation.

  Where a target holds a temporal as a `string` rather than a native type
  (the TS `--date-time-types=string` default, and any format a target has no
  type for), the stored string is not authoritative by construction either:
  re-run the pinned predicate over it before emit, exactly as for the
  string-shaped formats above.

  **Each of these checks is a `Violation`, never an escaping throw.** Every one
  of the unspellable states above is reachable through a native constructor or
  accessor that *throws* in at least one target — `Temporal.Duration.total`
  refuses a calendar duration, `Date.prototype.toISOString` refuses an invalid
  `Date`, `datetime.utcoffset` refuses a `tzinfo` beyond ±24 h, a `java.time`
  parser refuses an out-of-band offset. So the check must run **before** the
  call that would throw, and any throw that remains possible is caught at the
  member and pushed as a `Violation` on that member's path. An escaping
  exception is not a louder version of a rejection: it loses the path and
  discards every violation already aggregated for the payload, which is the
  aggregate **P11** exists to deliver, and it leaves the two directions
  reporting different violation sets (**P11.1**).

  Serialize then **re-serializes** typed → wire with
  offset and precision preserved to the type's resolution. A co-occurring
  [[minLength]] /
  [[maxLength]] / [[pattern]] is **not** subsumed by the type, though — the
  re-serialized wire string can still be too long or off-pattern — so that
  predicate re-runs over it **before emit** (**P12**). The wire string is
  projected **once per member** and shared by every predicate on that node,
  including a closed-value comparison ([[const]] / [[enum]]): two independent
  projections are two operands (**P12.2**), and a second projection in one
  scope is a redeclaration. Deriving it per predicate also makes the emitted
  code depend on how many predicates a node happens to carry, which is how a
  [[minLength]]`:0` — a bound the spec declares inert — can break a build. The
  only parse-side guard
  beyond validation is a **duration overflow check** (the regex caps no digit
  count, so an adversarial `PT999999999999H` that overflows the native type
  pushes a `Violation`); the duration-magnitude serialize check above enforces
  the same cap for constructible in-memory values.

## Containment between formats

Two formats stand in a **containment** relation when every value the narrower
one's pinned check accepts, the wider one's pinned check also accepts. This spec
owns the relation; [[allOf]] consumes it (a merge of a containment pair narrows
to the contained format, rather than rejecting the pair as a conflict).

**The basis, not a lookup table.** Containment is a claim about the **accepted
value sets of the pinned checks** — regex + length guard + calendar/range
predicate — never about what the format *names* suggest. A pair enters the
relation only when inclusion is established over those checks, by an argument
over the wider check's alphabet and length bounds and then confirmed against the
corpora. Adding a format, or tightening any pinned check, re-opens every pair it
touches. **Overlap is not containment:** two formats that share values but each
accept values the other refuses are a genuine conflict, and the merge must
reject them.

Established on that basis:

| Relation | Why |
|---|---|
| `uri ⊂ uri-reference` | RFC 3986 defines `URI-reference = URI / relative-ref`, and the pinned `uri-reference` body carries the `uri` body as its first alternative |
| `hostname ⊂ uri-reference` | a hostname's alphabet is `A-Za-z0-9`, `-` and `.` — all RFC 3986 unreserved — and contains no `:`, so every hostname is a legal relative-path reference |
| `uuid ⊂ hostname`, `ipv4 ⊂ hostname`, `date ⊂ hostname` | each is a single LDH label (or dot-separated numeric labels), bounded at 36 / 15 / 10 code points, inside the 63-per-label and 253-total guards |
| `duration ⊂ uri-reference`, and `uuid` / `ipv4` / `date` ⊂ `uri-reference` | by the alphabet argument above, and transitively through `hostname` |

`email`, `date-time`, `time` and `ipv6` are in **no** containment relation with
any supported format: `date-time`, `time` and `ipv6` all carry `:`, which a
relative-path reference's first segment excludes and no hostname label admits,
and `email`'s dot-atom alphabet includes `` ` ``, `{`, `|`, `}` and `^`, which
RFC 3986 excludes — so `email` is not contained in `uri-reference` even though
an ordinary address is.

`duration` is **not** contained in `hostname`, which is the case that shows why
the basis matters rather than the intuition: the pinned `duration` grammar bounds
no digit run, so `PT` followed by seventy zeros and `1S` is an accepted duration
(one second, well inside the overflow cap) and is a single 73-character label,
which the 63-per-label guard refuses.

## Property-testing matrix

### Accepted (positive)

| Shape | Example |
|---|---|
| `uuid` / `ipv4` / `ipv6` | `{type:"string", format:"uuid"}` |
| `date-time` / `date` / `time` | `{type:"string", format:"date-time"}` (materialized) |
| First calendar year | `0001-01-01`, `0001-01-01T00:00:00Z` |
| `duration` (time-only) | `{type:"string", format:"duration"}` → accepts `PT1H30M`, `PT0S` |
| `hostname` / `email` | `{type:"string", format:"hostname"}` |
| `uri` / `uri-reference` | `{type:"string", format:"uri"}` |
| Combined with `pattern` | `{type:"string", format:"uuid", pattern:"^0"}` (value must satisfy both) |
| On a nullable string | `{oneOf:[{type:"string", format:"uuid"},{type:"null"}]}` |
| String opt-out keeps the wider grammar (**unimplemented**) | opt-out `date-time` accepts `…T23:59:60Z`; opt-out `duration` accepts `P1Y` |

### Rejected at load time (negative)

| Reason | Example |
|---|---|
| Value not a string | `format:5`, `format:true`, `format:["uuid"]` |
| Type mismatch (P7.1) | `{type:"integer", format:"uuid"}`, `{type:"boolean", format:"date"}` |
| Unknown / non-standard format | `{type:"string", format:"phone"}`, `{…, format:"datetime"}` (typo) |
| Deferred standard format | `{…, format:"idn-email"}`, `{…, format:"iri"}`, `{…, format:"uri-template"}`, `{…, format:"regex"}` |
| Materialized narrowing: leap second | materialized `{…, format:"date-time"}` with `const:"2021-12-31T23:59:60Z"` |
| Materialized narrowing: calendar duration | materialized `{…, format:"duration"}` with `const:"P1Y"` / `"P4W"` / `"P1D"` |
| Calendar year zero | materialized `{…, format:"date"}` with `const:"0000-01-01"` (same for `date-time`) |
| Literal fails its format | `{…, format:"uuid", const:"not-a-uuid"}`, `{…, default:"nope"}` |

### Runtime fixtures (validator + round-trip)

Per-format accept/reject is exercised by `tests/json_schema_corpus_runtime.rs`
over `json-schema/corpora/format_conformance/` (136 pairs, covering `date`,
`time`, `date-time`, `ipv4`, `ipv6`, `uuid`), `json-schema/corpora/format_email/`
(56), `json-schema/corpora/format_hostname/` (41) and
`json-schema/corpora/format_uri/` (72, driven as `uri`). The materialization
round-trips are tabulated by `json-schema/corpora/format_materialize_clock/`
(`date-time` / `date` / `time` sections). Each valid row has a common canonical
wire plus a `typescript_date_wire` override where legacy `Date` intentionally
folds to UTC/milliseconds. `:60` rows are executable common rejections, not
skips. Python's nanosecond-to-microsecond loss remains the live `09#8`
divergence and is reported on every run rather than accepted as an override.

`json-schema/corpora/format_duration/` covers canonical values, `PT90M` and
`PT3600S` normalization, the common overflow edge, and malformed, calendar,
signed, and fractional rejections. `json-schema/corpora/format_uri_reference/`
covers absolute and relative references, queries, fragments, the empty
reference, escaping, raw Unicode, and malformed hosts. Accepted rows in both
corpora are round-tripped through generated code. Format execution uses six
profiles: the four default targets plus `typescript-date` and
`typescript-temporal`.

Representative cases:

- **String formats** — `uuid` canonical OK, wrong length/non-hex → fail;
  `ipv4` `256.0.0.1` / `01.2.3.4` → fail; `email` `user@localhost` → fail;
  `uri` truncated `%2` / non-ASCII → fail; `http://[1::2::3]` (double `::`) →
  fail (spliced `ipv6` grammar); `http://[::1]` / `http://[2001:db8::1]` → OK.
- **`date-time` round-trip** (offset & precision preserved to each type's
  limit): `2021-06-15T12:30:45.123456+02:00` → **verbatim** in Go / Java /
  Python / TS `string` / TS `temporal` (`ZonedDateTime`);
  `…12:30:45.123456789+02:00` → Go / Java / TS `string` / TS `temporal` keep
  `…789+02:00`, Python emits `…123456+02:00` (sub-µs truncated), only legacy TS
  `date` folds to `…10:30:45.123Z`; `+00:00` → `Z`; lowercase `t`/`z` →
  uppercase; `…T23:59:60Z` → **runtime parse reject** (materialized).
- **`date`**: `2020-02-29` OK; `2021-02-29` / `2021-13-01` → fail.
- **`time`**: `12:30:45+02:00` → `12:30:45+02:00` (**offset preserved** in every
  language and mode, trailing-zero fractional trimmed; a `string` in TS
  including `--date-time-types=temporal`); `±14:00` is accepted, while
  `±14:01`, `±18:00`, and `±23:59` are rejected, as for `date-time`.
- **`duration`**: `PT90M` → `PT1H30M`; `PT0S` OK; `P1Y` / `P4W` / `P1D` →
  **runtime parse reject** (materialized time-only); overflow `PT<huge>H` →
  `Violation`.
- Combined with a failing [[minLength]] / [[maxLength]] / [[pattern]] or a
  sibling field → **all** reported in one shot (**P11**).

## Interactions

- **[[contentEncoding]]**: a **materializing** temporal `format` and
  `contentEncoding` cannot share a node — both claim the same in-memory slot,
  and the two grammars are disjoint anyway (an RFC 3339 form contains `-` and
  `:`, which no base64 alphabet admits). **Reject the combination at load**,
  with a fix-it naming the conflict; the same rule is stated in
  [[contentEncoding]]. A **non-materializing** (string-shaped) `format` beside
  `contentEncoding` is **accepted**, and its check applies to the **encoded
  wire string** in *every* target, aggregating with the encoding's own check.
  Dropping it in one target is an accept-set divergence
  (`{contentEncoding:"base64", format:"email"}` must reject `"aGk="`
  everywhere), and the check being redundant with the encoding's pattern in
  most cases does not license omitting it in one.
- **[[pattern]]**: every regex-lowered format is held to that keyword's
  RE2-safe subset — each pinned pattern passes its gate **unchanged** — and uses
  the same compile-once mechanism, ASCII-class rule and end-anchor
  normalization. Both keywords may appear on one node — the value must satisfy
  **both**, aggregated independently.
- **[[type]]**: gates applicability — `format` is meaningful only for
  `string`; a mismatch is a load reject (**P7.1**). For a materialized
  temporal the **emitted field type is the native construct** where the target
  has one (the wire is still a JSON string); in TS that is governed by
  `--date-time-types` — `string` (default) keeps all four temporals as
  `string`, `date` gives only `date-time` a `Date`, and `temporal` uses the
  matching `Temporal` type for each.
- **[[const]] / [[default]] / [[enum]]**: a supplied string literal MUST
  satisfy the format at load; on a **materialized** node it must also be
  **materializable** (a `const` `date-time` cannot be `:60`; a `const`
  `duration` must be time-only) and is stored/echoed in its **serialized**
  form — which is also the form every other assertion on the node is checked
  against, so the literal is canonicalized **before** any of them run. A
  **closed value set** does not remove the format check: where a target gives
  the closed set its own named type, the pinned regex is still evaluated over
  that value's underlying `string`.
- **[[minLength]] / [[maxLength]]**: independent string assertions; not
  cross-checked against a format's implied length. On a materialized temporal
  they constrain the **canonical** *wire* string at both boundaries (see
  [[maxLength]]); the internal length guards
  (`hostname` / `email`) are separate.
- **[[nullability]]**: orthogonal — a `null` skips the format check and is not
  materialized; a present value is checked (and materialized).
- **[[oneOf]]**: an **asserted** (non-materializing) format on a non-object
  branch of a sum type rides along like any other branch constraint — the branch
  stays `string` and the pinned check runs once the token selects it. A
  **materialized temporal** format there is **deferred**: the synthesized
  `<Union><Kind>` wrapper has no native construct to hold, so it is rejected
  rather than silently materialized in one target and left an unvalidated
  `string` in the others ([[oneOf]] §Deferred). The [[nullability]] `oneOf`
  wrapper is not a sum type and materializes normally.
- **[[required]]**: orthogonal — presence vs value shape.
- **[[allOf]]**: two branches may each declare a `format`. The **containment
  relation** above decides the pair: a containment pair merges to the contained
  format, and anything else — overlap without containment, or two unrelated
  formats — is a conflict the merge rejects. Equal formats merge trivially.
- **[[uniqueItems]]**: `uniqueItems: true` over an element carrying a
  materializing temporal `format` is a **load reject** — the materialized value
  cannot answer element-to-element JSON equality (see [[uniqueItems]]). A
  non-materializing format leaves the element a `string` and is unaffected.
- **[[contains]]**: a `contains` **matcher** may not carry a temporal `format`;
  on an **element** it is supported, and the matcher measures the element's
  canonical wire form in both directions. Every target renders that canonical
  element projection before applying the matcher.

## Ecosystem variance

| Source dialect | Action |
|---|---|
| JSON Schema 2020-12 | `format-annotation` is the default (collect only); we opt into `format-assertion` for the curated subset and reject the rest. Native format names, no rewrite. |
| OpenAPI 3.1 | Adopts 2020-12 `format`; same names. Native. OAS-specific formats (`int32`, `int64`, `float`, `double`, `password`, `byte`, `binary`) are **not** JSON Schema formats — treated as unknown and rejected. |
| OpenAPI 3.0 / draft-4 | **Human porting guidance only:** such documents are rejected when they declare a non-2020-12 `$schema`; a `$schema`-less fragment reaches no dialect gate. When converting one to JSON Schema 2020-12, `date-time` / `date` / `uuid` / `email` / `uri` / `hostname` names carry over; rewrite `url` to `uri`. |
| Swagger 2.0 | **Human porting guidance only:** same conversion rules as OpenAPI 3.0; the original document is not an accepted input. |

**Why native validators / parsers can't serve as the oracle** (empirical, in
the corpora): they diverge and/or mutate, so delegating would break **P1** or
the wire round-trip. Highlights: JS `Date` accepts `2021-02-30` and a missing
offset; JS `Temporal` and Ruby clamp `:60`→`:59` (`Temporal` even with
`{overflow:'reject'}`); `.NET MailAddress` accepts `user@localhost`
and full IDN; `.NET Uri.CheckHostName` accepts underscores/trailing dots;
Java `Duration.parse`/`Period.parse` disagree on Y/M and `P1W`; `.NET
XmlConvert` collapses `P1Y`→365d and rolls `PT24H`→`P1D`; 27/57 tricky URIs
get divergent verdicts across the seven native URI parsers.

## Open questions

1. **Remaining deferred formats.** `idn-email`, `idn-hostname`, `iri`,
   `iri-reference` await a portable **IDNA / Unicode** story; `uri-template`,
   `json-pointer`, `relative-json-pointer`, `regex` are niche. Candidates for
   later admission once a portable owned check is corpus-proven.
2. **Full-grammar `duration` via a component struct.** The materialized
   `duration` is narrowed to time-only so it can be a native type. To also
   support calendar durations (`P1Y`, `P4W`), a **generated component struct**
   (`{years,months,weeks,days,hours,minutes,seconds}`) round-trips the full
   grammar byte-identically in all six languages (design B) — a candidate
   representation for a node that needs Y/M/W, or the behavior behind the
   string opt-out's accessor. Deferred pending demand.
3. **A `Temporal` type for `time`.** `time` is the one temporal that stays a
   `string` under `--date-time-types=temporal`, because Temporal ships no
   offset-bearing time-only type (`PlainTime` would drop the offset). If
   Temporal later adds one — or if a schema's `time` is known offset-less —
   `PlainTime` could materialize it, at the cost of the string carrier's
   unlimited fractional precision. Ruby (prospective) likewise has no
   time-of-day type.

## See also

- [[pattern]] — the regex keyword whose RE2-safe subset every pinned pattern
  stays inside, and whose compile-once mechanism, ASCII-class rule and
  end-anchor normalization `format` uses.
- [[type]] — supplies the base `string`; a materialized temporal replaces the
  field type with a native construct. The **applicability gate is owned here**:
  the load reject for a `format` on a non-string `type` (**P7.1**) is this
  spec's rule and this spec's test, not a rule [[type]] delegates back.
- [[const]] / [[default]] / [[enum]] — supplied literals validated (and, when
  materialized, canonicalized) against the format at load.
- [[minLength]] / [[maxLength]] — independent string assertions.
- [[nullability]] — a `null` is neither validated nor materialized.
- [[multipleOf]] — the sibling "support the portable subset, reject the
  hazardous form, deferred not excluded" decision posture.
- [[allOf]] — consumes the containment relation defined here when merging two
  branches that each declare a `format`.
- [[maximum]] — the `reason`-string convention.
