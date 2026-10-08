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
