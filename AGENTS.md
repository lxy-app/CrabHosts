# CrabHosts agent guide

## Project map

- `crates/core`: UI-independent model, validation, profile storage, managed hosts blocks, backups, atomic writes.
- `crates/cli`: the `crabhosts` binary and stable JSON automation contract.
- `apps/desktop`: Slint desktop shell; all system operations must go through `crabhosts-core`.
- `skills/crabhosts`: canonical Agent Skill. Update it whenever the CLI contract or safety workflow changes.
- `docs/sync.md`: synchronization design. Do not describe planned network features as implemented.

## Toolchain and commands

The workspace uses Rust 2024 edition and declares Rust 1.88 as MSRV in `Cargo.toml`.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p crabhosts-cli -- --help
cargo run -p crabhosts-desktop
```

Slint UI lives in `apps/desktop/ui/main.slint` and is compiled by `apps/desktop/build.rs`.

## Safety boundaries

- Never use the real `/etc/hosts` or Windows hosts file in tests.
- Tests and manual demos must set both `CRABHOSTS_HOME` and `CRABHOSTS_HOSTS_FILE` to temporary paths.
- Profile edits must not implicitly modify system hosts.
- System writes require a plan and explicit confirmation. Profile apply preserves unmanaged lines; restore intentionally restores a complete backup snapshot. Always create a backup and reject stale plans.
- AI-driven writes must bind approval to the dry-run `plan_id` by passing `--expected-plan <plan-id>` together with `--yes`.
- Never add implicit `sudo`, AppleScript elevation, shell interpolation, or remote-triggered automatic apply.
- Do not commit credentials, sync keys, real user hosts backups, or local absolute paths.
- JSON success envelope: `{ok, command, data}`; error envelope: `{ok, error: {code, message}}`.
- Confirmation required exits 3, permission denied exits 4, stale plan/conflict exits 5.

## Change conventions

- Keep synchronization metadata and stable entry/profile IDs in the core model; device-local apply state must not leak into synced documents.
- Validate IP addresses, hostnames, names, comments, schema versions, and managed block markers in the core layer.
- Prefer additive CLI changes. If output fields change, update `skills/crabhosts/references/cli.md` and integration tests in the same change.
- Do not mark sync, privilege helpers, or platform support complete without an implementation and real verification.
