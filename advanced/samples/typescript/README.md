# TypeScript WIT Samples

Advanced TypeScript sample suite for outputs generated from authored WIT inputs,
plus the snapshot-only Workflow API (Nexus service/client) output of the
JSON-Schema samples.

- Authored WIT inputs live in [`advanced/samples/inputs/*.wit`](../inputs)
- Checked-in generated WIT outputs live in
  `advanced/samples/typescript/wit/<example>/`
- Authored Temporal converters live in the shared `support/` directory;
  generated modules import them rather than copying them into each output
- Snapshot-only JSON-Schema Workflow API output lives in
  `advanced/samples/typescript/json_schema/api/<example>/`
- Vitest files live in `advanced/samples/typescript/tests/` (WIT round-trip and
  real-workflow/Nexus tests); proto wire fixtures live in
  [`advanced/samples/wire/proto`](../wire/proto)

Top-level rebuild command (both WIT and JSON Schema outputs):

```bash
cargo build-examples --lang typescript
```

Current workflow:

```bash
cargo build-examples --lang typescript
cd advanced/samples/typescript
npm install
npm run typecheck
npm run test
```

To rebuild one example only:

```bash
cargo build-examples --format wit --lang typescript workflow-service
```

The definitions-mode JSON-Schema samples (plain data models) live under
[`samples/typescript`](../../samples/typescript).

## Protobuf payloads and sample declarations

[`tests/proto-integer-payload.test.ts`](tests/proto-integer-payload.test.ts)
uses the SDK's `Int64Value` and `UInt64Value` messages to round-trip signed and
unsigned values outside JavaScript's safe integer range through JSON envelopes
and binary protobuf serialization. Generated converters return plain properties;
the test constructs messages at the transport boundary, as System Nexus does.
The installed SDK does not yet expose that transport, so this is boundary
coverage rather than a live System Nexus operation.

The notification and Stream protos are newer than the SDK. Sample-only interface
declarations live in
[`support/notification-service-proto.d.ts`](support/notification-service-proto.d.ts)
and [`proto/streams.d.ts`](proto/streams.d.ts). Their tests exercise plain
transfer objects, without runtime constructors or descriptor loading. These
declarations are not a generator fallback; remove them when the SDK exports the
same definitions.

The `proto-generic` and `proto-oneof` samples use the SDK's existing protobuf
models without a shim. Their tests include binary encode/decode round-trips,
nested generic payloads, required and optional oneofs, and scalar member
presence (including an explicitly selected empty string).

## Using a support module

In your own project, keep hand-written helpers
**outside the directory passed to `--output`** and export the functions
generated code calls. Supply either an installed package specifier or a module
specifier **relative to that output directory**, not relative to the shell's
working directory or a nested generated file. Nexgen adjusts relative paths for
nested `operations/*.ts` and imports the module as `support`.

For example, this repository exports `retryPolicyToProto` and
`durationFromProto` from
[`support/temporal_model_converters.ts`](support/temporal_model_converters.ts),
outside the generated `wit/<example>/` directories. The module is
`../../support/temporal_model_converters` relative to its
`wit/proto-backed-showcase` output directory:

```sh
cargo run --features advanced -- typescript advanced/samples/inputs/proto-backed-showcase.wit \
  advanced/samples/inputs/deps \
  --descriptors advanced/samples/descriptors/temporal_api.bin \
  --support-package ../../support/temporal_model_converters \
  --targets workflow --output advanced/samples/typescript/wit/proto-backed-showcase
```

Include both your authored module and generated output in your TypeScript
project so the import resolves. The sample's [`tsconfig.json`](tsconfig.json)
does so; from this directory, run `npm run typecheck` and `npm run test`. For
derived helper names and sourced expression imports, see
[the WIT guide](../../../GUIDE.md#typescript).
