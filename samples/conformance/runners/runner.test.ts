/**
 * Generic cross-language conformance runner for generated **TypeScript**
 * packages.
 *
 * Driven by `tests/json_schema_conformance_manifest.rs` through a plan file
 * (protocol in `tests/toolchain/mod.rs`). It runs under vitest because vitest's
 * transform pipeline is what resolves the generator's extension-less relative
 * imports; the file asserts nothing itself — the Rust driver compares the
 * verdicts it writes with the other three targets'.
 *
 * `registry.ts` is written by the driver next to this file: it maps a case id to
 * a *lazy* static-analysable `import()` so one uncompilable case reports as that
 * case's failure instead of blinding the whole target.
 */
import { readFileSync, writeFileSync } from "node:fs";
import {
  ApplicationFailure,
  defaultPayloadConverter,
  fromPayloadWithTypeInfo,
  toPayloadWithTypeInfo,
} from "@temporalio/common";
import type { TransferTypeConverter } from "nexus-rpc";
import { test } from "vitest";
// eslint-disable-next-line import/no-unresolved -- written by the Rust driver
import { REGISTRY } from "./registry";

type Probe = {
  id: string;
  kind: "parse" | "round_trip" | "serialize";
  wire: string;
  mutations?: Mutation[];
};
type Mutation = {
  path: string;
  set_integer?: string;
  set_number?: string;
  set_string?: string;
  set_null?: boolean;
  duplicate_element?: number;
  remove_array_element?: number;
  put_map_entry?: { key: string; value: unknown };
  remove_map_entry?: string;
  set_absent?: boolean;
  set_bytes?: number[];
  set_duration?: { seconds: number; nanoseconds: number };
};
type Case = { id: string; dir: string; model: string; probes: Probe[] };
type Verdict = Record<string, unknown>;

/** Deterministic, key-sorted JSON so the four targets are comparable. */
function canonicalStringify(value: unknown): string {
  if (value === null || typeof value !== "object") {
    if (typeof value === "number" && !Number.isFinite(value)) {
      throw new Error(`non-finite number in output: ${value}`);
    }
    return JSON.stringify(value) ?? "null";
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalStringify).join(",")}]`;
  }
  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, v]) => v !== undefined)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return `{${entries.map(([k, v]) => `${JSON.stringify(k)}:${canonicalStringify(v)}`).join(",")}}`;
}

function violationsOf(
  error: unknown,
): { path: string; reason: string }[] | null {
  if (
    !(error instanceof ApplicationFailure) ||
    error.type !== "PayloadValidationError"
  ) {
    return null;
  }
  // Locally-created failures retain the original array as their first detail;
  // this cast is cheap and performs no serialization.
  const found = error.details?.[0] as unknown;
  if (!Array.isArray(found)) {
    return null;
  }
  return found.map((v: { path: string; reason: string }) => ({
    path: v.path,
    reason: v.reason,
  }));
}

function converterOf(module: Record<string, unknown>, model: string): any {
  const exact = `${model.charAt(0).toLowerCase()}${model.slice(1)}TransferTypeConverter`;
  if (module[exact]) {
    return module[exact];
  }
  const candidates = Object.keys(module).filter((k) =>
    k.endsWith("TransferTypeConverter"),
  );
  const relaxed = candidates.find(
    (k) => k.toLowerCase() === exact.toLowerCase(),
  );
  if (relaxed) {
    return module[relaxed];
  }
  throw new Error(
    `no converter for ${model}; exports: ${candidates.join(", ")}`,
  );
}

function decodeWithTypeInfo(
  converter: TransferTypeConverter<unknown>,
  wire: string,
): unknown {
  const payload = defaultPayloadConverter.toPayload(JSON.parse(wire));
  return fromPayloadWithTypeInfo(defaultPayloadConverter, payload, undefined, {
    transferTypeConverter: converter,
  });
}

function encodeWithTypeInfo(
  converter: TransferTypeConverter<unknown>,
  model: unknown,
): unknown {
  const payload = toPayloadWithTypeInfo(
    defaultPayloadConverter,
    model,
    undefined,
    {
      transferTypeConverter: converter,
    },
  );
  return defaultPayloadConverter.fromPayload(payload);
}

type Step = { kind: "field"; name: string } | { kind: "index"; at: number };

/** `a.b[0][1]` -> field a, field b, index 0, index 1. */
function stepsOf(path: string): Step[] {
  const out: Step[] = [];
  for (const segment of path.split(".")) {
    const match = /^([A-Za-z0-9]+)((?:\[\d+\])*)$/.exec(segment);
    if (!match) {
      throw new Error(`unparsable mutation path segment ${segment}`);
    }
    out.push({ kind: "field", name: match[1]! });
    for (const index of match[2]!.matchAll(/\[(\d+)\]/g)) {
      out.push({ kind: "index", at: Number(index[1]) });
    }
  }
  return out;
}

function read(owner: any, step: Step): any {
  if (step.kind === "index") {
    return owner[step.at];
  }
  if (owner === undefined || owner === null || !(step.name in owner)) {
    throw new Error(`no member ${step.name}`);
  }
  return owner[step.name];
}

function write(owner: any, step: Step, value: unknown): void {
  if (step.kind === "index") {
    owner[step.at] = value;
    return;
  }
  if (owner === undefined || owner === null || !(step.name in owner)) {
    throw new Error(`no member ${step.name}`);
  }
  owner[step.name] = value;
}

function numberOf(spec: string): number {
  if (spec === "nan") return Number.NaN;
  if (spec === "inf") return Number.POSITIVE_INFINITY;
  if (spec === "-inf") return Number.NEGATIVE_INFINITY;
  return Number(spec);
}

function typedMap(value: unknown): Record<string, unknown> {
  const candidate = value as Record<string, unknown>;
  if (candidate.additionalProperties !== undefined) {
    return candidate.additionalProperties as Record<string, unknown>;
  }
  return candidate;
}

function applyMutation(model: any, mutation: Mutation): void {
  const steps = stepsOf(mutation.path);
  let owner = model;
  for (const step of steps.slice(0, -1)) {
    owner = read(owner, step);
  }
  const last = steps[steps.length - 1]!;
  if (mutation.duplicate_element !== undefined) {
    const sequence = read(owner, last) as unknown[];
    sequence.push(sequence[mutation.duplicate_element]);
    return;
  }
  if (mutation.remove_array_element !== undefined) {
    const sequence = read(owner, last) as unknown[];
    sequence.splice(mutation.remove_array_element, 1);
    return;
  }
  if (mutation.put_map_entry !== undefined) {
    const map = typedMap(read(owner, last));
    map[mutation.put_map_entry.key] = mutation.put_map_entry.value;
    return;
  }
  if (mutation.remove_map_entry !== undefined) {
    const map = typedMap(read(owner, last));
    delete map[mutation.remove_map_entry];
    return;
  }
  let value: unknown;
  if (mutation.set_integer !== undefined) {
    value = Number(mutation.set_integer);
  } else if (mutation.set_number !== undefined) {
    value = numberOf(mutation.set_number);
  } else if (mutation.set_string !== undefined) {
    value = mutation.set_string;
  } else if (mutation.set_null !== undefined) {
    value = null;
  } else if (mutation.set_absent !== undefined) {
    value = undefined;
  } else if (mutation.set_bytes !== undefined) {
    value = Uint8Array.from(mutation.set_bytes);
  } else if (mutation.set_duration !== undefined) {
    const { seconds, nanoseconds } = mutation.set_duration;
    const temporal = (
      globalThis as typeof globalThis & {
        Temporal?: {
          Duration: { from(value: Record<string, number>): unknown };
        };
      }
    ).Temporal;
    const current = read(owner, last);
    if (typeof current === "string") {
      if (nanoseconds === 0) {
        const hours = Math.floor(seconds / 3600);
        const minutes = Math.floor((seconds % 3600) / 60);
        const remainder = seconds % 60;
        value = `PT${hours === 0 ? "" : `${hours}H`}${minutes === 0 ? "" : `${minutes}M`}${remainder === 0 ? "" : `${remainder}S`}`;
        if (value === "PT") value = "PT0S";
      } else {
        const fraction = `.${String(nanoseconds).padStart(9, "0").replace(/0+$/, "")}`;
        value = `PT${seconds}${fraction}S`;
      }
    } else if (temporal !== undefined) {
      value = temporal.Duration.from({ seconds, nanoseconds });
    } else {
      throw new Error("Temporal.Duration is unavailable");
    }
  } else {
    throw new Error(`unknown mutation ${JSON.stringify(mutation)}`);
  }
  write(owner, last, value);
}

function runProbe(
  converter: TransferTypeConverter<unknown>,
  probe: Probe,
): Verdict {
  let model: unknown;
  try {
    model = decodeWithTypeInfo(converter, probe.wire);
  } catch (error) {
    const violations = violationsOf(error);
    return violations === null
      ? {
          outcome: "error",
          message: `${(error as Error).name}: ${(error as Error).message}`,
        }
      : { outcome: "parse_rejected", violations };
  }
  if (probe.kind === "parse") {
    return { outcome: "accepted" };
  }
  try {
    for (const mutation of probe.mutations ?? []) {
      applyMutation(model, mutation);
    }
  } catch (error) {
    return {
      outcome: "error",
      message: `mutation failed: ${(error as Error).message}`,
    };
  }
  let transfer: unknown;
  try {
    transfer = encodeWithTypeInfo(converter, model);
  } catch (error) {
    const violations = violationsOf(error);
    return violations === null
      ? {
          outcome: "error",
          message: `${(error as Error).name}: ${(error as Error).message}`,
        }
      : { outcome: "serialize_rejected", violations };
  }
  try {
    return { outcome: "accepted", wire: canonicalStringify(transfer) };
  } catch (error) {
    return { outcome: "accepted", wire: null, note: (error as Error).message };
  }
}

test("json-schema conformance probes", async () => {
  const planPath = process.env.NEXGEN_CONFORMANCE_PLAN!;
  const resultPath = process.env.NEXGEN_CONFORMANCE_RESULT!;
  const plan = JSON.parse(readFileSync(planPath, "utf8")) as { cases: Case[] };
  const results: Record<string, Record<string, Verdict>> = {};
  for (const testCase of plan.cases) {
    const probes: Record<string, Verdict> = {};
    results[testCase.id] = probes;
    let converter: TransferTypeConverter<unknown>;
    try {
      const entry = REGISTRY[testCase.id];
      if (!entry) {
        throw new Error(
          `case ${testCase.id} missing from the generated registry`,
        );
      }
      converter = converterOf(
        (await entry.load()) as Record<string, unknown>,
        testCase.model,
      );
    } catch (error) {
      const message = `import failed: ${(error as Error).message}`;
      for (const probe of testCase.probes) {
        probes[probe.id] = { outcome: "error", message };
      }
      continue;
    }
    for (const probe of testCase.probes) {
      probes[probe.id] = runProbe(converter, probe);
    }
  }
  writeFileSync(resultPath, JSON.stringify(results, null, 1));
});
