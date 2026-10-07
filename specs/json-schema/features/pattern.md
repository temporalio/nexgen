# `pattern`

Source: JSON Schema 2020-12, Validation vocabulary, §6.3.3
"Validation Keywords for Strings → pattern".

Asserts that a string instance **matches a regular expression**. A pure
runtime assertion — no type impact. The string keyword with the richest
cross-language hazard: the four targets ship **different regex engines**
whose dialect, anchoring, and character-class semantics diverge, so the
supported form is narrowed to the **portable (RE2-safe) subset** matched
with **unanchored search** and **ASCII** class semantics. That
configuration was validated against the **conformance corpus** of
`(pattern, instance)` pairs run through all four runtime engines plus the
Rust gate (`json-schema/corpora/pattern_conformance/`), which proved the compile
gate + pinned flags alone is *not* enough: constructs that compile everywhere
still match differently, and constructs the gate's own engine compiles still
fail to compile in a target. The gate therefore carries an explicit rule per
construct — some rejecting it, some **normalizing** the emitted pattern
(**Conformance-verified gate rules**, below). Every corpus pair agrees
value-for-value under those rules, but corpus agreement is not by itself
evidence that the gate is complete.

## Spec summary

Verbatim (2020-12 validation, §6.3.3):

> The value of this keyword MUST be a string. This string SHOULD be a
> valid regular expression, according to the ECMA-262 regular expression
> dialect.

> A string instance is considered valid if the regular expression matches
> the instance successfully. Recall: regular expressions are not
> implicitly anchored.

Distilled:
- Value MUST be a **string** (a regular expression, ECMA-262 dialect).
- Instance valid iff the regex **matches somewhere** in the string —
  **not implicitly anchored** (a substring match suffices unless the
  pattern itself uses `^`/`$`).
- Applies only to string instances; a `pattern` on a non-string [[type]]
  is rejected at load (**P7.1**).
- Pure assertion; no annotation behavior.

## Support decision

**Support:** partial — **portable (RE2-safe) subset only**, matched with
**unanchored, ASCII-class, code-point** semantics. A pattern that the
pure-Rust **`regex` crate** (the generator's own engine) cannot compile —
lookahead `(?=…)`/`(?!…)`, lookbehind, backreferences `\1`, and other
backtracking-only Perl constructs — is **rejected at load** (deferred,
*not* a categorical P6 exclusion). Additional syntax categories are rejected
or normalized where the runtime engines diverge: inline flags are rejected;
`.` / `\s` / `\S` and the `$` end-anchor are normalized; non-portable escapes,
assertion spellings, class operations, named captures, and ambiguous unbounded
repetitions are rejected. The full rule list is below, with its two open gaps
marked (rules 7 and 8). Applies only
to `string` fields. "RE2-safe" below names the
**regular (no-backtracking) subset** — the algorithm family that Rust's
`regex` and Go's `regexp` both implement — not a dependency on Go.

Rationale (citing [[PRINCIPLES.md]]):
- **P1 (identical cross-language validation).** The same
  `(pattern, instance)` must produce the same accept/reject in Go, TS,
  Python, and Java. The four engines diverge on the axes below; each is
  pinned to the portable choice, and the
  conformance corpus then caught three more (**Conformance-verified gate
  rules**, further below):
  1. **Dialect.** The *regular* engines — Rust's `regex` crate (what the
     generator itself uses) and Go's `regexp` (RE2) — have **no
     backtracking** and reject lookaround/backreferences; ECMA-262, Python
     `re`, and `java.util.regex` all **accept** them. So the **load-time gate
     compiles the pattern with the Rust `regex` crate** (below; pure Rust, no
     Go toolchain), which refuses exactly the non-regular constructs — the
     ones with no portable linear-time semantics anyway. Compiling under the
     gate's engine is **necessary but not sufficient**: its accepted language
     is a *superset* of ECMA-262-with-`u`, Python `re`, and `java.util.regex`
     in several directions — a quantifier stacked on a quantifier (`a{2}*`) and
     a leading `]` inside a class are two — so each such direction needs its
     own rule below.
  2. **Anchoring.** The spec says regexes are *not* implicitly anchored,
     so we use each engine's **unanchored search**: Go `MatchString`, JS
     `RegExp.test`, Python `re.search`, Java `Matcher.find`. The footgun
     is **Java `Matcher.matches()`**, which anchors the *whole* input
     (`"cat"` fails to match `"the cat sat"` — verified) and Python
     `re.match`, which anchors the *start*; using either would silently
     diverge from the spec and the other three.
  3. **Character classes & the dot.** `\d`/`\w`/`\s` must be **ASCII**
     (ECMA-262: `\d` ≡ `[0-9]`), and `.` must match one **code point**.
     Verified divergences, each pinned: Python `re` makes `\d`
     **Unicode-aware by default** (matches Arabic-Indic `٣`) → compile
     with **`re.ASCII`**; JS `.` is a **UTF-16 code unit** without the
     `u` flag (fails on astral input) → emit with the **`u` flag**; Java
     `\d` becomes Unicode under `UNICODE_CHARACTER_CLASS` → use
     **default flags** (Java's `.` already matches a code point, and `\d`
     is ASCII by default). Go RE2 is already ASCII-class + rune-`.`. The `u`
     flag settles the *width* of `.`; the engines also disagree on which line
     terminators it excludes, which is why the emitted `.` is the explicit
     class of gate rule 4.
- **P4 (minimal runtime deps).** Every target's regex engine is in its
  standard library / language runtime (Go `regexp`, JS `RegExp`, Python
  `re`, `java.util.regex`) — no third-party dependency, unlike a shared
  decimal library. The constraint is purely *which subset and flags*, not
  *whether* an engine exists.
- **P7 / P7.1 (reject ambiguity loudly).** A pattern that matches in three
  languages and fails to compile as a regular expression (or matches an
  astral character in one and not another) is exactly the
  silently-inconsistent output the mission forbids. Reject the non-portable form at load with a clear
  diagnostic naming the offending construct.

**Conformance-verified gate rules (beyond no-backtracking).** The
`(pattern, instance)` corpus run through all four runtime engines + the
Rust gate (`json-schema/corpora/pattern_conformance/`) showed the compile gate +
pinned flags is **not** sufficient. Each rule below was established by
measuring the construct in all four engines — the corpus pins the ones whose
divergence a `(pattern, instance)` pair can express — so the gate applies:
1. **Inline flag groups `(?i)` / `(?flags:…)` → reject.** JS `RegExp`
   cannot compile them (they are not ECMA-262 syntax); Rust/Go/Python/Java
   all do — a pure compile-acceptance gap (`(?i)^cat$` fails only in JS).
   No portable rewrite (case-folding a whole pattern is out of scope), so
   this one is a reject, not a normalization.
2. **`\s` / `\S` → normalize to a canonical ASCII class.** JS whitespace
   `\s` is the full **Unicode** set and is **not** flag-controllable
   (matches NBSP U+00A0, ideographic space U+3000, …), whereas Go/RE2,
   Python `re.ASCII`, and Java-default are ASCII — *and even the ASCII sets
   disagree on `\v`* (only Go/RE2 omits it). So rather than lean on any
   engine's `\s`, the emitter **rewrites `\s`→`[\t\n\x0B\f\r ]` and
   `\S`→`[^\t\n\x0B\f\r ]`** (the explicit set spliced in for *every*
   target), which every engine then matches identically. The set is
   deliberately **ASCII** (dropping ECMA-262's Unicode spaces, consistent
   with the ASCII `\d`/`\w` pinning) and **includes `\v` (U+000B)** —
   written `\x0B`, not `\v`, to avoid shorthand ambiguity — because
   ECMA-262 / Python / Java / JS all include it and only Go/RE2 omits it, so
   spelling it explicitly makes Go agree and honors author intent.
   The rewrite is defined for every placement, and the corpus pins each
   one: standalone `\s`/`\S`; `\s` inside a class
   (`[\s.]`→`[\t\n\x0B\f\r .]`, `[^\s]`→`[^\t\n\x0B\f\r ]`); sole-member
   `[\S]`→`[^…]` and the double-negation `[^\S]`→`[…]`. **`\d`/`\w` are
   untouched** (identical ASCII across all four already). *One narrow
   reject remains:* **`\S` inside a multi-member class** (`[\S.]`, `[\S\d]`)
   — "not-whitespace OR something" is an open-ended complement RE2/JS/Python
   cannot spell as a positive member list (no nested negation / class
   subtraction), so it is statically detected and rejected with the
   explicit-class fix-it.
3. **`$` end-anchor → normalize.** Python and Java `$` match at end-of-input
   *or before a single trailing `\n`*; Go and JS `$` match end-of-input
   only. Rejecting `$` is untenable — `^…$` is the most common pattern shape
   and there is **no** portable end-anchor to switch to (JS has no
   `\z`/`\Z`). So the emitter **rewrites the unescaped `$` assertion** to an
   end-of-input anchor without the newline exception: `\Z` for Python, `\z`
   for Java; Go and JS keep `$` (already exception-free). Provably
   semantics-aligning (multiline is never enabled), so it *eliminates* the
   divergence rather than hiding it; `^` needs no change. **Watch the `\Z`
   vs `\z` letter flip:** Python's strict end-of-string anchor is `\Z`,
   whereas Java's (and .NET's / Ruby's — see Prospective targets) is `\z`,
   with `\Z` being the *lenient* one there. The generator emits the right
   letter per target.
4. **`.` → `[^\n]`.** Runtime dot semantics disagree on which line terminators
   are excluded. The explicit class pins the project's one-code-point,
   not-newline rule and is the spelling used in diagnostics.
5. **Non-portable escapes → reject.** Only the shared escape vocabulary is
   admitted: ECMA-262 `u` mode restricts an identity escape to
   `^ $ \ . * + ? ( ) [ ] { } |` and `/`, plus `-` **inside** a character
   class. So escaped punctuation such as `\_`, `\"`, `\ `, `\#`, `\&`, `\~`,
   and `\-` outside a class; `\a`/`\v`; octal/`\0`; `\uFFFF`;
   `\UFFFFFFFF`; `\x{…}`; and Unicode property escapes `\p{…}` are rejected.
   The portable spellings are the bare character, or `\xHH` for a control code.
6. **Non-portable assertions and captures → reject.** `\A`, `\z`, every
   word-boundary spelling (`\b`, `\B`, `\b{start}`/`\b{end}`, `\<`/`\>`),
   and both named-capture syntaxes are outside the shared grammar. Plain
   `\b` / `\B` compile everywhere but are still rejected: Java treats a
   non-ASCII letter adjacent to the boundary as a word character while the
   other targets and the pinned ASCII `\w` set do not. Authors must spell the
   intended ASCII delimiter structure explicitly (`pattern` never reads a
   capture, so `(?:…)` remains the portable group spelling).
7. **Ambiguous punctuation/classes → reject.** A lone `{`, `}`, or `]`
   **outside a character class**; POSIX classes; nested classes; and class set
   operations (`&&`, `--`, `~~`) are rejected rather than interpreted
   differently per engine. The `outside a class` scoping is load-bearing;
   leading-`]` spellings such as `[]]`, `[]a]`, `[^]]` and `[]-a]` are also
   rejected because Node in `u` mode refuses them.
8. **Ambiguous unbounded repetition → reject (D7).** Nested/unbounded
   quantifier shapes such as `^([a-z]+)+$`, stacked repetitions such as
   `a{2}*`, and repeated alternations with equal positive fixed-width branches
   such as `^(a|b)*$` and `^(ab|cd)*$` are rejected. The last family is a
   measured Java `StackOverflowError` hazard on inputs of a few kilobytes — an
   `Error`, which no generated `catch` intercepts. **This rule remains a
   structural guard, not a general regex-complexity proof**: it deliberately
   retains unequal-width structured alternatives used by the pinned URI
   grammar. New admitted shapes still require cross-engine corpus evidence.

All of these compile under the gate's own engine, so the gate additionally
walks the pattern's `regex-syntax` **AST**: it applies every reject above
(inline-flag groups, non-portable escapes and assertions, named captures, lone
brackets, POSIX / nested / set-operator classes, ambiguous unbounded
repetitions), locates each `\s`/`\S` Perl node (with its `negated` flag,
enclosing-class context, and byte span) to splice the rewrite, locates each `.`
for the class rewrite, and locates the `$`
assertion for the anchor rewrite. The AST is escape-safe for free: an
escaped `\$` or a literal `s` from `\\s` produces no assertion / Perl node
and is left untouched. Still pure Rust, no Go toolchain.

This is the same "support the portable subset, reject the hazardous form
at load, deferred not excluded" posture as [[multipleOf]] (fractional
divisors) and the [[patternProperties]] carve-out — which already flagged
this exact RE2-vs-ECMA-262 dialect gap for the *key* space; `pattern`
manages it for the *value*.

Loader behavior:
- `pattern` not a string → reject.
- `pattern` on a non-string [[type]] → reject (**P7.1**). Where the `type` is
  itself unsupported — an array `type` such as `["string","null"]` — **that**
  reject takes precedence: the actionable fix is the `type` spelling, and a
  `pattern` diagnostic sends the author to the wrong keyword.
- **`pattern` does not compile under the Rust `regex` crate → reject** with
  a "not portable / not yet supported" diagnostic that names the construct
  (lookahead, lookbehind, backreference, …) and notes the generator
  supports the regular (no-backtracking) subset. This is the portability
  gate: the loader (pure Rust) compiles the pattern once with the `regex`
  crate — **no Go toolchain dependency** — and success ⟹ the regular
  subset. The gate then walks
  the `regex-syntax` **AST** for the conformance rules above; compile success
  alone does not establish portability. The gate itself runs no production
  match — but the literal check below does, and it runs under the **pinned
  runtime semantics**, not the gate engine's defaults.
- **Inline flag group `(?i)` / `(?flags:…)` → reject** — not ECMA-262; JS
  cannot compile it (Conformance-verified gate rules).
- **`\s` / `\S` → normalized** to `[\t\n\x0B\f\r ]` / `[^\t\n\x0B\f\r ]` in
  the emitted pattern (all placements: standalone, in-class, and
  sole-member `[\S]`/`[^\S]`). **Exception:** `\S` inside a *multi-member*
  class (`[\S.]`) → reject with a fix-it (spell the set explicitly) — an
  open-ended complement with no portable positive form. `\d`/`\w` untouched.
- **`$` end-anchor → normalized, not rejected** — emitted as `\Z` (Python)
  / `\z` (Java), kept as `$` (Go/JS), so it means end-of-input in every
  target (no trailing-`\n` exception).
- **`.` → normalized** to `[^\n]` in the emitted pattern (gate rule 4), so the
  emitted regex — and every diagnostic that quotes it — carries the explicit
  class rather than the authored `.`.
- **Non-portable escapes, assertion spellings, named captures, lone
  `{`/`}`/`]`, POSIX / nested / set-operator classes, and ambiguous unbounded
  repetitions → reject** (gate rules 5-8), each with a fix-it naming the
  portable spelling.
- **Empty pattern `""`** → matches every string (vacuous no-op) → accepted
  but constrains nothing (mirrors [[minLength]]`:0`).
- A `const`/`default`/`enum` string literal on the **same node** that does
  **not** match the pattern → reject at load (e.g. `{type:"string",
  pattern:"^[a-z]+$", const:"AB"}`). The string-regex half of the deferred
  literal-vs-constraint obligation ([[const]] / [[default]]).
  The literal is matched against the **gate-normalized** pattern under the
  **same pinned semantics the emitted validators use** — ASCII `\d`/`\w`/`\s`,
  code-point `.`, unanchored — so the loader's accept set is the runtime accept
  set. Matching with an engine's Unicode-by-default classes instead does both
  harms at once: `{pattern:"^\\w+$", default:"café"}` loads and emits a constant
  every target's own validator rejects, and `{pattern:"^\\W+$", default:"é"}`
  is refused though all four accept it. On a materialized node the literal is
  **canonicalized before it is matched** (see Serialize-side).

**Deferred, not excluded.** The rejects are the conservative v1 line, not a
permanent boundary. A future release could **admit `(?i)`** via a case-fold
rewrite or a real flag channel, admit the **backtracking constructs** where a
portable rewrite exists / by shipping a shared engine, admit **`\S` in a
multi-member class** if a portable positive form is found, or admit any of the
rule 5-8 categories by rewriting them to their portable spelling at emit time
(a POSIX class and an escaped punctuation character both have one) — each gated
on the conformance corpus agreeing. Tracked in the open question below; mirrors
[[multipleOf]]'s fractional-divisor deferral and [[patternProperties]]'
single-pattern carve-out.

## Type mapping

None. `pattern` never changes the emitted field type — [[type]]'s
`string`, unless a materializing sibling ([[format]] / [[contentEncoding]])
governs it; the regex lives only
in the validator, emitted as a **compiled constant** (below) rather than
recompiled per call.

## Validator mapping

Per **P10**/**P11**. A single "does the regex match?" predicate against a
compiled-once pattern, identical in both directions (shared `Validate`,
**P12**). Each language compiles the pattern **once** (module/package
init) and reuses it; the flags/method are the P1-pinned choices above, and
`<pattern>` denotes the **gate-normalized** form — `\s`/`\S` already
expanded to the explicit ASCII class and `$` rewritten per target
(`\Z`/`\z`), inline flags already rejected — so no runtime row has to cope
with them.

| Language | Strategy |
|---|---|
| Go | Package-level `var patRe = regexp.MustCompile(<pattern>)` (compiled once at init; the load-time gate already proved it compiles). The shared `Validate` checks `if !patRe.MatchString(v) { push(Violation{Path, Reason: fmt.Sprintf("must match pattern %q, got %q", <pattern>, v)}) }` — `MatchString` is unanchored; RE2 is ASCII-class + rune-`.`. Collected into one `PayloadValidationError` application failure. |
| TypeScript | Module-level ``const PAT_RE = /<pattern>/u;`` (or `new RegExp(<pattern>, "u")` when the literal can't be spelled). **The `u` flag is mandatory** (code-point `.`; verified). ``if (!PAT_RE.test(v)) push(Violation{path, reason: `must match pattern <pattern>, got ${JSON.stringify(v)}`})``. `test` is unanchored and — with no `g` flag — stateless. Throw one `PayloadValidationError` application failure. |
| Python | A module-level `_PATTERN_<HEX> = re.compile(<pattern>, re.ASCII)` (with the `$`→`\Z` normalization applied), keyed by the pattern text so identical patterns share one compiled instance per module. Both directions of the model's `_<Model>TransferTypeConverter` inline the check — `if _PATTERN_<HEX>.search(value) is None: violations.append(Violation(path=…, reason="must match pattern <pattern>, got " + _quote(value)))` (the authored pattern in a plain string literal, never the `\Z`-rewritten `_PATTERN_<HEX>.pattern`) — collected into the single `PayloadValidationError` application failure (**PRINCIPLES Python §2/§3**). The comparison is emitted inline rather than behind a runtime helper, the same way TypeScript emits it. **`re.search` (unanchored — never `re.match`, which anchors the start, or `fullmatch`), `re.ASCII` (ASCII `\d\w\s`).** |
| Java | Static `private static final Pattern PAT_RE = Pattern.compile(<pattern>);` (**default flags** — ASCII `\d\w\s`, code-point `.`; with the `$`→`\z` normalization applied). The per-POJO collecting deserializer (PRINCIPLES Java §5) reads the `String` and checks `if (!PAT_RE.matcher(v).find())`, pushing a `Violation{path, "must match pattern " + <pattern> + ", got " + v}` (the authored pattern, never the `\z`-rewritten one) into the single `PayloadValidationError` application failure. **`Matcher.find` (unanchored), never `matches()`** (which anchors the whole input — verified footgun). Not bean-validation `@Pattern`. |

The compiled identifiers are also part of **P15**. Go derives
`<model><position>Pattern` (and the parallel `Format` / `ContentEncoding`
families); Java derives `<POSITION>_PATTERN` / `_FORMAT` and the
`_CONTAINS_*` families. A property position begins with the member's emitted
identifier, then appends stable `Item` / `Contains` suffixes, so an
`x-go-name` or `x-java-name` override moves the whole family. Every such name
enters the emitted-name manifest before rendering; a collision rejects rather
than producing a duplicate package variable or class field.

**Informative `reason` strings.** The `Violation` `reason` names the
**pattern and the offending value** (`must match pattern "^[a-z]+$", got
"AB1"`), per the [[maximum]] convention. The pattern is an emitted
compile-time constant; the value is interpolated at runtime. The quoted
pattern is the **authored (loader-normalized) text** in every target — the
per-target `$`→`\z`/`\Z` rewrite (and any other host-engine respelling) is
used only for matching, never shown in the reason, so all targets print the
same pattern for the same schema.

**The emitted pattern is a string literal in the target's own escape
grammar.** The pattern text can contain any code point the schema author wrote,
including a non-printable one (a no-break space, a combining mark, a soft
hyphen). Each target's literal is spelled with that target's escapes — Go and
Python have no `\u{…}` form, so a non-printable code point must be emitted
either verbatim or as an escape the target accepts, never in a Rust-flavored
one. An unspellable escape does not produce a wrong verdict; it produces a
package that does not compile.

**A runtime throw is a `Violation`.** No check on this path may let an
exception escape the field handler. A regex engine that throws rather than
returning a verdict — Java's matcher recursing on a deeply-backtracking pattern
throws an `Error`, not an exception — must be caught at the member and pushed as
a `Violation` on that member's path. An escaping throw loses the path *and*
every violation already aggregated for the payload, which is the aggregate
**P11** exists to deliver, and it leaves the two directions reporting different
violation sets (**P11.1**).

**Why compile-once.** Recompiling a regex per (de)serialize call is a
needless cost (P2 favors ergonomics/idiom, and a package-level compiled
pattern *is* the idiom in all four); the load-time gate is what lets the
emitted `MustCompile`/`Pattern.compile` be unguarded — its job is to
turn any runtime compile failure into a load-time reject. An admitted pattern
that a target's engine then refuses is therefore a **gate defect**, never a
licensed exception. Leading-`]` classes and quantifiers stacked on an
exact-count quantifier are gate-rejected and pinned in the corpus.

### Serialize-side (P12)

The match is a shared-`Validate` predicate, so it **re-runs before emit**
over the decoded value — a model constructed with a non-matching string
(a Go `string` / Java `String` / Python `str` set to an off-pattern value
in memory) fails serialize with the same aggregated primitive rather than
emitting an invalid value. Real teeth in every target, since constructing a
value in memory is unchecked in all four — a Go struct literal, a TS object
literal, a Java setter, and an inert Python dataclass alike bypass the parse
adapter, so the only place the match can be re-asserted is before emit. No
parse-adapter-only or encode-adapter-only logic: the match is pure and
direction-agnostic.

**On a materialized node** ([[format]] temporal / [[contentEncoding]]
bytes) the decoded value is not a `string`, so the regex matches the
**canonical wire string** — the form the encode adapter produces for that value
— at **both** boundaries: on parse the value is parsed and re-canonicalized
*before* the predicate runs, and on serialize the re-serialized wire string is
matched **before emit**. Still one predicate, identical in both directions.
Matching the incoming form on parse and the canonical form on serialize would
make that false: `PT90M` and `PT1H30M` are one [[format]] `duration` value whose
canonical spelling is `PT1H30M`, so a payload could be admitted and then be
unre-emittable — a **P1** accept-set defect, not a rounding of one, and the
operand drift **P12.2** names outright.

That same string is the closed-value comparison's operand too ([[const]] /
[[enum]]), and it is projected **once per member**: two independent projections
are two operands (**P12.2**), and a second projection in one scope is a
redeclaration.

A **literal** ([[const]], [[enum]], [[default]]) on such a node is measured
against the same string: the loader canonicalizes it **before** the pattern
check, so the spelling checked is the spelling the emitted constant carries.
Checking the authored spelling instead is wrong in both directions — it accepts
`{format:"duration", pattern:"^PT90M$", const:"PT90M"}`, whose emitted constant
`PT1H30M` its own emitted pattern rejects, and it refuses
`{format:"duration", pattern:"^PT1H30M$", const:"PT90M"}`, which is satisfiable.

## Property-testing matrix

### Accepted (positive)

| Shape | Example |
|---|---|
| Anchored pattern (`$` normalized per target) | `{type:"string", pattern:"^[a-z]+$"}` |
| Unanchored substring | `{type:"string", pattern:"cat"}` (matches `"the cat sat"`) |
| ASCII digit/word class | `{type:"string", pattern:"^\\d{3}-\\w{4}$"}` |
| `\s`/`\S` (normalized to ASCII class) | `{type:"string", pattern:"^\\s+$"}`, `{…, pattern:"\\S"}`, `{…, pattern:"[^\\s]"}` |
| Explicit whitespace class (unchanged) | `{type:"string", pattern:"^[ \\t]+$"}` |
| Empty pattern (no-op) | `{type:"string", pattern:""}` |

### Rejected at load time (negative)

| Reason | Example |
|---|---|
| Value not a string | `pattern:5`, `pattern:true`, `pattern:["a"]` |
| Type mismatch (P7.1) | `{type:"integer", pattern:"^\\d+$"}` |
| Non-portable: lookahead | `{type:"string", pattern:"(?=.*[A-Z]).+"}` |
| Non-portable: lookbehind | `{type:"string", pattern:"(?<=x)y"}` |
| Non-portable: backreference | `{type:"string", pattern:"(a)\\1"}` |
| Inline flag group (JS can't compile) | `{type:"string", pattern:"(?i)^cat$"}` |
| `\S` in a multi-member class (open complement) | `{type:"string", pattern:"[\\S.]"}`, `{…, pattern:"[\\S\\d]"}` |
| Non-portable escape | `{…, pattern:"^\\d{3}\\-\\d{4}$"}` (escaped `-`), `{…, pattern:"^\\p{L}+$"}` |
| Non-portable assertion/capture | `{…, pattern:"\\Acat"}`, `{…, pattern:"(?<word>cat)"}` |
| Class operation / POSIX class | `{…, pattern:"[a-z&&[^x]]"}`, `{…, pattern:"[[:alpha:]]"}` |
| Ambiguous unbounded repetition (D7) | `{…, pattern:"^([a-z]+)+$"}`, `{…, pattern:"^(\\w+\\s*)+$"}` |
| Literal fails pattern | `{type:"string", pattern:"^[a-z]+$", const:"AB"}`, `{…, default:"9"}` |

### Runtime fixtures (validator)

The per-`(pattern, instance)` match behavior — unanchored search, ASCII
class semantics, code-point `.`, and the `\s`/`\S` and `$` normalizations —
is specified by the **conformance corpus**
(`json-schema/corpora/pattern_conformance/corpus.json`). Each row states the
pattern, the instance, and the expected disposition, including which patterns
the gate must reject; a row's expectation is data, never a special case
carried in a test's source. That corpus is this keyword's regression suite —
new edge cases are added there, not enumerated here.

`tests/json_schema_corpus_runtime.rs` is the executable consumer: it generates
one member per accepted pattern and runs all 97 runtime rows through Go,
TypeScript, Python, and Java. The 44 `expect_gate_reject` rows exercise the gate
helper directly; they are not end-to-end loader coverage because no runtime
pattern is emitted for a rejected schema.
`tests/json_schema_conformance_manifest.rs` adds the integration/serialize
case and records `pattern.parse` in its fixed coverage ledger.

Fixtures outside the corpus (validator integration, not pure matching):
- Combined with a failing [[minLength]]/[[maxLength]] or sibling field →
  **all** reported in one shot (**P11**); serialize of an off-pattern
  in-memory value → rejected before emit (**P12**).

## Interactions

- **[[minLength]] / [[maxLength]]**: independent string assertions; all
  present keywords apply and aggregate. We do **not** attempt
  regex↔length satisfiability (a pattern like `^.{5,}$` implies a minimum
  length, but general regex-length reasoning is undecidable and out of
  scope) — each is checked on its own.
- **[[type]]**: gates applicability — `pattern` is meaningful only for
  `string`; a mismatch is a load reject (**P7.1**). `pattern` does not
  force the emitted type to `string`: a materializing sibling ([[format]] /
  [[contentEncoding]]) may replace it with a native construct while the
  regex still checks the wire string.
- **[[const]] / [[default]] / [[enum]]**: a supplied string literal MUST
  match `pattern` at load (rule above) — the regex half of the deferred
  literal-vs-constraint obligation. A **closed value set** on the same node
  does not remove the match: where a target gives the closed set its own named
  type, the predicate is evaluated over that value's underlying `string`.
  Handing the named type to the regex primitive is a compile error, and
  dropping the predicate silently makes the emitted validator depend on the
  load-time check having been exact.
- **[[patternProperties]]**: the *key-space* regex keyword (temporarily
  unsupported). It faces the **same** RE2-vs-ECMA-262 dialect gap this
  keyword manages for values — see [[patternProperties]], which points
  here for the confined, managed case.
- **[[format]]**: the named-shape string keyword. Its regex-lowered
  formats (`uuid`, `ipv4`, `ipv6`, and the syntactic pass of the temporal
  formats) are held to this keyword's RE2-safe subset — each pinned pattern
  passes this gate **unchanged** — and use the same compile-once mechanism and
  per-target end-anchor rewrite; temporal formats add a shared calendar
  predicate on top. Both may appear on one node — the value must satisfy both.

## Ecosystem variance

| Source dialect | Action |
|---|---|
| JSON Schema 2020-12 | Native (ECMA-262 dialect). Portable subset accepted; non-portable constructs rejected (deferred). |
| OpenAPI 3.1 | Adopts 2020-12 — same. |
| OpenAPI 3.0 / draft-4 | `pattern` present since draft-4, same ECMA-262 intent. The rewrite an imported pattern needs is wider than the backtracking constructs: escaped punctuation (`\d{3}\-\d{4}`), Unicode-property escapes (`\p{L}`), POSIX classes (`[[:alpha:]]`), the `[\s\S]` "any character" idiom and ambiguous unbounded repetitions (`(\w+\s*)+`) are all common in authored schemas and all reject. `\s`/`\S`, `.` and `$` are handled automatically by normalization. |
| Swagger 2.0 | Same as OAS 3.0. |

The cross-engine divergences enumerated in the gate rules are handled: the
compile gate + `regex-syntax` AST checks **reject** the non-portable
constructs and **normalize** `\s`/`\S` (→ explicit ASCII class), `.`
(→ `[^\n]`) and `$` (→ per-target end-anchor), which is what makes the four
runtimes agree value-for-value on every corpus pair. Beyond those rules, the open edges are
**lone-surrogate wire strings** and any divergence that appears only past the
corpus's length ceiling — short runtime examples alone cannot prove a repeated
pattern safe. Word-boundary patterns are therefore gate-rejected, including
the Unicode-adjacent witness; the `.`-newline corner is normalized and
runtime-covered. Every accepted row the corpus contains is executed in all
four current runtimes; every rejected row is rechecked by the loader gate. New
acceptance edges belong in that corpus before the gate is widened.

### Prospective targets (.NET, Ruby)

Not current targets, but both are planned. Conformance was verified against
the same corpus (`json-schema/corpora/pattern_conformance/`), and both are feasible
with **per-target emission transforms only — no new gate rules**. The
findings (record them here so the recipe survives to implementation time):

| Target | Engine | Runtime config + transforms to match the pinned semantics |
|---|---|---|
| **.NET** | `System.Text.RegularExpressions` | `Regex.IsMatch(v, p, RegexOptions.ECMAScript)` — `IsMatch` is unanchored; **`ECMAScript`** makes `\d\w\s` ASCII (Unicode by default otherwise). `$`→**`\z`** (its `\Z` is the *lenient* one, reverse of Java). **Astral `.` is a divergence:** .NET `.` matches one UTF-16 unit and there is **no `u`-flag equivalent**, so `^a.b$` misses `"a😀b"` — the emitter must rewrite each `.`→`(?:[\uD800-\uDBFF][\uDC00-\uDFFF]|.)` (verified 70/72 → 72/72 with the rewrite). This is the same problem JS has, solved by an explicit rewrite instead of a flag. |
| **Ruby** | Onigmo (`Regexp`) | `re.match?(v)` — unanchored. `\d`/`\w`/`\s` are ASCII by default (good), and `.` is a code point (good). But `^`/`$` are **always line anchors** (no non-multiline mode), so normalize **`^`→`\A`, `$`→`\z`** (never `\Z` — that is the lenient one). Word boundaries are already outside the accepted subset. `\s`/`\S` normalization and the `$`/`\S`-class rules carry over unchanged. |

Neither needs a new *gate* rule — the existing rejects already cover them, and
both accept the normalized output. The `.NET` astral rewrite and the Ruby
`^`/`$`→`\A`/`\z` rewrite are additions to those targets'
*emitters*, mirroring how each current target already applies its own
per-engine flag/anchor treatment. Note the astral rewrite must target the
**negated class** the gate emits, not `.`: no `.` survives normalization, so a
recipe written against `.` would never fire.

## Open questions

1. **Widen the accepted subset.** The v1 gate rejects backtracking
   constructs (lookaround/backreferences), inline flag groups, `\S`
   inside a multi-member class, and the rule 5-8 categories. Each is a
   candidate for later admission via a semantics-preserving rewrite — `(?i)` →
   case-fold expansion or a real flag channel, backtracking → a portable
   rewrite where one exists or a shared engine, multi-member `[\S…]` → a
   positive form if one is found, an escaped punctuation character or a POSIX
   class → its explicit spelling at emit time —
   each gated on the conformance corpus (`json-schema/corpora/pattern_conformance/`)
   still agreeing across all targets (incl. the prospective .NET/Ruby).
   Revisit on demand (mirrors [[multipleOf]]'s fractional-divisor and
   [[patternProperties]]' single-pattern carve-outs). `\s`/`\S`, `.` and `$`
   were on this list and are now **resolved** by normalization.

## See also

- [[minLength]] / [[maxLength]] — the other string assertions (length);
  independent of `pattern`.
- [[patternProperties]] — the key-space regex keyword; same dialect gap,
  points here for the managed value-level case.
- [[type]] — supplies the emitted `string`; gates applicability; owns the
  cross-language conformance-suite open question.
- [[const]], [[default]], [[enum]] — supplied string literals validated
  against `pattern` at load.
- [[multipleOf]] — the sibling "support the portable subset, reject the
  hazardous form, deferred not excluded" decision posture.
- [[maximum]] — the `reason`-string convention.
