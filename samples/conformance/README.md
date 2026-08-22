# JSON Schema conformance manifest

`json-schema.json` is the language-neutral index for cross-language runtime
conformance cases. It records what loads, accepted wire values, parse and
serialize failures, their expected violation paths, and the narrow set of
presence/nullability collapses permitted by the JSON Schema specification.

Each case has a stable `id` and declares exactly one consumer in Go,
TypeScript, Python, and Java. A consumer declaration names a repository-relative
test source and a test-name anchor in that source. The Rust coverage test rejects
missing targets, stale source paths or anchors, missing fixtures, malformed raw
JSON, duplicate identifiers and paths, and undeclared manifest fields.

Accepted values use either `fixture` for a repository fixture or `wire_json` for
an inline JSON value. Parse failures use `wire_json`; serialize failures use a
language-neutral `native_value` description because non-finite and otherwise
invalid native values are not always representable as JSON. Failure entries
compare paths only; exception wording remains target-idiomatic.

For a load-time rejection, set `expected_load.result` to `rejected` and include
the targeted `diagnostic`. Accepted cases must include at least one accepted wire
value. Every manifest case must retain all fields, using empty arrays when a case
does not exercise that dimension.
