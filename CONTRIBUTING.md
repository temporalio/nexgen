# Contributing

Run `cargo validate` before pushing; it validates Rust and all five language
sample projects. Follow the generated sample guidance in [AGENTS.md](AGENTS.md).

Public-facing changes need a [changelog fragment](changelog/README.md).

## Preparing a release

From a clean worktree, run:

```sh
python3 scripts/prepare_release.py 0.3.0
```

The script creates or resumes `chore/release-0.3.0`, updates the crate and
lockfile versions, verifies the lockfile, assembles a dated release section,
and consumes fragments. It regenerates samples, commits the release changes,
pushes, and opens or reuses the release PR. Empty releases are allowed.
`--date YYYY-MM-DD` overrides the release date.
Resuming a prepared release preserves its changelog and any later fragments.
Release PRs use the `skip-changelog` label because they consume existing entries.

After merging, run the **Create Release** workflow from main. It uses the
shared tool to extract the completed changelog section for the draft GitHub
release, attaches binaries, and publishes the crate.

GitHub Actions checks out a pinned SDK Rust revision to build only the shared
changelog CLI for fragment validation and release-note extraction. Local release
preparation needs no SDK Rust checkout; Nexgen has no Core submodule.
Keep the tool pins in the changelog and Create Release workflows consistent.
