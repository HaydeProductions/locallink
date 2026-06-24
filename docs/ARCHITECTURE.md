# LocalLink architecture contract

This document records the intended ownership boundaries for the LocalLink redesign. It is deliberately staged: the current codebase can adopt this contract before every executable rename, UI screen, or protocol change exists.

## Executable ownership

### `LocalLink.exe`

`LocalLink.exe` is the product entry point and startup organiser.

It should:

- read startup preferences from config;
- start `LocalLinkTray.exe` when `startup.use_tray` is true;
- start `LocalLinkUI.exe` when `startup.launch_ui` is true;
- start UI anyway when Tray was the only configured entry point and Tray fails, so the user can see a clear failure message;
- exit after organising startup unless a later design gives it a resident responsibility.

It must not own Core, Tray, UI state, transport sessions, spaces, or add-on processes. It must not auto-start Core just because the app was opened.

### `LocalLinkUI.exe`

`LocalLinkUI.exe` is the main control interface.

It should show status and send requests through the local API for actions such as Core start/stop, trusted-device management, space management, add-on desired-state changes, settings, and activity/events.

It must not own Core, Tray, transport sessions, connection spaces, or add-on child processes. Closing the UI must not kill Core, Tray, active spaces, or add-ons.

### `LocalLinkTray.exe`

`LocalLinkTray.exe` is an optional lightweight controller.

It should show Core status, open/focus UI, request Core start/stop, and quit itself. Later it may show quick space/device status.

It must not own Core, UI, transport sessions, connection spaces, or add-on child processes.

### `LocalLinkCore.exe`

`LocalLinkCore.exe` is the runtime owner.

It owns discovery, trusted device identity, encrypted peer transport sessions, the local control API, connection spaces, group routing, and add-on runtimes.

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

When the config is missing these fields, LocalLink defaults to UI-only startup. When the config is manually corrupted so both are false, LocalLink repairs it back to UI-only startup.

## Runtime ownership model

Core is the runtime owner. UI and Tray are controllers.

```text
LocalLinkCore
  ├── TransportManager
  │   └── PeerSession map
  │       └── peer_id -> encrypted TCP session
  └── SpaceManager
      └── ConnectionSpace map
          └── space_id -> direct/group context
```

A peer session is the secure per-device transport connection. A connection space is the user-facing and API-facing context that uses one or more peer sessions.

## Connection spaces

LocalLink should model both direct connections and groups as `ConnectionSpace` records rather than separate systems.

- A direct space contains at most one remote member.
- A group space can contain multiple members.
- Direct and group spaces share the same persistence, API, routing, and add-on ownership model.

Persistent spaces live in `%APPDATA%\\LocalLink\\spaces.json`:

```json
{
  "spaces": [
    {
      "space_id": "desktop-space-id",
      "name": "Desktop",
      "kind": "direct",
      "members": ["desktop-device-id"],
      "addons": {
        "clipboard": { "enabled": true }
      }
    },
    {
      "space_id": "office-space-id",
      "name": "Office",
      "kind": "group",
      "members": ["desktop-device-id", "laptop-device-id"],
      "addons": {
        "clipboard": { "enabled": true },
        "file-transfer": { "enabled": false }
      }
    }
  ]
}
```

## Group routing

The first group implementation should route over existing per-peer encrypted sessions.

Default group send behaviour is broadcast: when no target peer is specified, Core sends one encrypted message to each active member of the space.

Direct send inside a group must also be supported at the API layer: when a target peer is specified, Core must validate that the target is a member of the space and currently connected, then send only to that peer.

Group sends should report per-peer delivery results, and a partial delivery failure should not necessarily fail the whole group send.

## Encryption phasing

The first redesign phase keeps the existing per-peer encryption model. Space metadata travels inside encrypted per-peer payloads.

Group-level cryptography is a later hardening phase after space routing is stable. Possible later work includes signed group membership, shared group identity, optional group message keys, and key rotation when members are added or removed.

## Add-on ownership

Add-ons belong to spaces, not to UI and not to global Core state long-term.

```text
LocalLinkCore
  └── SpaceManager
      └── ConnectionSpace
          └── SpaceAddonRuntime
              ├── clipboard-sync instance for this space
              └── file-transfer instance for this space
```

The UI and Tray may request add-on desired-state changes through the local API. Core applies those changes and owns all add-on process lifecycle decisions.

## Staging rule

Do not implement this as one oversized change. The intended sequence is:

1. architecture contract and startup preference foundation;
2. persistent `spaces.json` model;
3. Core state wiring;
4. space-aware local API;
5. space-aware protocol/events;
6. add-on runtime ownership migration;
7. UI migration;
8. executable restructure;
9. Tray integration;
10. later group crypto hardening.
