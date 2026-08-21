#!/usr/bin/env bash

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

run() {
  printf '\n==> %s\n' "$*"
  "$@"
}

run_in() {
  local dir="$1"
  shift
  printf '\n==> (cd %s && %s)\n' "$dir" "$*"
  (cd "$dir" && "$@")
}

require_gradle_java() {
  if ! command -v java >/dev/null 2>&1; then
    echo "Java 17 or later is required to launch the sample Gradle builds." >&2
    echo "CI and the Gradle toolchains use Java 21; set JAVA_HOME and PATH accordingly." >&2
    exit 1
  fi

  local java_version java_major
  java_version="$(java -version 2>&1 | sed -n '1s/.*version "\([^"]*\)".*/\1/p')"
  java_major="${java_version%%.*}"
  if [[ ! "$java_major" =~ ^[0-9]+$ ]] || ((java_major < 17)); then
    echo "Java 17 or later is required to launch the sample Gradle builds; found ${java_version:-unknown}." >&2
    echo "CI and the Gradle toolchains use Java 21; set JAVA_HOME and PATH accordingly." >&2
    exit 1
  fi
}

require_gradle_java

for tier in samples advanced/samples; do
  run_in "$tier/python" uv sync --locked
  run_in "$tier/typescript" npm ci
done

run cargo fmt --check
# The `advanced` feature exposes the WIT/proto CLI surface the integration tests
# exercise; enable it so the full suite runs.
run cargo test --features advanced

for tier in samples advanced/samples; do
  run_in "$tier/python" uv run ruff check .
  run_in "$tier/python" uv run ruff format --check .
  run_in "$tier/python" uv run basedpyright
  run_in "$tier/python" uv run pytest
  run_in "$tier/typescript" npm exec -- prettier --check .
  run_in "$tier/typescript" npm run typecheck
  run_in "$tier/typescript" npm run test
  run_in "$tier/go" bash -c 'unformatted="$(gofmt -l .)"; if [ -n "$unformatted" ]; then echo "gofmt required for:" >&2; echo "$unformatted" >&2; exit 1; fi'
  run_in "$tier/go" go test ./...
  run_in "$tier/java" ./gradlew build --no-daemon
  run_in "$tier/dotnet" dotnet test tests/ --nologo
done
