# TypeScript JSON-Schema Samples

TypeScript sample suite for outputs generated from JSON-Schema inputs in
[`samples/schemas`](../schemas) (definitions mode — plain data models, no Nexus
service scaffolding).

- Authored JSON-Schema inputs live in `samples/schemas/`
- Checked-in generated definitions live in `samples/typescript/<example>/`
  (`chat`, `kb`, `showcase`, `temporal`, plus the `temporal-date` /
  `temporal-temporal` date-time representation variants)
- Vitest round-trip tests live in `samples/typescript/tests/`, driving the
  generated transfer type converters through the Temporal data converter against
  the canonical wire fixtures in
  [`samples/wire/json_schema`](../wire/json_schema)

Top-level rebuild command:

```bash
cargo build-examples --format json-schema --lang typescript
```

Current workflow:

```bash
cargo build-examples --format json-schema --lang typescript
cd samples/typescript
npm install
npm run typecheck
npm run test
```

The Workflow API (Nexus service/client) variant of these same schemas is
snapshot-only and lives under
[`advanced/samples/typescript/json_schema/api`](../../advanced/samples/typescript).
