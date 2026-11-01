## Summary

<!-- What does this PR change, and why? -->

## Scope

- [ ] Rust workspace (`crates/`)
- [ ] Web client (`apps/web/`)
- [ ] Desktop shell (`apps/desktop/`)
- [ ] Server deployment (`Dockerfile`, `deploy/`)
- [ ] CI / docs only

## Test plan

<!-- How did you verify this works? Include commands run, e2e specs added, etc. -->

## Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo test` passes (or justified why not applicable)
- [ ] `cargo clippy --workspace --all-targets` has no new warnings
- [ ] `cd apps/web && npx playwright test` passes (for UI / protocol changes)
- [ ] No secrets, private keys, or personal data are added
- [ ] If changing protocol or crypto, linked to design doc / prior discussion
