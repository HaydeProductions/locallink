# LocalLink production readiness

This branch is a deployment-hardening branch. It keeps `main` unchanged and adds production gates plus a concrete list of reliability fixes that should be completed before release.

## Implemented in this branch

- Adds a GitHub Actions CI workflow for formatting, clippy, workspace tests, security audit, dependency policy checks, and Windows packaging smoke tests.
- Preserves the branch separately as `deployment` so review and release testing can happen without changing `main`.

## Required release blockers

1. Replace the shared destructive event queue with per-consumer delivery. The UI and each add-on must have separate cursors so one client cannot consume another client's events.
2. Move trust binding until after transport authentication succeeds. Discovery metadata must not be persisted as trusted identity before authentication.
3. Reject transport handshakes when HELLO device identity and authenticated response identity do not match.
4. Add receive-side replay protection for encrypted frames by rejecting duplicate or non-monotonic frame sequence numbers.
5. Add a heartbeat watchdog that disconnects peers after missed heartbeat responses.
6. Remove wildcard image-name process killing. The core should stop only processes it owns by child handle or exact recorded PID and path.
7. Enumerate all active physical network adapters for discovery, not only Ethernet-named adapters.
8. Evict stale discovered peers after a bounded TTL.
9. Replace build-time source rewriting in the UI with explicit source modules.
10. Write config and trusted-device files atomically via temporary file plus rename.

## Validation gate

Run these before merging or deploying:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

On Windows, also run:

```powershell
./scripts/build-release.ps1
```
