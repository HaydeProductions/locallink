# LocalLink Architecture Contract

This document records the current redesign direction for LocalLink. It is intentionally conservative for the first implementation phase: preserve the existing UI design and behaviour where possible, while moving runtime ownership and future connection modelling into the backend.

## Phase 1 scope

Phase 1 is an architecture and configuration foundation only.

It may add documentation and backend-safe configuration fields. It should not redesign the current widget-style UI, rename executables, or move add-on/runtime ownership in this step.

## UI preservation rule

The current UI style should be preserved during the redesign. Backend changes should be preferred over visible UI changes. When a UI change becomes necessary in later phases, it should fit the current visual language rather than replacing it.

The UI remains a controller. It should request Core actions through the local control API; it should not become the owner of transport sessions, spaces, or add-on child processes.

## Executable roles

### LocalLink.exe

`LocalLink.exe` is the future product entry point and startup organiser.

Responsibilities:

- read startup preferences
- start `LocalLinkTray.exe` when `startup.use_tray = true`
- start `LocalLinkUI.exe` when `startup.launch_ui = true`
- if Tray was the only configured entry point and fails, start UI anyway with enough context to explain the Tray failure
- exit after organising startup unless a later design requires it to remain resident

It must not own Core, Tray, UI, transport sessions, spaces, or add-ons.

Phase 1 does not rename the current packaged binaries or introduce launcher logic.

### LocalLinkUI.exe

`LocalLinkUI.exe` is the main control interface.

Responsibilities:

- show Core status
- start, stop, and restart Core
- manage trusted devices
- manage direct and group spaces once the space model exists
- manage add-on desired state per space once add-ons are space-scoped
- edit startup preferences
- show activity and settings

Closing the UI must not kill Core, Tray, active spaces, transport sessions, or add-ons.

### LocalLinkTray.exe

`LocalLinkTray.exe` is an optional lightweight controller.

Responsibilities:

- show Core status
- open or focus UI
- request Core start or stop
- quit Tray
- optionally show quick space/device status later

Tray must not own Core, spaces, transport sessions, or add-ons.

### LocalLinkCore.exe

`LocalLinkCore.exe` is the runtime owner.

Responsibilities:

- discovery
- trusted device identity
- encrypted peer transport sessions
- authentication and encryption
- local API
- connection spaces
- group routing
- add-on runtimes

Core owns runtime state. UI, Tray, and Launcher are entry points/controllers only.

## Startup preferences

Startup preferences live under `startup` in `config.json`:

```json
{
  "startup": {
    "launch_ui": true,
    "use_tray": false
  }
}
```

Invariant:

```text
startup.launch_ui || startup.use_tray
```

At least one user entry point must always be enabled.

Defaults:

```json
{
  "launch_ui": true,
  "use_tray": false
}
```

If the config is manually edited so both are false, Core repairs the preferences back to the defaults. Tray failure should not permanently disable Tray; a future launcher should open UI for that run and explain the failure.

## Core connection model direction

The backend should move from exposing raw peer connections as the main user model to exposing `ConnectionSpace` records.

A `ConnectionSpace` is the user/API/add-on context where devices communicate. There are two kinds:

- direct space: one remote member
- group space: multiple remote members

Direct and group connections should not become separate systems. Both should use the same space model.

Underneath spaces, Core keeps the existing per-peer encrypted transport sessions stable:

```text
PeerSession = transport-level encrypted connection to one device
ConnectionSpace = user/API/add-on relationship using one or more PeerSessions
```

A group is not a shared socket. Group messages are routed by Core over the existing per-peer encrypted sessions.

## Add-on ownership direction

Add-ons should become space-scoped. The long-term ownership model is:

```text
LocalLinkCore
  SpaceManager
    ConnectionSpace
      SpaceAddonRuntime
```

Add-on manifests can continue to define what an add-on is. Whether an add-on should run should eventually be stored as desired state on each space.

UI and Tray should only request desired-state changes. They should not spawn or kill add-on child processes.

## Compatibility during migration

Existing peer-level commands and direct connection behaviour should remain while space-aware APIs are added.

The first backend phases should preserve:

- existing encrypted per-peer transport
- manual connection behaviour
- direct Ethernet stability
- existing UI style and primary flows
- existing add-on manifests until space-scoped desired state is introduced

Group-level cryptography is explicitly out of scope for the first space implementation. Initial group routing should use the existing encrypted peer sessions.
