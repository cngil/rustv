# Contributing to rustv

Thanks for helping. rustv is a private, light, scriptable TV shell for Linux.
Contributions that keep it that way are welcome.

## Ground rules

- **No telemetry, accounts or cloud calls.** rustv never connects to the
  internet. Changes that add network traffic from `tvd` or the shell will not
  be merged.
- **Stay light.** The daemon targets low-end hardware. Avoid heavy
  dependencies and idle CPU use.
- **Config over hardcoding.** User-visible behavior belongs in the TOML
  config, not in the source.
- **Optional by default.** New on-screen information or features must be
  switchable by the user.

## Getting started

```sh
git clone https://github.com/cngil/rustv && cd rustv
cargo build
RUST_LOG=debug cargo run -p tvd -- --config config/config.example.toml
```

Requirements are listed in the [README](README.md#requirements). Rust 1.80+
is needed.

## Before you open a pull request

All three must pass:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Formatting follows [`rustfmt.toml`](rustfmt.toml).

## Workflow

1. Open an issue first for anything larger than a small fix, so we can agree
   on the approach.
2. Fork, then branch from `main`.
3. Keep each pull request focused on one change.
4. Update the README, `config/config.example.toml` and the API section when
   you change behavior, config keys or endpoints.
5. Add tests for new logic where practical.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/): `feat`,
`fix`, `docs`, `refactor`, `test`, `chore`, with a scope when useful.

```
feat(tvd): add battery state to /api/v1/state
fix(remote): stop key repeat when the socket closes
docs: describe the pairing flow
```

Split unrelated changes into separate commits.

## Language

Code, comments, commit messages, issues and docs are in English.

## Security

Do not report vulnerabilities in public issues. Contact the maintainer
privately through GitHub. See the [Security section](README.md#security) of
the README for the threat model.

## License

By contributing you agree that your work is licensed under
[GPL-3.0-or-later](LICENSE).
