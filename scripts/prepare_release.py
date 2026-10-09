"""Prepare checked-in files for a nexgen release."""

from __future__ import annotations

import argparse
import datetime
import json
import pathlib
import re
import subprocess
from collections.abc import Sequence

VERSION_RE = re.compile(r"[0-9]+(?:\.[0-9]+)+(?:[a-zA-Z0-9_.+-]+)?")
CHANGELOG_CATEGORIES = (
    "added",
    "stabilized",
    "changed",
    "deprecated",
    "breaking-changes",
    "fixed",
    "security",
)
_RELEASE_FILES = (
    "CHANGELOG.md",
    "Cargo.toml",
    "Cargo.lock",
)
_RELEASE_FILE_SET = frozenset(_RELEASE_FILES)
_GENERATED_SAMPLE_DIRECTORIES = (
    "advanced/samples/",
    "samples/",
)


def validate_version(version: str) -> str:
    if not VERSION_RE.fullmatch(version):
        raise ValueError(
            f"Invalid version {version!r}; expected a version like '0.3.0'"
        )
    return version


def parse_date(date: str) -> datetime.date:
    try:
        return datetime.date.fromisoformat(date)
    except ValueError as err:
        raise ValueError(f"Invalid release date {date!r}; expected YYYY-MM-DD") from err


def prepare_changelog(
    repo_root: pathlib.Path,
    *,
    version: str,
    release_date: datetime.date,
) -> None:
    # Resume without consuming entries added after this release was prepared.
    changelog_path = repo_root / "CHANGELOG.md"
    text = changelog_path.read_text(encoding="utf-8")
    if re.search(rf"(?m)^## \[{re.escape(version)}\](?:\s+-[^\n]*)?$", text):
        return
    sections = []
    fragments = []
    for category in CHANGELOG_CATEGORIES:
        entries = []
        for path in sorted((repo_root / "changelog" / category).glob("*.md")):
            lines = [
                line
                for line in path.read_text(encoding="utf-8").splitlines()
                if line.strip()
            ]
            if not lines:
                raise RuntimeError(f"Empty changelog fragment: {path}")
            entries.extend(f"- {line}\n" for line in lines)
            fragments.append(path)
        if entries:
            heading = category.replace("-", " ").title()
            if category == "breaking-changes":
                heading = f":boom: {heading}"
            sections.append(f"### {heading}\n\n{''.join(entries)}\n")
    first_release = text.find("## [")
    preamble = text if first_release == -1 else text[:first_release]
    history = "" if first_release == -1 else text[first_release:]
    changelog_path.write_text(
        f"{preamble.rstrip()}\n\n## [{version}] - {release_date.isoformat()}\n\n"
        f"{''.join(sections)}{history}",
        encoding="utf-8",
    )
    for fragment in fragments:
        fragment.unlink()


def replace_manifest_version(text: str, version: str) -> str:
    return _replace_once(
        r'(?m)^version = "[^"]+"[^\S\r\n]*$',
        f'version = "{validate_version(version)}"',
        text,
        description="Cargo.toml package version",
    )


def replace_lock_package_version(text: str, version: str) -> str:
    return _replace_once(
        r'(?ms)(\[\[package\]\]\nname = "nexgen"\nversion = ")[^"]+(")',
        rf"\g<1>{validate_version(version)}\2",
        text,
        description="Cargo.lock nexgen package version",
    )


def create_release_branch(repo_root: pathlib.Path, version: str) -> None:
    subprocess.run(["git", "fetch", "origin", "main"], cwd=repo_root, check=True)
    branch = f"chore/release-{version}"
    current_branch = subprocess.run(
        ["git", "branch", "--show-current"],
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if current_branch == branch:
        return

    branch_exists = (
        subprocess.run(
            ["git", "show-ref", "--verify", "--quiet", f"refs/heads/{branch}"],
            cwd=repo_root,
        ).returncode
        == 0
    )
    if branch_exists:
        subprocess.run(["git", "switch", branch], cwd=repo_root, check=True)
    else:
        subprocess.run(
            ["git", "switch", "--create", branch, "origin/main"],
            cwd=repo_root,
            check=True,
        )


def changed_files(repo_root: pathlib.Path) -> set[str]:
    result = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    )
    return {line[3:] for line in result.stdout.splitlines()}


def ensure_clean_worktree(repo_root: pathlib.Path) -> None:
    changes = changed_files(repo_root)
    if changes:
        raise RuntimeError(
            "Release preparation requires a clean worktree; found changes in "
            + ", ".join(sorted(changes))
        )


def ensure_only_release_changes(
    repo_root: pathlib.Path,
    fragments: set[str],
) -> None:
    unexpected_files = {
        path
        for path in changed_files(repo_root)
        if path not in _RELEASE_FILE_SET
        and not path.startswith(_GENERATED_SAMPLE_DIRECTORIES)
        and not (path in fragments and not (repo_root / path).exists())
    }
    if unexpected_files:
        raise RuntimeError(
            "Release preparation changed unexpected files: "
            + ", ".join(sorted(unexpected_files))
        )


def verify_lockfile(repo_root: pathlib.Path) -> None:
    subprocess.run(
        ["cargo", "metadata", "--locked", "--format-version", "1"],
        cwd=repo_root,
        check=True,
        stdout=subprocess.DEVNULL,
    )


def regenerate_samples(repo_root: pathlib.Path) -> None:
    subprocess.run(["cargo", "build-examples"], cwd=repo_root, check=True)


def commit_release_changes(repo_root: pathlib.Path, version: str) -> None:
    subprocess.run(
        [
            "git",
            "add",
            "--all",
            "--",
            *_RELEASE_FILES,
            "changelog/",
            *_GENERATED_SAMPLE_DIRECTORIES,
        ],
        cwd=repo_root,
        check=True,
    )
    if changed_files(repo_root):
        subprocess.run(
            ["git", "commit", "-m", f"Prepare release {version}"],
            cwd=repo_root,
            check=True,
        )


def push_release_branch(repo_root: pathlib.Path, version: str) -> None:
    branch = f"chore/release-{version}"
    subprocess.run(
        ["git", "push", "--set-upstream", "origin", branch],
        cwd=repo_root,
        check=True,
    )


def create_release_pr(repo_root: pathlib.Path, version: str) -> tuple[str, bool]:
    """Create the release PR, or return the existing open release PR.

    Returns the PR URL and whether this invocation created it. A closed or
    merged PR with the release branch is not resumable: continuing would
    misleadingly report a usable PR when there is none.
    """
    branch = f"chore/release-{version}"
    existing_prs = subprocess.run(
        [
            "gh",
            "pr",
            "list",
            "--base",
            "main",
            "--head",
            branch,
            "--state",
            "all",
            "--json",
            "url,state",
        ],
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    )
    prs = json.loads(existing_prs.stdout)
    open_prs = [pr for pr in prs if pr["state"] == "OPEN"]
    if open_prs:
        subprocess.run(
            ["gh", "pr", "edit", open_prs[0]["url"], "--add-label", "skip-changelog"],
            cwd=repo_root,
            check=True,
        )
        return open_prs[0]["url"], False
    if prs:
        prior_prs = ", ".join(f"{pr['state'].lower()} {pr['url']}" for pr in prs)
        raise RuntimeError(
            f"Found {prior_prs} for release branch {branch!r}. "
            "Reopen that PR or use a new release branch before resuming."
        )

    created_pr = subprocess.run(
        [
            "gh",
            "pr",
            "create",
            "--base",
            "main",
            "--head",
            branch,
            "--title",
            f"Prepare release {version}",
            "--body",
            f"Prepare nexgen release {version}.",
            "--label",
            "skip-changelog",
        ],
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    )
    url = created_pr.stdout.strip()
    if not url:
        raise RuntimeError("GitHub CLI created a PR but did not return its URL")
    return url, True


def _replace_once(
    pattern: str,
    replacement: str,
    text: str,
    *,
    description: str,
) -> str:
    updated, count = re.subn(pattern, replacement, text, count=1)
    if count != 1:
        raise RuntimeError(f"Could not find {description}")
    return updated.rstrip("\n")


def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(
        description=(
            "Bump the crate version, assemble and consume changelog fragments, "
            "regenerate checked-in samples, "
            "and verify Cargo.lock."
        )
    )
    parser.add_argument("version", help="Release version, for example 0.3.0")
    parser.add_argument(
        "--date",
        default=datetime.date.today().isoformat(),
        help="Release date in YYYY-MM-DD format. Defaults to today.",
    )
    parser.add_argument(
        "--skip-lock",
        action="store_true",
        help="Do not run 'cargo metadata --locked'. Intended only for local testing.",
    )
    args = parser.parse_args(argv)

    repo_root = pathlib.Path(__file__).resolve().parents[1]
    version = validate_version(args.version)
    release_date = parse_date(args.date)
    ensure_clean_worktree(repo_root)
    create_release_branch(repo_root, version)

    fragments = {
        str(path.relative_to(repo_root))
        for path in (repo_root / "changelog").glob("*/*.md")
    }
    manifest_path = repo_root / "Cargo.toml"
    lock_path = repo_root / "Cargo.lock"

    manifest_text = (
        replace_manifest_version(
            manifest_path.read_text(encoding="utf-8"),
            version,
        )
        + "\n"
    )
    lock_text = (
        replace_lock_package_version(
            lock_path.read_text(encoding="utf-8"),
            version,
        )
        + "\n"
    )

    manifest_path.write_text(manifest_text, encoding="utf-8")
    lock_path.write_text(lock_text, encoding="utf-8")

    if not args.skip_lock:
        verify_lockfile(repo_root)

    prepare_changelog(repo_root, version=version, release_date=release_date)
    regenerate_samples(repo_root)
    ensure_only_release_changes(repo_root, fragments)
    commit_release_changes(repo_root, version)
    push_release_branch(repo_root, version)
    pr_url, created_pr = create_release_pr(repo_root, version)
    action = "Opened" if created_pr else "Found existing"
    print(f"{action} release PR: {pr_url}")
    print(f"Prepared release {version} dated {release_date.isoformat()}")


if __name__ == "__main__":
    main()
