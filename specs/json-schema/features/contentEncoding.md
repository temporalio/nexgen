# `contentEncoding`

Source: JSON Schema 2020-12, Validation, §8.3 "A Vocabulary for the Contents of
String-Encoded Data → contentEncoding".

Declares that a JSON **string** carries binary data encoded with a named scheme
(`base64`, `base64url`, `quoted-printable`, …). In 2020-12 the content keywords
are **annotations by default** — they do not assert. We adopt the two
byte-transform encodings — **`base64`** (standard alphabet) and **`base64url`**
(URL-safe alphabet, RFC 4648 §5) — and
**materialize both into a native bytes type** (Go `[]byte`, Java `byte[]`,
Python `bytes`, TS `Uint8Array`), differing only in the wire codec the adapter
selects. Every other encoding is rejected at load. This mirrors [[format]]'s
posture exactly: adopt a curated, provably-portable subset, own the check and
the canonical serializer rather than delegating to a native decoder, and reject
the rest.

## Spec summary

Verbatim (2020-12 validation, §8.3):

> If the instance value is a string, this property defines that the string
> SHOULD be interpreted as encoded binary data and decoded using the encoding
> named by this property.

> The value of this property MUST be a string.

And the vocabulary-wide annotation rule (§8.2):

> [The content keywords] function as annotations. … Implementations MAY … decode
> … but … MUST NOT … automatically … Instead … [offer] an opt-in.

Distilled:
- Value **MUST be a string** naming an encoding (`base64`, `base64url`,
  `base16`, `quoted-printable`, `7bit`, `8bit`, `binary`).
- Applies only to a **string** instance; on any other type it is a no-op per the
  spec.
- **Annotation by default** — like [[format]]'s `format-annotation`, it does not
  validate unless an implementation opts in. We opt into assertion +
  materialization for `base64` / `base64url`.
- Describes a string that is *really* binary: the JSON payload is the encoded
  text, the modeled value is the decoded bytes.

## Support decision

**Support:** partial —
**`base64` and `base64url`, materialized to a native bytes type.** Every other
encoding is **rejected at load** (deferred, *not* a categorical [[not]]-style
**P6** exclusion).

The defining choices (citing [[PRINCIPLES.md]]):
- **Both encodings lower to the same bytes type, identically (P1).** Each
  declared encoding has **one deterministic canonical wire form** that every
  language emits byte-identically — the **P1** line — and both decode into the
  same native bytes value. Go / Java / Python get it from a standard **and** a
  URL-safe stdlib codec (Go `base64.StdEncoding` / `base64.RawURLEncoding`, Java
  `Base64.getEncoder()` / `getUrlEncoder().withoutPadding()`, Python
  `base64.b64encode` / `urlsafe_b64encode`); TS has no browser-portable stdlib
  codec, so it gets a small generator-owned pure-JS codec (below). We already
  own the parse/encode adapters (PRINCIPLES: shadow-layout `UnmarshalJSON`, the
  collecting Jackson (de)serializer, the TS and Python transfer type
  converters), so selecting the standard vs URL-safe codec per node is a codec
  choice, not new machinery.
- **A native bytes field is the idiomatic shape (P2).** A base64 blob modeled as
  a bare `string` forces every consumer to decode by hand at each use site;
  `[]byte` / `byte[]` / `bytes` / `Uint8Array` is what a human would have
  written. This is the same materialization tradeoff [[format]] makes for
  temporals — pay conversion cost for the idiomatic field.
- **No new dependency, browser-safe (P4).** Go / Java / Python use only their
  stdlib base64 (Go `encoding/base64`, Java `java.util.Base64`, Python
  `base64`); TS uses a small **emitted pure-JS codec** —
  **no `Buffer` and no `atob`/`btoa`** — so the generated TS runs unchanged in
  the browser as well as Node. No runtime schema library, consistent with the
  hand-emitted-validator ethos.
- **Own the check, don't delegate (P1/P10).** As with [[format]], the validity
  check is a **generator-owned** pinned regex over the wire string (inside the
  [[pattern]] RE2-safe subset) rather than a decoder's own error behavior —
  several base64 decoders are *lenient* (Python `b64decode` without
  `validate=True` silently drops non-alphabet bytes; a naive hand-rolled decoder
  would skip them too), which would let malformed input through differently per
  language and break **P1**. The regex is the oracle; the decoder runs only
  after it passes.

**`base64` is padded, `base64url` is unpadded** — each has a single
**strict canonical form** (no lenient variants accepted). `base64` uses the
standard `+`/`/` alphabet with required `=` padding; `base64url` uses the
URL-safe `-`/`_` alphabet with **no** padding (the RFC 4648 §5 form used by
JWT/JWS and emitted by Go's `RawURLEncoding`). The two encodings materialize to
the **same** bytes type and differ only in the wire codec; the declared
`contentEncoding` picks it. Because only the canonical form is accepted, the
wire round-trips byte-identically with no re-canonicalization step.

**Every non-base64 encoding is rejected.** `quoted-printable`, `7bit`, `8bit`,
`binary`, and `base16` have no portable bytes lowering (`7bit`/`8bit`/`binary`
are not even transformations — they assert the *string itself* is already the
content) and would be accepted-but-unenforced metadata, the exact "looks
constrained, silently isn't" footgun **P10** forbids. Reject with a fix-it.

Loader behavior:
- `contentEncoding` value **not a string** → **reject** (P7.1; the spec's own
  MUST). `{contentEncoding: 5}`, `{contentEncoding: ["base64"]}`.
- `contentEncoding` on a **non-string** [[type]]
  (`{type:"integer", contentEncoding:"base64"}`) → **reject** (P7.1): the spec
  makes it a vacuous no-op; a statically meaningless keyword is a load reject
  here, exactly as [[format]] / [[pattern]] treat a type mismatch.
- Any encoding other than `base64` / `base64url` (`quoted-printable`, `7bit`,
  `8bit`, `binary`, `base16`, unknown) → **reject** with a fix-it listing the
  supported values.
- `contentMediaType` / `contentSchema` present on the same node → **reject**,
  owned by [[contentMediaType]] / [[contentSchema]] (a base64 blob *labeled*
  with a media type has nowhere to emit the label in the model; the reject there
  wins over materialization here).
- A supplied [[const]] / [[default]] / [[enum]] string literal must be valid for
  the declared encoding (checked at load, P7.1) and is stored / echoed in its
  **canonical** form.

## Type mapping

The encoded string is materialized to the language-native **bytes** type; the
wire stays a JSON string. Both encodings share the type and differ only in the
canonical wire form.

| Go       | Java     | Python  | TS           | Canonical wire (identical in all targets)                                                                                          |
| -------- | -------- | ------- | ------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `[]byte` | `byte[]` | `bytes` | `Uint8Array` | `base64`: padded standard alphabet (RFC 4648 §4); `base64url`: unpadded URL-safe alphabet (RFC 4648 §5); no line breaks either way |

The four adapters call the stdlib codec **explicitly** (we own the adapters; we
do not rely on any native `[]byte`/`byte[]` default binding), choosing the
standard or URL-safe variant per the declared `contentEncoding`:

| Language | `base64` codec                                     | `base64url` codec                                                                                           |
| -------- | -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| Go       | `base64.StdEncoding`                               | `base64.RawURLEncoding`                                                                                     |
| Java     | `Base64.getDecoder()` / `Base64.getEncoder()`      | `Base64.getUrlDecoder()` / `Base64.getUrlEncoder().withoutPadding()`                                        |
| Python   | `base64.b64decode(s, validate=True)` / `b64encode` | `urlsafe_b64decode(s + pad)` / `urlsafe_b64encode(b).rstrip(b"=")`                                          |
| TS       | generator-owned `b64ToBytes(s)` / `bytesToB64(v)`  | generator-owned `b64urlToBytes(s)` / `bytesToB64url(v)` — same pure-JS codec, URL-safe alphabet, no padding |

- **TS** — field `Uint8Array` (the idiomatic binary type in both the browser and
  Node). No native bytes-in-JSON, so the codec is a
  **generator-owned pure-JS base64/base64url codec** — a lookup table + plain
  arithmetic over the `Uint8Array`, emitted **once** as a shared runtime helper
  (the compile-once analog). It uses **no `Buffer` and no `atob`/`btoa`**, so
  the generated TS runs unchanged in the browser (P4). *(Keeping a canonical
  `string` in TS — the [[format]]-style fallback used for `date`/`duration` — is
  the alternative in Open questions.)*
- **Python** — the codec is generator-owned, living in the `_parse_base64` /
  `_parse_base64url` / `_format_base64` / `_format_base64url` runtime helpers
  rather than in a library bytes type, for the same reason [[format]] owns its
  temporal parsing: full control of the accept/reject line and the canonical
  output. `urlsafe_b64decode` requires padding, so the unpadded wire is
  re-padded before decode (`s + "=" * (-len(s) % 4)`).

**No member-derived identifier, no P15 surface.** Unlike [[default]]'s
`<Field>OrDefault()` accessor, bytes materialization renames nothing and the
field is simply typed as bytes. A target may share a generator-owned compiled
predicate by encoding kind; that declaration is independent of authored member
and derived-position names, so two members cannot collide through it.

## Validator mapping

Per **P10** / **P11**. The check is a single predicate over the wire **string**,
run in the **parse adapter** (that is where the encoded text is still
observable): the pinned regex for the declared encoding, compiled **once**
([[pattern]]'s machinery — ASCII-class rule and per-target end-anchor
normalization apply). A value that fails is one aggregated `Violation`; a value
that passes is decoded into the native bytes value.

Pinned patterns (written `^…$`, **emitted** with the normalized anchors):

| Encoding    | Pinned pattern                                                                           |
| ----------- | ---------------------------------------------------------------------------------------- |
| `base64`    | `^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/][AQgw]==\|[A-Za-z0-9+/]{2}[AEIMQUYcgkosw048]=)?$` |
| `base64url` | `^(?:[A-Za-z0-9_-]{4})*(?:[A-Za-z0-9_-][AQgw]\|[A-Za-z0-9_-]{2}[AEIMQUYcgkosw048])?$`    |

`base64` accepts canonical **padded** standard base64; `base64url` accepts
canonical **unpadded** URL-safe base64. Both accept the empty string (→ zero
bytes) and reject the *other* encoding's alphabet, wrong padding, embedded
whitespace/newlines, and stray characters — so the wire is unambiguous and the
stdlib decoder below agrees. The final-character classes constrain unused low
bits in the last quantum; without that constraint, multiple accepted strings
could decode to the same bytes and re-encode differently.

| Language   | Strategy                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Go         | Parse adapter: run the encoding's pinned regex over the wire string, pushing a `Violation` on failure; else decode with the codec above → `[]byte`. Encode adapter: re-encode with the same codec. `regexp.MustCompile` compiled once at init.                                                                                                                                                                                                        |
| TypeScript | `fromTransferType`: pinned regex (`/…/u`) — **essential**, since the pure-JS decoder assumes canonical input and won't itself reject malformed text — then the generator-owned decoder → `Uint8Array`. `toTransferType`: the generator-owned encoder. Lookup table + arithmetic; **no `Buffer`/`atob`**, so it runs in the browser.                                                                                                                   |
| Python     | `_parse_base64(v, path, violations)` / `_parse_base64url(...)`, called from the converter: regex over the wire string, then `b64decode(s, validate=True)` (`base64`) or `urlsafe_b64decode(s + pad)` (`base64url`) → `bytes`; on failure they append a `Violation` and return `None` so the rest of the object still validates. Serialize: `_format_base64` / `_format_base64url` emit `b64encode(b)` / `urlsafe_b64encode(b).rstrip(b"=")` as ASCII. |
| Java       | The per-POJO collecting deserializer (PRINCIPLES Java §5): regex over the `String`, then `Base64.getDecoder()` / `getUrlDecoder()` `.decode(s)` → `byte[]`, pushing a `Violation` on failure. The `Serializer` emits with `getEncoder()` / `getUrlEncoder().withoutPadding()`.                                                                                                                                                                        |

**Informative `reason` strings.** The `Violation` `reason` names the encoding
and the offending value (`must be base64url-encoded, got "…"`), per the
[[format]] / [[maximum]] convention, truncating a long value.

**Why compile-once.** As [[pattern]] / [[format]]: the pinned pattern is a
package-level compiled constant; the load gate proves it compiles, so the
emitted `MustCompile` / `Pattern.compile` is unconditional.

### Serialize-side (P12)

The materialized field is a
**native bytes value that cannot hold an invalid encoding** — any `[]byte` /
`byte[]` / `bytes` / `Uint8Array` re-encodes to valid base64 — so the type
system replaces the serialize-side *encoding* validator, exactly as [[format]]'s
materialized temporals do. Serialize is therefore a pure **canonicalization**
(bytes → canonical base64/base64url per the node's encoding), the one place
`contentEncoding` has genuine encode-adapter logic. A co-occurring [[maxLength]]
/ [[minLength]] / [[pattern]] is **not** subsumed by the type, though — the
canonical base64 can still be too long or off-pattern — so that predicate
re-runs over the canonicalized wire string **before emit** (**P12**), as those
specs describe. The canonical wire string is projected **once per member** and
shared by every predicate on that node, including a closed-value comparison
([[const]] / [[enum]]), which compares on that same string. Deriving it
independently per predicate makes the emitted code depend on how many predicates
a node happens to carry — which is how a [[minLength]]`:0`, a bound the spec
declares inert, can break a build.

## Property-testing matrix

### Accepted (positive)

| Shape                                 | Example                                                                                   |
| ------------------------------------- | ----------------------------------------------------------------------------------------- |
| Standard base64 → bytes               | `{type:"string", contentEncoding:"base64"}`                                               |
| URL-safe base64 → bytes               | `{type:"string", contentEncoding:"base64url"}`                                            |
| Empty content                         | `""` → zero-length bytes (either encoding)                                                |
| On a nullable string                  | `{oneOf:[{type:"string", contentEncoding:"base64"},{type:"null"}]}`                       |
| Combined with `pattern` on the wire   | `{type:"string", contentEncoding:"base64", pattern:"^AAA"}` (value must satisfy both)     |
| Combined with `maxLength` on the wire | `{type:"string", contentEncoding:"base64", maxLength:1024}` (bounds the *encoded* string) |
| `const` base64 literal                | `{type:"string", contentEncoding:"base64", const:"aGk="}` (stored/echoed canonical)       |

### Rejected at load time (negative)

| Reason                                    | Example                                                                                                                                                     |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Value not a string                        | `{contentEncoding:5}`, `{contentEncoding:["base64"]}`                                                                                                       |
| Type mismatch (P7.1)                      | `{type:"integer", contentEncoding:"base64"}`                                                                                                                |
| Unsupported encoding                      | `{…, contentEncoding:"quoted-printable"}`, `"7bit"`, `"8bit"`, `"binary"`, `"base16"`                                                                       |
| With `contentMediaType` / `contentSchema` | `{…, contentEncoding:"base64", contentMediaType:"image/png"}` → reject (see [[contentMediaType]])                                                           |
| Literal wrong for its encoding            | `{…, contentEncoding:"base64", const:"a-b_"}` (URL-safe chars under `base64`); `{…, contentEncoding:"base64url", const:"aGk="}` (padding under `base64url`) |

### Runtime fixtures (validator + round-trip)

- Valid `base64` / `base64url` → decodes to the expected bytes; re-serializes
  **byte-identically** across Go/Java/Python/TS (canonical padded / unpadded
  form respectively).
- Wrong-alphabet or wrong-padding wire (URL-safe `-_` under `base64`, `=`
  padding under `base64url`, missing padding under `base64`, embedded newline,
  stray `!`) → one aggregated `Violation`; **no** language's lenient decoder
  silently accepts it (the regex is the gate).
- The **same bytes** encode to different wire under the two encodings (`>>>` →
  `Pj4+` under `base64`, `Pj4-` under `base64url`) — verified distinct and each
  canonical.
- Empty string → zero-length bytes, round-trips to `""`.
- A failing sibling constraint (`maxLength` / `pattern` / another field) →
  **all** reported in one shot (**P11**).

## Interactions

- **[[type]]**: gates applicability — meaningful only for `string`; a mismatch
  is a load reject (**P7.1**). The
  **emitted field type is the native bytes construct**, not `string` (the wire
  is still a JSON string).
- **[[contentMediaType]] / [[contentSchema]]**: both are **rejected** (they have
  no place to emit in the model). A base64 blob *labeled* with a media type
  therefore rejects on the label, not here — see [[contentMediaType]]. This is
  why materialized bytes are unlabeled binary in v1.
- **[[pattern]]**: the pinned encoding patterns stay inside its RE2-safe subset
  and use its compile-once mechanism, ASCII-class rule, and end-anchor
  normalization. Both may appear — the **wire string** must satisfy both.
- **[[format]]**: a **materializing** temporal format cannot share a node with
  `contentEncoding` — both would own the same in-memory slot, and the grammars
  are disjoint anyway (an RFC 3339 form contains `-` and `:`, which no base64
  alphabet admits). **Reject the combination at load**, with a fix-it naming the
  conflict, rather than silently discarding either adapter. A
  **non-materializing** (string-shaped) format is **accepted** beside
  `contentEncoding` and its check applies to the **encoded wire string** in
  *every* target, aggregating with the encoding's check: dropping it in one
  target is an accept-set divergence, and being redundant with the base64 shape
  in most cases does not license omitting it in one.
- **[[minLength]] / [[maxLength]]**: independent string assertions over the
  **encoded wire string** (not the decoded byte length); not cross-checked
  against the base64 shape.
- **[[const]] / [[default]] / [[enum]]**: a supplied string literal MUST be
  valid for the declared encoding at load and is stored / echoed in canonical
  form, mirroring [[format]].
- **[[nullability]]**: orthogonal — a `null` skips the check and is not
  materialized; a present value is checked and decoded. A [[const]] / [[enum]]
  belongs on the **non-null branch**. Putting it on the `oneOf` wrapper is a
  load reject under the shared applicator ownership rule; the diagnostic tells
  the author to move it into the branch whose values it constrains. This keeps
  the null branch and bytes materialization intact without creating a second
  wrapper-level precedence rule.
- **[[oneOf]]**: **deferred** on a non-object branch of a sum type — the
  synthesized `<Union><Kind>` wrapper has no bytes construct to hold, so it is
  rejected rather than materialized in one target and left an unvalidated
  `string` in the others ([[oneOf]] §Deferred). The [[nullability]] `oneOf`
  wrapper is not a sum type and materializes normally.
- **[[uniqueItems]]**: **supported** on a `contentEncoding` element. Because
  only the canonical wire form is accepted, a byte value has exactly one
  spelling, so the encoded strings partition the elements exactly as the decoded
  byte values do. That support is **contingent on** the strictness above:
  admitting unpadded, URL-safe-under-`base64`, or non-canonical trailing bits
  would give one byte value several spellings and bring the element under
  [[uniqueItems]]' deferral.
- **[[contains]]**: `contentEncoding` on a `contains` **matcher** rejects; on
  the **element** it is supported, and the matcher measures the encoded wire
  string in both directions. Every target renders that canonical element
  projection before applying the matcher.
- **[[required]]**: orthogonal — presence vs value shape.

## Ecosystem variance

| Source dialect      | Action                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| JSON Schema 2020-12 | Annotation by default; we opt into assertion + materialization for `base64` / `base64url`, reject the rest. Native keyword, no rewrite.                                                                                                                                                                                                                                                                                                                                                                                                                        |
| OpenAPI 3.1         | Adopts 2020-12 — `contentEncoding` native, same names. The OAS 3.0 `type:"string", format:"byte"` (base64) / `format:"binary"` idioms are **not** JSON Schema formats — rejected as unknown [[format]]s; author should use `contentEncoding:"base64"`.                                                                                                                                                                                                                                                                                                         |
| OpenAPI 3.0         | Human porting note: no `contentEncoding`; base64 is spelled `format:"byte"`, which is not a JSON Schema format. A document *declaring* the OAS 3.0 dialect is not an accepted input, but the spelling itself reaches the loader in a `$schema`-less fragment and rejects as an unknown [[format]] — and that diagnostic must name `contentEncoding:"base64"` as the in-subset alternative, since it is the one migration hint an OAS-3.0 author most needs. Not for `format:"binary"`: that means raw octets, so the same hint would silently change the wire. |
| draft-07            | Human porting note: the keyword existed there with the same names. A document that declares draft-07 rejects on the dialect pin before this keyword is inspected; in a `$schema`-less fragment the keyword is read as here — `base64` / `base64url` assert and materialize, every other encoding rejects.                                                                                                                                                                                                                                                      |

## Open questions

1. **TS `Uint8Array` vs canonical `string`.** TS materializes via a
   generator-owned pure-JS codec; the [[format]]-style fallback (keep a
   canonical base64 `string` in TS, as `date`/`duration` do) remains the
   conservative alternative if a consumer would rather own the decoding
   themselves.
2. **Media-type-labeled bytes.** Carrying `contentMediaType` as
   **Temporal payload metadata** (rather than a model field) would let
   `{contentEncoding:"base64", contentMediaType:"image/png"}` be accepted as
   labeled binary — see [[contentMediaType]].

## See also

- [[format]] — the sibling "adopt a curated portable subset, own the check and
  the canonical serializer, materialize to a native type, defer the rest" spec
  that this one mirrors.
- [[type]] — supplies the base `string`; gates applicability; the materialized
  field type is the native bytes construct.
- [[pattern]] — the regex keyword whose RE2-safe subset the base64 check stays
  inside, and whose compile-once mechanism and anchor normalization it uses.
- [[contentMediaType]] — rejected; owns the "no place to emit a media type in
  the model" rationale that also blocks labeled base64 blobs.
- [[contentSchema]] — rejected; the embedded-document keyword.
- [[const]] / [[default]] / [[enum]] — supplied literals validated and
  canonicalized against the encoding at load.
- [[minLength]] / [[maxLength]] — independent assertions over the encoded wire
  string.
- [[nullability]] — a `null` is neither checked nor materialized.
- [[PRINCIPLES.md]] — **P1** (identical wire), **P2** (idiomatic field), **P4**
  (stdlib only), **P10/P11/P12** (enforced, aggregated, serialize-side).
