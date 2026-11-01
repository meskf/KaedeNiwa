# Contributing to KaedeNiwa

Thanks for your interest. KaedeNiwa is source-available under the
[PolyForm Noncommercial 1.0.0](LICENSE) license. By opening a pull request
you agree that your contribution is licensed under the same terms.

## Scope

We welcome:

- Bug reports with reproducible steps
- Security reports through [Security Advisories](https://github.com/meskf/KaedeNiwa/security/advisories/new), **not** public issues
- Documentation fixes
- Test coverage for existing behaviour
- Small, focused changes with a clear test plan

We are cautious about:

- Cryptographic changes. Any change to `kaede-crypto`, `kaede-mls`, or the
  DM/channel wire format should link to a design discussion first.
- Dependency upgrades in the crypto stack (`openmls`, `rustls`, `aes-gcm`).
  These will be reviewed manually, even if dependabot opens the PR.
- Adding a new workspace crate. Prefer extending an existing one.

## Development setup

Rust workspace:

~~~
cargo build -p kaede-server --features dev-mode
~~~

WASM client and frontend:

~~~
cd apps/web
npm ci
npm run build:wasm
npm run dev
~~~

The default `playwright.config.ts` auto-starts both the server and vite.
See `.github/workflows/ci.yml` for the exact sequence used in CI.

## Pre-commit checks

Before opening a PR, run:

~~~
cargo fmt --all -- --check
cargo clippy --workspace --exclude kaede-client-wasm --all-targets -- -D warnings
cargo test --workspace --exclude kaede-client --exclude kaede-client-wasm
cargo test -p kaede-client --features native
cargo deny check
~~~

If you change UI or protocol behaviour, also run:

~~~
cd apps/web
npx playwright test
~~~

## Reviewing

Maintainer reviews focus on:

1. Does the change preserve the E2EE boundary? (server must never see plaintext or group keys)
2. Does it break wire compatibility? If so, is a version bump in place?
3. Are failure modes handled? (offline, replay, key mismatch, epoch race)
4. Is there a test that would catch a regression?
