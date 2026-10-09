//! The RE2-safe portable subset for the JSON-Schema `pattern` keyword: a
//! load-time compile gate + a `regex-syntax` AST conformance pass (portability
//! rejects plus the `\s`/`\S` and `.` normalizations, all target-agnostic) plus
//! the per-target `$` end-anchor rewrite applied by the Python/Java backends.
//! See `specs/json-schema/features/pattern.md` for the authoritative rules and
//! `specs/json-schema/corpora/pattern_conformance/corpus.json` for the corpus
//! that pinned every rule below.
//!
//! The gate is pure Rust — it compiles the pattern with the `regex` crate (the
//! same no-backtracking family as Go's RE2) and then walks the `regex-syntax`
//! AST. **Compiling under Rust is necessary but not sufficient:** Rust's
//! accepted language is a *superset* of ECMA-262-with-`u`, Python `re`, and
//! `java.util.regex` in several directions, so the AST pass carries an explicit
//! rule for every construct where a Rust-compilable pattern would either fail to
//! compile in a target or match differently there. Every rule below was measured
//! against Go 1.26 / Node (`u` flag) / CPython `re.ASCII` / OpenJDK 21, not
//! reasoned about. No Go (or other) toolchain is involved at load time.
//!
//! Rejects (each verified to break or diverge in at least one target):
//! - backtracking constructs the `regex` crate cannot compile (lookaround,
//!   backreferences, …) — the compile gate;
//! - inline flag groups (`(?i)` / `(?flags:…)`) — JS cannot compile them;
//! - `\S` inside a multi-member character class — no portable positive form;
//! - **non-portable escapes**: `\-`, `\_`, `\"`, `\ `, `\#`, `\&`, `\~`, … (JS
//!   `u` mode restricts `IdentityEscape` to the syntax characters and `/`),
//!   `\a` (JS), `\v` (Java reads it as a *vertical whitespace class*, not
//!   U+000B), `\0`/octal (Java), `\uFFFF` (Go), `\x{…}` (JS + Python),
//!   `\p{…}`/`\pL` (Python; `\pL` also JS);
//! - **lone `{`, `}`, `]`** outside a class — JS `u` (and Java, for `{`) treat
//!   them as malformed quantifier/class brackets;
//! - `\A` / `\z` / word-boundary assertions — either fail to compile in a
//!   target or disagree beside non-ASCII input;
//! - **named capture groups** — `(?P<n>…)` breaks Java, `(?<n>…)` breaks Python;
//! - **POSIX classes** (`[[:alpha:]]`), **nested classes** (`[a[\s]]`) and
//!   **class set operations** (`[a&&b]`, `[a--b]`, `[a~~b]`) — JS cannot compile
//!   them and Go/Python/Java each read them differently;
//! - **nested / ambiguous quantifiers** (`(a+)+`, `(a?)*`, `(a|a)*`) — "regular"
//!   is a property of the *language*, not the *engine*: these are linear in
//!   Go/RE2 and exponential in the three backtracking targets (decision D7).
//!
//! Normalizations (rewritten in the emitted pattern rather than rejected):
//! - `\s`/`\S` → the explicit ASCII whitespace class, in every placement
//!   including inside a class;
//! - `.` → `[^\n]` — the four engines' "any character except a line terminator"
//!   sets differ (Go/Python exclude only `\n`; JS also `\r`, U+2028, U+2029;
//!   Java also `\r`, U+0085, U+2028, U+2029);
//! - `$` → `\Z` (Python) / `\z` (Java) at emit time, via [`rewrite_end_anchor`].

use regex_syntax::ast::{self, Ast};

/// The canonical ASCII whitespace member list (no surrounding brackets), spliced
/// in for `\s`/`\S`. `\x0B` (U+000B vertical tab) is written as a hex escape,
/// not `\v`, to avoid the shorthand-class ambiguity where Go/RE2's `\s` omits
/// U+000B while JS/Python/Java include it — spelling it makes every engine
/// agree. Deliberately ASCII (ECMA-262's Unicode spaces dropped), consistent
/// with the ASCII `\d`/`\w` pinning.
const WS: &str = r"\t\n\x0B\f\r ";
const DIGIT: &str = "0-9";
const WORD: &str = "A-Za-z0-9_";

/// The portable spelling of `.`. Go/RE2 and Python `re` exclude only `\n` from
/// `.`; JS additionally excludes `\r`, U+2028 and U+2029; Java additionally
/// excludes `\r`, U+0085, U+2028 and U+2029. Splicing the explicit negated class
/// pins every engine to the RE2/Python reading (measured: all four agree on
/// `\r`, U+0085, U+2028, U+2029, `\n` and an astral code point after the
/// rewrite).
const DOT_CLASS: &str = r"[^\n]";

/// Punctuation whose *escaped* form every target accepts **outside** a character
/// class. ECMA-262 `u` mode is the binding constraint: `IdentityEscape` is
/// restricted to `SyntaxCharacter` (`^ $ \ . * + ? ( ) [ ] { } |`) plus `/`, so
/// Rust-legal escapes such as `\-`, `\_`, `\"`, `\ `, `\#`, `\&` and `\~` are
/// `SyntaxError`s in JavaScript.
const PORTABLE_ESCAPES: &[char] = &[
    '^', '$', '\\', '.', '*', '+', '?', '(', ')', '[', ']', '{', '}', '|', '/',
];

/// Additional escapes legal **inside** a character class: ECMA-262 `u` mode's
/// `ClassEscape` admits `-` on top of the list above.
const PORTABLE_CLASS_ESCAPES: &[char] = &['-'];

/// Characters that must not appear **verbatim** outside a character class: JS
/// `u` mode reads a lone `}` or `]` as a stray quantifier/class bracket, and a
/// lone `{` as an incomplete quantifier (Java rejects `{` too). Rust and Python
/// treat all three as literals.
const LONE_BRACKETS: &[char] = &['{', '}', ']'];

#[derive(Debug, Clone)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

/// A load-time rejection of a non-portable `pattern`, carrying a fix-it message.
#[derive(Debug)]
pub struct PatternError(pub String);

/// The load-time compile gate + portability pass + normalization. Returns the
/// normalized pattern (Perl whitespace classes expanded to the explicit ASCII
/// class, `.` expanded to `[^\n]`, the `$` end-anchor kept canonical for the
/// per-target backend rewrite), or a `PatternError` naming the non-portable
/// construct and the portable spelling to use instead. See the module docs for
/// the full rule list.
pub fn gate_and_normalize(pattern: &str) -> Result<String, PatternError> {
    // 1. Compile gate. Success ⟹ the regular (no-backtracking) subset. This is
    //    necessary but *not* sufficient — step 2 carries the rules for every
    //    construct Rust accepts that a target engine does not.
    if let Err(error) = regex::Regex::new(pattern) {
        let detail = error
            .to_string()
            .lines()
            .find(|line| line.contains("error:"))
            .map(str::trim)
            .unwrap_or("not a regular expression")
            .to_string();
        return Err(PatternError(format!(
            "pattern {pattern:?} is not portable across the supported engines ({detail}); \
             only the regular (no-backtracking) subset is supported \
             (no lookahead/lookbehind/backreferences)"
        )));
    }

    // 2. Walk the AST (guaranteed to parse, since it just compiled) for the
    //    portability rejects and the `\s`/`\S` and `.` normalizations.
    let ast = ast::parse::Parser::new()
        .parse(pattern)
        .map_err(|error| PatternError(format!("pattern {pattern:?} failed to parse: {error}")))?;

    let mut edits = Vec::new();
    walk_normalize(&ast, pattern, &mut edits)?;
    Ok(apply_edits(pattern, edits))
}

/// The target-neutral `must match pattern "<pattern>"` lead of a `pattern`
/// violation reason. `pattern` is the loader-normalized text (never a
/// per-target `$` rewrite), quoted as a JSON string — the same form every
/// target quotes the offending value in — so every target prints the same
/// bytes for the same schema. Backends embed it as a literal in their own
/// string grammar and append `, got <JSON-quoted value>` for value positions.
/// See `specs/json-schema/features/pattern.md` ("Informative reason strings").
pub fn violation_reason(pattern: &str) -> String {
    let quoted = serde_json::to_string(pattern).expect("a string always serializes");
    format!("must match pattern {quoted}")
}

/// Rewrite every unescaped `$` end-anchor assertion to `replacement` (Python
/// `\Z`, Java `\z`), leaving `^`, escaped `\$`, and literals untouched. Applied
/// by the Python/Java backends at emit time; Go/JS keep `$` (already
/// exception-free). The input is the loader-normalized pattern, so it is known
/// to parse. On the unexpected event it does not, the pattern is returned
/// unchanged (the load gate already accepted it).
pub fn rewrite_end_anchor(pattern: &str, replacement: &str) -> String {
    let Ok(ast) = ast::parse::Parser::new().parse(pattern) else {
        return pattern.to_string();
    };
    let mut edits = Vec::new();
    collect_end_anchors(&ast, &mut edits, replacement);
    apply_edits(pattern, edits)
}

fn collect_end_anchors(ast: &Ast, edits: &mut Vec<Edit>, replacement: &str) {
    match ast {
        Ast::Assertion(assertion) => {
            if matches!(
                assertion.kind,
                ast::AssertionKind::EndLine | ast::AssertionKind::EndText
            ) {
                edits.push(Edit {
                    start: assertion.span.start.offset,
                    end: assertion.span.end.offset,
                    replacement: replacement.to_string(),
                });
            }
        }
        Ast::Concat(concat) => {
            for child in &concat.asts {
                collect_end_anchors(child, edits, replacement);
            }
        }
        Ast::Alternation(alternation) => {
            for child in &alternation.asts {
                collect_end_anchors(child, edits, replacement);
            }
        }
        Ast::Repetition(repetition) => collect_end_anchors(&repetition.ast, edits, replacement),
        Ast::Group(group) => collect_end_anchors(&group.ast, edits, replacement),
        _ => {}
    }
}

/// Walk the AST rejecting every non-portable construct and collecting the
/// `\s`/`\S` and `.` rewrite edits.
fn walk_normalize(ast: &Ast, pattern: &str, edits: &mut Vec<Edit>) -> Result<(), PatternError> {
    match ast {
        Ast::Empty(_) => Ok(()),
        // Bare inline flag directive `(?i)` / `(?m)` … — JS cannot compile it.
        Ast::Flags(_) => Err(inline_flag_error(pattern)),
        Ast::Literal(literal) => check_literal(literal, pattern, false),
        // `.` means a different set in each engine — splice the explicit class.
        Ast::Dot(span) => {
            edits.push(Edit {
                start: span.start.offset,
                end: span.end.offset,
                replacement: DOT_CLASS.to_string(),
            });
            Ok(())
        }
        Ast::Assertion(assertion) => check_assertion(assertion, pattern),
        // `\p{…}` / `\pL`: Python `re` has no Unicode-property escape at all and
        // `\pL` is also a JS `SyntaxError` (JS requires the braced form).
        Ast::ClassUnicode(_) => Err(PatternError(format!(
            "pattern {pattern:?} uses a Unicode property escape (`\\p{{…}}` / `\\pL`), \
             which Python's `re` cannot compile (and the one-letter form is a JavaScript \
             SyntaxError); spell the intended set as an explicit character class"
        ))),
        Ast::ClassPerl(perl) => {
            let members = perl_class_members(&perl.kind);
            let replacement = if perl.negated {
                format!("[^{members}]")
            } else {
                format!("[{members}]")
            };
            edits.push(Edit {
                start: perl.span.start.offset,
                end: perl.span.end.offset,
                replacement,
            });
            Ok(())
        }
        Ast::ClassBracketed(class) => handle_class(class, pattern, edits),
        Ast::Concat(concat) => {
            for child in &concat.asts {
                walk_normalize(child, pattern, edits)?;
            }
            Ok(())
        }
        Ast::Alternation(alternation) => {
            for child in &alternation.asts {
                walk_normalize(child, pattern, edits)?;
            }
            Ok(())
        }
        Ast::Repetition(repetition) => {
            check_repetition(repetition, pattern)?;
            walk_normalize(&repetition.ast, pattern, edits)
        }
        Ast::Group(group) => {
            match &group.kind {
                // `(?flags:…)` sets flags on a group — reject (JS cannot
                // compile it). `(?:…)` is a bare non-capturing group (empty flag
                // list) — fine.
                ast::GroupKind::NonCapturing(flags) if !flags.items.is_empty() => {
                    return Err(inline_flag_error(pattern));
                }
                // `(?P<n>…)` is a `PatternSyntaxException` in Java; `(?<n>…)` is
                // a `PatternError` in Python. Neither spelling is portable.
                ast::GroupKind::CaptureName { name, .. } => {
                    let name = &name.name;
                    return Err(PatternError(format!(
                        "pattern {pattern:?} uses a named capture group `{name}`, which has no \
                         portable spelling (`(?P<{name}>…)` fails to compile in Java, \
                         `(?<{name}>…)` fails in Python); use a non-capturing group `(?:…)` \
                         — `pattern` only ever asks whether the regex matches, so captures \
                         are never read"
                    )));
                }
                _ => {}
            }
            walk_normalize(&group.ast, pattern, edits)
        }
    }
}

/// Zero-width assertions: `^` and `$` are portable (`$` is normalized per
/// target at emit time). Word boundaries are not: Java's default `\b` uses a
/// Unicode-aware word definition while the other targets use the portable
/// ASCII `\w` set, so inputs such as `éfoo` disagree. `\A`, `\z` and the
/// extended `\b{…}` family also have no shared spelling.
fn check_assertion(assertion: &ast::Assertion, pattern: &str) -> Result<(), PatternError> {
    use ast::AssertionKind::*;
    match assertion.kind {
        StartLine | EndLine => Ok(()),
        WordBoundary | NotWordBoundary => Err(PatternError(format!(
            "pattern {pattern:?} uses a word-boundary assertion (`\\b` / `\\B`), whose \
             meaning beside non-ASCII text differs across the supported engines (Java uses \
             Unicode word boundaries while the portable word class is ASCII); avoid word \
             boundaries and spell the intended ASCII delimiter structure explicitly"
        ))),
        StartText => Err(PatternError(format!(
            "pattern {pattern:?} uses the `\\A` start-of-text anchor, which is a JavaScript \
             SyntaxError; use `^` (the generator never enables multiline mode, so `^` already \
             means start-of-input in every target)"
        ))),
        EndText => Err(PatternError(format!(
            "pattern {pattern:?} uses the `\\z` end-of-text anchor, which is a JavaScript \
             SyntaxError and a Python `re` error; use `$` (the generator rewrites it to \
             `\\Z`/`\\z` per target so it always means end-of-input)"
        ))),
        WordBoundaryStart
        | WordBoundaryEnd
        | WordBoundaryStartAngle
        | WordBoundaryEndAngle
        | WordBoundaryStartHalf
        | WordBoundaryEndHalf => Err(PatternError(format!(
            "pattern {pattern:?} uses a Rust/Go-only word-boundary assertion \
             (`\\b{{start}}` / `\\b{{end}}` / `\\<` / `\\>`), which no other target engine \
             can compile; avoid word-boundary assertions and spell the intended ASCII \
             delimiter structure explicitly"
        ))),
    }
}

/// Validate one literal's *spelling*. The character it denotes is irrelevant —
/// what differs across engines is which escape forms parse.
fn check_literal(
    literal: &ast::Literal,
    pattern: &str,
    in_class: bool,
) -> Result<(), PatternError> {
    use ast::{HexLiteralKind, LiteralKind, SpecialLiteralKind};
    match &literal.kind {
        LiteralKind::Verbatim => {
            if !in_class && LONE_BRACKETS.contains(&literal.c) {
                let c = literal.c;
                return Err(PatternError(format!(
                    "pattern {pattern:?} contains a lone `{c}` outside a character class, which \
                     JavaScript's `u` mode rejects as a stray quantifier/class bracket \
                     (Java rejects a lone `{{` too); escape it as `\\{c}`"
                )));
            }
            Ok(())
        }
        // An escaped punctuation character (`\*`, `\-`, `\_`, …).
        LiteralKind::Meta | LiteralKind::Superfluous => {
            let c = literal.c;
            if PORTABLE_ESCAPES.contains(&c) || (in_class && PORTABLE_CLASS_ESCAPES.contains(&c)) {
                return Ok(());
            }
            Err(PatternError(format!(
                "pattern {pattern:?} escapes `{c}` as `\\{c}`, which is a JavaScript SyntaxError \
                 (ECMA-262 `u` mode only allows escaping `^ $ \\ . * + ? ( ) [ ] {{ }} | /`, \
                 plus `-` inside a character class); write `{c}` unescaped"
            )))
        }
        LiteralKind::Octal => Err(PatternError(format!(
            "pattern {pattern:?} uses an octal escape, which Java rejects \
             (`Illegal octal escape sequence`); use the two-digit hex form `\\xHH`"
        ))),
        LiteralKind::HexFixed(HexLiteralKind::X) => Ok(()),
        LiteralKind::HexFixed(HexLiteralKind::UnicodeShort) => Err(PatternError(format!(
            "pattern {pattern:?} uses a `\\uFFFF` escape, which Go's `regexp` cannot compile; \
             use the two-digit hex form `\\xHH` for ASCII or write the character literally"
        ))),
        LiteralKind::HexFixed(HexLiteralKind::UnicodeLong) => Err(PatternError(format!(
            "pattern {pattern:?} uses a `\\UFFFFFFFF` escape, which only Rust and Go accept; \
             write the character literally"
        ))),
        LiteralKind::HexBrace(_) => Err(PatternError(format!(
            "pattern {pattern:?} uses a braced hex escape (`\\x{{…}}`), which is a JavaScript \
             SyntaxError and a Python `re` error; write the character literally \
             (the emitted JavaScript regex carries the `u` flag, so an astral character \
             is one code point)"
        ))),
        LiteralKind::Special(SpecialLiteralKind::Bell) => Err(PatternError(format!(
            "pattern {pattern:?} uses the `\\a` bell escape, which is a JavaScript SyntaxError; \
             use `\\x07`"
        ))),
        LiteralKind::Special(SpecialLiteralKind::VerticalTab) => Err(PatternError(format!(
            "pattern {pattern:?} uses the `\\v` escape, which Java reads as a *vertical \
             whitespace class* (it matches `\\n`, `\\r`, U+0085, U+2028, U+2029 as well) while \
             every other target reads it as U+000B; use `\\x0B`"
        ))),
        LiteralKind::Special(SpecialLiteralKind::Space) => Err(PatternError(format!(
            "pattern {pattern:?} uses the verbose-mode `\\ ` escape, which only Rust accepts; \
             write a plain space"
        ))),
        LiteralKind::Special(
            SpecialLiteralKind::FormFeed
            | SpecialLiteralKind::Tab
            | SpecialLiteralKind::LineFeed
            | SpecialLiteralKind::CarriageReturn,
        ) => Ok(()),
    }
}

fn handle_class(
    class: &ast::ClassBracketed,
    pattern: &str,
    edits: &mut Vec<Edit>,
) -> Result<(), PatternError> {
    let spelling = &pattern[class.span.start.offset..class.span.end.offset];
    let after_open = spelling
        .strip_prefix('[')
        .and_then(|body| body.strip_prefix('^').or(Some(body)))
        .unwrap_or(spelling);
    if after_open.starts_with(']') {
        return Err(PatternError(format!(
            "pattern {pattern:?} places `]` first in a character class, which JavaScript's `u` mode rejects; escape it as `\\]`"
        )));
    }
    let mut items = Vec::new();
    collect_class_set(&class.kind, pattern, &mut items)?;

    // A sole Perl class reduces to its explicit ASCII member set. Rust's
    // compile gate uses Unicode `\d`/`\w`, while the emitted runtimes are pinned
    // to ASCII; normalization keeps both the literal checks and runtimes on the
    // same operand.
    if items.len() == 1
        && let ast::ClassSetItem::Perl(perl) = items[0]
    {
        let effective_negated = class.negated ^ perl.negated;
        let members = perl_class_members(&perl.kind);
        let replacement = if effective_negated {
            format!("[^{members}]")
        } else {
            format!("[{members}]")
        };
        edits.push(Edit {
            start: class.span.start.offset,
            end: class.span.end.offset,
            replacement,
        });
        return Ok(());
    }

    // Multi-member class: a positive Perl class becomes its bare member list;
    // an open-ended complement has no portable positive form — reject.
    for item in &items {
        if let ast::ClassSetItem::Perl(perl) = item {
            if perl.negated {
                return Err(PatternError(format!(
                    "pattern {pattern:?} uses a negated Perl class inside a multi-member character class \
                     (an open-ended complement with no portable positive form); \
                     spell the intended set explicitly (e.g. `[^\\t\\n\\x0B\\f\\r ]`)"
                )));
            }
            edits.push(Edit {
                start: perl.span.start.offset,
                end: perl.span.end.offset,
                replacement: perl_class_members(&perl.kind).to_string(),
            });
        }
    }
    Ok(())
}

fn perl_class_members(kind: &ast::ClassPerlKind) -> &'static str {
    match kind {
        ast::ClassPerlKind::Digit => DIGIT,
        ast::ClassPerlKind::Space => WS,
        ast::ClassPerlKind::Word => WORD,
    }
}

/// Flatten a `ClassSet` into its leaf items, rejecting every non-portable class
/// construct on the way down. Unlike a plain flatten this *cannot* silently drop
/// a subtree: a nested class, a POSIX class, a Unicode property or a set binary
/// operation is an error, not an opaque leaf — which is what previously let a
/// `\s`/`\S` inside `[a[\s]]`, `[[\S]]` or `[\w&&\s]` escape normalization
/// entirely.
fn collect_class_set<'a>(
    set: &'a ast::ClassSet,
    pattern: &str,
    out: &mut Vec<&'a ast::ClassSetItem>,
) -> Result<(), PatternError> {
    match set {
        ast::ClassSet::Item(item) => collect_class_item(item, pattern, out),
        // `&&` / `--` / `~~`: a JavaScript SyntaxError, a literal member list in
        // Python, an intersection in Java (`&&` only) and in Go/Rust — measured
        // to produce three different answers for `[\w&&\s]` and `[a-z&&[^aeiou]]`.
        ast::ClassSet::BinaryOp(op) => {
            let spelling = match op.kind {
                ast::ClassSetBinaryOpKind::Intersection => "&&",
                ast::ClassSetBinaryOpKind::Difference => "--",
                ast::ClassSetBinaryOpKind::SymmetricDifference => "~~",
            };
            Err(PatternError(format!(
                "pattern {pattern:?} uses the character-class set operator `{spelling}`, which \
                 JavaScript cannot compile and which Go, Python and Java each read differently \
                 (Python treats it as ordinary members); spell the resulting set explicitly"
            )))
        }
    }
}

fn collect_class_item<'a>(
    item: &'a ast::ClassSetItem,
    pattern: &str,
    out: &mut Vec<&'a ast::ClassSetItem>,
) -> Result<(), PatternError> {
    match item {
        ast::ClassSetItem::Union(union) => {
            for child in &union.items {
                collect_class_item(child, pattern, out)?;
            }
            Ok(())
        }
        ast::ClassSetItem::Bracketed(_) => Err(PatternError(format!(
            "pattern {pattern:?} nests a character class inside another (`[a[…]]`), which is a \
             JavaScript SyntaxError and which Go, Python and Java each read differently; \
             merge the members into a single class"
        ))),
        ast::ClassSetItem::Ascii(_) => Err(PatternError(format!(
            "pattern {pattern:?} uses a POSIX character class (`[:alpha:]`), which is a \
             JavaScript SyntaxError and which Python and Java read as a plain member list; \
             spell the range explicitly (e.g. `[A-Za-z]`)"
        ))),
        ast::ClassSetItem::Unicode(_) => Err(PatternError(format!(
            "pattern {pattern:?} uses a Unicode property escape (`\\p{{…}}`) inside a character \
             class, which Python's `re` cannot compile; spell the intended set explicitly"
        ))),
        ast::ClassSetItem::Literal(literal) => {
            check_literal(literal, pattern, true)?;
            out.push(item);
            Ok(())
        }
        ast::ClassSetItem::Range(range) => {
            check_literal(&range.start, pattern, true)?;
            check_literal(&range.end, pattern, true)?;
            out.push(item);
            Ok(())
        }
        ast::ClassSetItem::Perl(_) | ast::ClassSetItem::Empty(_) => {
            out.push(item);
            Ok(())
        }
    }
}

/// Decision **D7**: gate-reject the nested / ambiguous quantifier shapes.
/// "Regular" is a property of the *language*, not the *engine* — Go/RE2 and Rust
/// run `^(a+)+$` in linear time, while the three backtracking targets do not
/// (measured: 39 s for a 31-character input in CPython). A gate-accepted schema
/// must not be a remote DoS in three of four targets, so an **unbounded** loop
/// (`*`, `+`, `{n,}`) is rejected when its body is ambiguous:
///
/// - the body can match the empty string (`(a?)+`, `(a*)*`), or
/// - the body reduces to another *inexact* repetition (`(a+)+`, `(a{1,2})+`,
///   `(a+|b)*`) — "reduces to" meaning after stripping groups, and for a
///   concatenation, when every other element is nullable (`(a+b*)+`), or
/// - the body is an alternation with two textually identical branches
///   (`(a|a)*`), or
/// - the body is an alternation whose branches have the same positive fixed
///   width (`(a|b)*`, `(ab|cd)*`). Java recursively evaluates each iteration
///   of these forms and can overflow its stack on otherwise ordinary input.
///
/// An **exact-count** inner repetition is unambiguous and stays accepted, which
/// is what keeps the generator's own pinned `format` / `contentEncoding` regexes
/// (`(?:[A-Za-z0-9+/]{4})*`, the `hostname` / `email` / `uri` bodies, the `ipv6`
/// grammar) inside the gate.
fn check_repetition(repetition: &ast::Repetition, pattern: &str) -> Result<(), PatternError> {
    if matches!(repetition.ast.as_ref(), Ast::Repetition(_)) {
        return Err(PatternError(format!(
            "pattern {pattern:?} applies a repetition operator directly to another repetition, which Go, JavaScript and Python reject; group or rewrite the repeated expression"
        )));
    }
    if !is_unbounded(&repetition.op.kind) {
        return Ok(());
    }
    let body = strip_groups(&repetition.ast);
    let reason = if is_nullable(body) {
        "its body can match the empty string"
    } else if reduces_to_inexact_repetition(body) {
        "it applies a quantifier to another open-ended quantifier"
    } else if has_duplicate_alternation_branch(body, pattern) {
        "its body is an alternation with two identical branches"
    } else if has_equal_fixed_width_alternation(body) {
        "Java's backtracking engine recurses once per alternation iteration and can overflow its stack"
    } else {
        return Ok(());
    };
    Err(PatternError(format!(
        "pattern {pattern:?} contains an ambiguous unbounded quantifier ({reason}), which runs \
         in linear time under Go's RE2 but backtracks exponentially in JavaScript, Python and \
         Java — a schema that is safe in one target and a denial-of-service in the other three; \
         rewrite the repetition so each iteration consumes an unambiguous, non-empty chunk \
         (e.g. `^a+$` for `^(a+)+$`), or bound the inner quantifier to an exact count"
    )))
}

fn is_unbounded(kind: &ast::RepetitionKind) -> bool {
    match kind {
        ast::RepetitionKind::ZeroOrOne => false,
        ast::RepetitionKind::ZeroOrMore | ast::RepetitionKind::OneOrMore => true,
        ast::RepetitionKind::Range(range) => {
            matches!(range, ast::RepetitionRange::AtLeast(_))
        }
    }
}

/// True for a repetition whose iteration count is not fixed — the shape that
/// makes a nested loop ambiguous. `{n}` is exact and therefore safe.
fn is_inexact(kind: &ast::RepetitionKind) -> bool {
    !matches!(
        kind,
        ast::RepetitionKind::Range(ast::RepetitionRange::Exactly(_))
    )
}

fn strip_groups(ast: &Ast) -> &Ast {
    match ast {
        Ast::Group(group) => strip_groups(&group.ast),
        other => other,
    }
}

fn is_nullable(ast: &Ast) -> bool {
    match ast {
        Ast::Empty(_) | Ast::Flags(_) | Ast::Assertion(_) => true,
        Ast::Literal(_)
        | Ast::Dot(_)
        | Ast::ClassUnicode(_)
        | Ast::ClassPerl(_)
        | Ast::ClassBracketed(_) => false,
        Ast::Repetition(repetition) => {
            min_repetitions(&repetition.op.kind) == 0 || is_nullable(&repetition.ast)
        }
        Ast::Group(group) => is_nullable(&group.ast),
        Ast::Concat(concat) => concat.asts.iter().all(is_nullable),
        Ast::Alternation(alternation) => alternation.asts.iter().any(is_nullable),
    }
}

fn min_repetitions(kind: &ast::RepetitionKind) -> u32 {
    match kind {
        ast::RepetitionKind::ZeroOrOne | ast::RepetitionKind::ZeroOrMore => 0,
        ast::RepetitionKind::OneOrMore => 1,
        ast::RepetitionKind::Range(range) => match range {
            ast::RepetitionRange::Exactly(n)
            | ast::RepetitionRange::AtLeast(n)
            | ast::RepetitionRange::Bounded(n, _) => *n,
        },
    }
}

/// Whether `ast` behaves as a bare inexact repetition for the purposes of the
/// nested-quantifier rule: itself one, an alternation with such a branch, or a
/// concatenation in which one element is one and every other element is nullable
/// (so the concatenation can degenerate to it).
fn reduces_to_inexact_repetition(ast: &Ast) -> bool {
    match strip_groups(ast) {
        Ast::Repetition(repetition) => is_inexact(&repetition.op.kind),
        Ast::Alternation(alternation) => alternation.asts.iter().any(reduces_to_inexact_repetition),
        Ast::Concat(concat) => concat.asts.iter().enumerate().any(|(index, child)| {
            reduces_to_inexact_repetition(child)
                && concat
                    .asts
                    .iter()
                    .enumerate()
                    .all(|(other, sibling)| other == index || is_nullable(sibling))
        }),
        _ => false,
    }
}

/// `(a|a)*` and friends: two branches that accept the same input give a
/// backtracking engine 2^n paths through the loop. Compared on the source text,
/// which is exact and never over-rejects.
fn has_duplicate_alternation_branch(ast: &Ast, pattern: &str) -> bool {
    let Ast::Alternation(alternation) = strip_groups(ast) else {
        return false;
    };
    let mut seen = std::collections::HashSet::new();
    alternation
        .asts
        .iter()
        .any(|branch| !seen.insert(&pattern[branch.span().start.offset..branch.span().end.offset]))
}

/// Java recursively evaluates a repeated alternation even when its branches
/// are disjoint fixed-width strings (`(a|b)*`, `(ab|cd)*`), overflowing on only
/// a few thousand characters. The equal-width guard catches that reproduced
/// family without rejecting the generator's pinned URI grammar, whose repeated
/// alternatives consume different-sized chunks (for example one literal byte
/// or a three-byte percent escape).
fn has_equal_fixed_width_alternation(ast: &Ast) -> bool {
    let Ast::Alternation(alternation) = strip_groups(ast) else {
        return false;
    };
    let mut branches = alternation.asts.iter();
    let Some(first_width) = branches.next().and_then(fixed_width) else {
        return false;
    };
    first_width > 0 && branches.all(|branch| fixed_width(branch) == Some(first_width))
}

/// Exact code-point width when it is statically known. Assertions and empty
/// nodes consume zero; character-producing atoms consume one; only exact-count
/// repetitions and equal-width alternations preserve a fixed width.
fn fixed_width(ast: &Ast) -> Option<u32> {
    match ast {
        Ast::Empty(_) | Ast::Flags(_) | Ast::Assertion(_) => Some(0),
        Ast::Literal(_)
        | Ast::Dot(_)
        | Ast::ClassUnicode(_)
        | Ast::ClassPerl(_)
        | Ast::ClassBracketed(_) => Some(1),
        Ast::Group(group) => fixed_width(&group.ast),
        Ast::Concat(concat) => concat
            .asts
            .iter()
            .try_fold(0_u32, |width, child| width.checked_add(fixed_width(child)?)),
        Ast::Alternation(alternation) => {
            let mut branches = alternation.asts.iter();
            let width = fixed_width(branches.next()?)?;
            branches
                .all(|branch| fixed_width(branch) == Some(width))
                .then_some(width)
        }
        Ast::Repetition(repetition) => {
            let ast::RepetitionKind::Range(ast::RepetitionRange::Exactly(count)) =
                repetition.op.kind
            else {
                return None;
            };
            fixed_width(&repetition.ast)?.checked_mul(count)
        }
    }
}

fn inline_flag_error(pattern: &str) -> PatternError {
    PatternError(format!(
        "pattern {pattern:?} uses an inline flag group (e.g. `(?i)` / `(?flags:…)`) \
         which is not ECMA-262 syntax and cannot compile in JavaScript; remove it \
         (per-pattern case-insensitivity and friends are not portable)"
    ))
}

/// Apply non-overlapping `edits` to `src` (left to right by start offset).
fn apply_edits(src: &str, mut edits: Vec<Edit>) -> String {
    if edits.is_empty() {
        return src.to_string();
    }
    edits.sort_by_key(|edit| edit.start);
    let mut out = String::with_capacity(src.len() + 16);
    let mut cursor = 0;
    for edit in edits {
        out.push_str(&src[cursor..edit.start]);
        out.push_str(&edit.replacement);
        cursor = edit.end;
    }
    out.push_str(&src[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(pattern: &str) -> String {
        gate_and_normalize(pattern).expect("should accept")
    }

    #[test]
    fn keeps_plain_patterns_unchanged() {
        assert_eq!(norm("^[A-Z]{2,4}$"), "^[A-Z]{2,4}$");
        assert_eq!(norm(r"^\d{3}-\w{4}$"), r"^[0-9]{3}-[A-Za-z0-9_]{4}$");
        assert_eq!(norm(""), "");
    }

    #[test]
    fn normalizes_perl_digit_and_word_to_ascii() {
        assert_eq!(norm(r"\d"), "[0-9]");
        assert_eq!(norm(r"\D"), "[^0-9]");
        assert_eq!(norm(r"\w"), "[A-Za-z0-9_]");
        assert_eq!(norm(r"\W"), "[^A-Za-z0-9_]");
        assert_eq!(norm(r"[x\d]"), "[x0-9]");
        assert!(gate_and_normalize(r"[x\D]").is_err());
    }

    #[test]
    fn normalizes_perl_space() {
        assert_eq!(norm(r"\s"), r"[\t\n\x0B\f\r ]");
        assert_eq!(norm(r"\S"), r"[^\t\n\x0B\f\r ]");
        assert_eq!(norm(r"^\s+$"), r"^[\t\n\x0B\f\r ]+$");
        assert_eq!(norm(r"[\s.]"), r"[\t\n\x0B\f\r .]");
        assert_eq!(norm(r"[^\s]"), r"[^\t\n\x0B\f\r ]");
        assert_eq!(norm(r"[\S]"), r"[^\t\n\x0B\f\r ]");
        assert_eq!(norm(r"[^\S]"), r"[\t\n\x0B\f\r ]");
    }

    /// `08#2`: `.` is a different set in each engine — it is spliced out.
    #[test]
    fn normalizes_dot_to_an_explicit_class() {
        assert_eq!(norm("a.b"), r"a[^\n]b");
        assert_eq!(norm("^a.$"), r"^a[^\n]$");
        assert_eq!(norm("a.*b"), r"a[^\n]*b");
        // An escaped dot and a dot inside a class are literals — untouched.
        assert_eq!(norm(r"a\.b"), r"a\.b");
        assert_eq!(norm("[.]"), "[.]");
    }

    #[test]
    fn rewrites_end_anchor_per_target() {
        assert_eq!(rewrite_end_anchor("^[a-z]+$", r"\Z"), r"^[a-z]+\Z");
        assert_eq!(rewrite_end_anchor("^[a-z]+$", r"\z"), r"^[a-z]+\z");
        // Escaped `\$` and `^` are untouched.
        assert_eq!(rewrite_end_anchor(r"^a\$", r"\z"), r"^a\$");
    }

    #[test]
    fn rejects_backtracking() {
        assert!(gate_and_normalize("(?=.*[A-Z]).+").is_err());
        assert!(gate_and_normalize("(?<=x)y").is_err());
        assert!(gate_and_normalize(r"(a)\1").is_err());
    }

    #[test]
    fn rejects_inline_flags() {
        assert!(gate_and_normalize("(?i)^cat$").is_err());
        assert!(gate_and_normalize("(?i:ab)").is_err());
        // A bare non-capturing group is fine.
        assert!(gate_and_normalize("(?:ab)+").is_ok());
    }

    #[test]
    fn rejects_open_complement_class() {
        assert!(gate_and_normalize(r"[\S.]").is_err());
        assert!(gate_and_normalize(r"[\S\d]").is_err());
    }

    /// `08#1`: escapes Rust accepts that at least one target engine does not.
    #[test]
    fn rejects_non_portable_escapes() {
        // The flagship case: an ordinary phone pattern whose `\-` is a JS
        // `SyntaxError` under the mandatory `u` flag.
        assert!(gate_and_normalize(r"^\d{3}\-\d{4}$").is_err());
        assert!(gate_and_normalize(r"\_").is_err());
        assert!(gate_and_normalize("a\\\"b").is_err());
        assert!(gate_and_normalize(r"a\ b").is_err());
        assert!(gate_and_normalize(r"a\ab").is_err()); // \a — JS
        assert!(gate_and_normalize(r"a\vb").is_err()); // \v — Java class
        assert!(gate_and_normalize(r"\0").is_err()); // octal — Java
        assert!(gate_and_normalize("a\\u0041b").is_err()); // \uFFFF — Go
        assert!(gate_and_normalize(r"\x{1F600}").is_err()); // \x{…} — JS, Python
        assert!(gate_and_normalize(r"\p{L}+").is_err()); // \p{…} — Python
        assert!(gate_and_normalize(r"\pL").is_err()); // \pL — JS, Python
        // The portable escapes stay accepted, including `\-` *inside* a class.
        for portable in [
            r"a\.b", r"a\*b", r"a\+b", r"a\?b", r"a\(b", r"a\)b", r"a\[b", r"a\]b", r"a\{b",
            r"a\}b", r"a\|b", r"a\^b", r"a\$b", r"a\\b", r"a\/b", r"a\tb", r"a\nb", r"a\rb",
            r"a\fb", r"a\x41b", r"[a\-z]", r"[\^a]", r"[\]]",
        ] {
            assert!(
                gate_and_normalize(portable).is_ok(),
                "{portable} should stay accepted"
            );
        }
    }

    /// `08#1`: a lone `{`, `}` or `]` is a JS (and, for `{`, Java) syntax error.
    #[test]
    fn rejects_lone_brackets() {
        assert!(gate_and_normalize("a}b").is_err());
        assert!(gate_and_normalize("a]b").is_err());
        assert!(gate_and_normalize("a{b").is_err());
        // Inside a class they are ordinary members in every engine.
        assert!(gate_and_normalize("[{}]").is_ok());
        // A real quantifier is not a lone bracket.
        assert!(gate_and_normalize("^a{2,4}$").is_ok());
        for leading_close in [r"[]]", r"[]a]", r"[^]]", r"[]-a]"] {
            assert!(
                gate_and_normalize(leading_close).is_err(),
                "{leading_close} must reject for JavaScript u-mode"
            );
        }
    }

    /// `08#1`: named groups have no spelling that Python and Java both accept.
    #[test]
    fn rejects_named_capture_groups() {
        assert!(gate_and_normalize("(?P<n>a)b").is_err());
        assert!(gate_and_normalize("(?<name>a)b").is_err());
        // Plain and non-capturing groups are fine.
        assert!(gate_and_normalize("(a)b").is_ok());
        assert!(gate_and_normalize("(?:a)b").is_ok());
    }

    /// `08#1`: `\A` / `\z` are JS syntax errors (`\z` is a Python error too).
    #[test]
    fn rejects_non_portable_anchors() {
        assert!(gate_and_normalize(r"\Aabc").is_err());
        assert!(gate_and_normalize(r"abc\z").is_err());
        for boundary in [r"\bfoo\b", r"foo\B"] {
            let error = gate_and_normalize(boundary).unwrap_err().0;
            assert!(error.contains("non-ASCII"), "{boundary}: {error}");
        }
    }

    /// `08#1` / `08#3`: POSIX classes, nested classes and class set operations
    /// compile in two or more targets and match *differently* there.
    #[test]
    fn rejects_non_portable_class_constructs() {
        assert!(gate_and_normalize("[[:alpha:]]+").is_err());
        assert!(gate_and_normalize("[a-z&&[^aeiou]]").is_err());
        assert!(gate_and_normalize(r"[\w&&\s]").is_err());
        assert!(gate_and_normalize("[a-z--[aeiou]]").is_err());
        assert!(gate_and_normalize(r"[a[\s]]").is_err());
        // `[[\S]]` used to slip past the `\S`-in-class reject via the nested
        // class; it is now rejected as a nested class.
        assert!(gate_and_normalize(r"[[\S]]").is_err());
    }

    /// `08#9` / decision D7: nested and ambiguous unbounded quantifiers.
    #[test]
    fn rejects_ambiguous_unbounded_quantifiers() {
        for hostile in [
            "^(a+)+$",
            "^(a*)*$",
            "^(a?)+$",
            "^([a-z]+)+$",
            "^(a{1,2})+$",
            "^(a+b*)+$",
            "^(a+|b)*$",
            "^(a|a)*$",
            "^(ab|cd)*$",
            r"^(\w+\s*)+$",
        ] {
            assert!(
                gate_and_normalize(hostile).is_err(),
                "{hostile} should gate-reject (ReDoS)"
            );
        }
        for stacked in ["a{2}*", "a{2}{3}", "a*{3}", "a+{2}", "a?{2}"] {
            assert!(
                gate_and_normalize(stacked).is_err(),
                "{stacked} must reject as a stacked repetition"
            );
        }
        assert!(gate_and_normalize("^(a|b)*$").is_err());
        // Unambiguous loops stay accepted.
        for safe in [
            "^a+$",
            "^.*$",
            "^(ab)+$",
            "^(a{2})+$",
            r"^(\d{3}-)+$",
            "^(a+b)+$",
            r"^(\.a+)+$",
            "^(?:[A-Za-z0-9+/]{4})*$",
        ] {
            assert!(
                gate_and_normalize(safe).is_ok(),
                "{safe} should stay accepted: {:?}",
                gate_and_normalize(safe).err()
            );
        }
    }

    /// The normalized form must itself be inside the accepted subset — the
    /// spliced `[\t\n\x0B\f\r ]` and `[^\n]` classes are re-parsed by
    /// [`rewrite_end_anchor`] at emit time and by the loader's literal check.
    #[test]
    fn normalization_is_idempotent() {
        for pattern in [
            r"^\s+$",
            r"[\s.]",
            r"[\S]",
            "a.b",
            r"^\d{3}-\w{4}$",
            r"^[a-z]+$",
            "",
        ] {
            let once = norm(pattern);
            let twice = gate_and_normalize(&once)
                .unwrap_or_else(|error| panic!("re-gating {once:?} failed: {error:?}"));
            assert_eq!(once, twice, "normalizing {pattern:?} is not idempotent");
        }
    }

    /// Regression: drive the shared conformance corpus through the gate. Every
    /// `expect_gate_reject` pair must reject; every other pair must accept.
    /// The corpus (not a hard-coded id in this test) is the single source of
    /// truth for which pairs reject.
    #[test]
    fn conformance_corpus_gate_agrees() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/pattern_conformance/corpus.json"
        ))
        .expect("corpus parses");
        for pair in corpus["pairs"].as_array().expect("pairs array") {
            let id = pair["id"].as_str().unwrap_or("<no id>");
            let pattern = pair["pattern"].as_str().expect("pattern string");
            let expect_reject = pair["expect_gate_reject"].as_bool().unwrap_or(false);
            let result = gate_and_normalize(pattern);
            if expect_reject {
                assert!(
                    result.is_err(),
                    "pair `{id}` ({pattern:?}) should gate-reject"
                );
            } else {
                assert!(
                    result.is_ok(),
                    "pair `{id}` ({pattern:?}) should gate-accept: {:?}",
                    result.err()
                );
            }
        }
    }

    /// Every gate-accepted corpus pair carries the expected match verdict that
    /// the per-language corpus runners assert against, and every gate-rejected
    /// pair carries none (there is nothing to match).
    #[test]
    fn conformance_corpus_declares_expected_matches() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../specs/json-schema/corpora/pattern_conformance/corpus.json"
        ))
        .expect("corpus parses");
        for pair in corpus["pairs"].as_array().expect("pairs array") {
            let id = pair["id"].as_str().unwrap_or("<no id>");
            let expect_reject = pair["expect_gate_reject"].as_bool().unwrap_or(false);
            if expect_reject {
                assert!(
                    pair.get("expect_match").is_none(),
                    "gate-rejected pair `{id}` must not declare `expect_match`"
                );
            } else {
                assert!(
                    pair["expect_match"].is_boolean(),
                    "pair `{id}` must declare a boolean `expect_match`"
                );
            }
        }
    }
}
