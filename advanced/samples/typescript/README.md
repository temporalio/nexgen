# TypeScript WIT Samples

Advanced TypeScript sample suite for outputs generated from authored WIT inputs,
plus the snapshot-only native-api (Nexus service/client) output of the
JSON-Schema samples.

- Authored WIT inputs live in [`advanced/samples/inputs/*.wit`](../inputs)
- Checked-in generated WIT outputs live in
  `advanced/samples/typescript/wit/<example>/`
- Authored Temporal converters live in the shared `support/` directory;
  generated modules import them rather than copying them into each output
- Snapshot-only JSON-Schema native-api output lives in
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

## Temporary notification protobuf declarations

The notification-service example uses the SDK namespace
`temporal.api.notificationservice.v1`. Until the TypeScript SDK exports these
models,
[`support/notification-service-proto.d.ts`](support/notification-service-proto.d.ts)
supplies sample-only interface declarations matching the checked-in descriptor.
Remove this file and update the SDK dependency **before landing this change**.
The generator has no fallback types or notification-specific logic.

The notification tests exercise conversion of plain wire objects, including
generic payloads, failure details, and oneof validation. They do not yet
exercise notification protobuf binary encoding or SDK message constructors.

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
