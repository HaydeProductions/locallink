# LocalLink production readiness

This document tracks the remaining work needed before LocalLink should be treated as production ready.

## Implemented

- GitHub Actions CI for formatting, clippy warnings, workspace tests, RustSec audit, and Windows packaging smoke tests.
- Config and trusted-device writes use temporary files plus rename to reduce partial-write corruption risk.
- Discovery-time trusted identity binding has been removed from manual connect.
- Removing a trusted MAC now disconnects stored trusted device IDs as well as currently discovered peers.
- The core add-on manager now stops only child processes it launched and tracks directly; wildcard image-name/process-name killing has been removed from the core.
- Discovery now enumerates all active physical Windows network adapters instead of only Ethernet-named adapters.
- Discovery now evicts stale peers after a bounded TTL.

## Required release blockers

1. Replace the shared destructive event queue with per-consumer delivery. The UI and each add-on must have separate cursors so one client cannot consume another client's events.
2. Move final trust binding until after transport authentication succeeds. Discovery metadata must not be persisted as trusted identity before authentication.
3. Reject transport handshakes when HELLO device identity and authenticated response identity do not match.
4. Add receive-side replay protection for encrypted frames by rejecting duplicate or non-monotonic frame sequence numbers.
5. Add a heartbeat watchdog that disconnects peers after missed heartbeat responses.
6. Replace build-time source rewriting in the UI with explicit source modules.
7. Re-enable strict clippy (`-D warnings`) once existing UI warnings are fixed.
8. Add an explicit cargo-deny policy file before re-enabling dependency policy CI.

## Validation gate

Run these before merging or deploying:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features
cargo test --workspace --all-features
cargo audit
```

On Windows, also run:

```powershell
./scripts/build-release.ps1
```
