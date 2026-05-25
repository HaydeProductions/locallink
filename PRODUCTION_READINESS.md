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
- Transport writes no longer use an aggressive 3-second cancellation timeout that could interrupt a frame mid-write.
- TCP_NODELAY is enabled for low-latency direct-link traffic.
- Heartbeats are conservative: less frequent, and the connection is removed only after repeated heartbeat write failures.

## Required release blockers

1. Replace the shared destructive event queue with per-consumer delivery. The UI and each add-on must have separate cursors so one client cannot consume another client's events.
2. Move final trust binding until after transport authentication succeeds. Discovery metadata must not be persisted as trusted identity before authentication.
3. Reject transport handshakes when HELLO device identity and authenticated response identity do not match.
4. Add receive-side replay protection for encrypted frames by rejecting duplicate or non-monotonic frame sequence numbers within a single authenticated session.
5. Replace build-time source rewriting in the UI with explicit source modules.
6. Re-enable strict clippy (`-D warnings`) once existing UI warnings are fixed.
7. Add an explicit cargo-deny policy file before re-enabling dependency policy CI.

## Stability principle

LocalLink is expected to maintain high-speed direct-link connections for hours. Connection-health checks must be conservative: any valid inbound encrypted traffic should count as liveness, reconnects should start fresh session-local counters, and watchdogs must not disconnect on brief stalls or a single failed heartbeat write.

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
