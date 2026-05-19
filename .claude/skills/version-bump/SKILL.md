---
name: version-bump
description: Use only when explicitly invoked as $version-bump for this repository's Rust package release version bump workflow. Do not use implicitly for ordinary version-related discussion.
---

# Version Bump

Use this skill only after explicit `$version-bump` invocation.

## Workflow

1. Inspect the current package version in `Cargo.toml`.
2. Propose the next SemVer version and explain the reasoning.
3. Stop and wait for explicit user approval.
4. After approval, run `git status --short`.
5. If there are any existing unstaged, staged, or uncommitted changes, reject the version bump request and stop.
6. Update only the package version in `Cargo.toml`.
7. Run `cargo check` to refresh `Cargo.lock`.
8. Recheck `git status --short`.
9. If files other than `Cargo.toml` and `Cargo.lock` changed, stop and report the unexpected changes.
10. Commit only `Cargo.toml` and `Cargo.lock`.
11. Tag the commit with the version number.
12. Push the current branch commit.
13. Push the tag.

## Commit And Tag Rules

- Use a conventional commit message: `chore(release): bump version to X.Y.Z`.
- Do not append an issue reference unless the user provides one.
- Use tag name `vX.Y.Z`.
- Push the release commit before pushing the tag.
- If pushing the tag fails because of remote or authentication issues, report the failure clearly.
