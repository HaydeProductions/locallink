# LocalLink Guide

LocalLink is a local-device connection platform.

Users add trusted devices by MAC address, manually connect to them, and enable add-ons that use the secure LocalLink transport in the background.

LocalLink is not just a file sharing app. File sharing, clipboard sync, notifications, chat, media control, and other features should be implemented as add-ons that use LocalLink as the secure transport layer.

## Project Structure

LocalLink is a Rust workspace:

- locallink-core: core daemon, discovery, trust management, secure transport, local API, add-on routing
- locallink-ui: widget-style desktop UI
- locallink-addon-echo: example add-on
- scripts: build and helper scripts
- docs: documentation

## Core Idea

1. LocalLink discovers nearby devices.
2. The user adds a trusted device by MAC address.
3. The user manually clicks Connect.
4. LocalLink authenticates and encrypts the session.
5. Add-ons use the secure connection through LocalLink Core.

Discovery is automatic.
Connection is manual.
Add-ons do not handle networking themselves.

## Main Components

### locallink-core

locallink-core.exe is the background engine.

It handles:

- device discovery
- trusted MAC storage
- manual connection requests
- PSK authentication
- encrypted peer sessions
- local API
- add-on routing
- service events
- channel events

The UI and add-ons talk to the core through a local API on:

127.0.0.1:47900

This API should remain local-only.

### locallink-ui

LocalLink.exe is the user-facing widget.

Main UI areas:

- Discover: shows nearby devices and lets users add devices by MAC address
- Devices: shows trusted devices and lets users manually connect or disconnect
- Add-ons: shows installed add-ons and lets users enable or disable them
- Activity: shows recent add-on/device activity
- Settings: core controls and advanced technical details

### Add-ons

Add-ons are separate executables that use the LocalLink API.

Examples:

- Clipboard Sync
- File Transfer
- Notifications
- Chat
- Remote Commands
- Media Control

Add-ons should not implement discovery, encryption, reconnection, or peer routing. They should call LocalLink Core.

## Storage Layout

LocalLink stores runtime and user data in:

%APPDATA%\LocalLink

Typical layout:

- config.json
- trusted-devices.json
- trusted-peers.json
- addons/
- logs/
- runtime/
- state/

Example add-on location:

%APPDATA%\LocalLink\addons\example-echo\manifest.json

## Device Workflow

User workflow:

1. Start LocalLink on both devices.
2. Open Discover.
3. Nearby devices appear.
4. Add a device by MAC address.
5. Open Devices.
6. Click Connect.
7. LocalLink creates a secure encrypted session.
8. Enable add-ons as needed.

LocalLink should not auto-connect simply because a nearby device is discovered.

## Discovery

LocalLink discovery uses local IPv6 multicast.

Nearby devices announce:

- device_id
- device_name
- tcp_port
- MAC address hints

Discovery does not establish trust. It only answers:

"What nearby devices are visible?"

Trust is established through:

- user-added MAC address
- shared PSK authentication
- successful encrypted session

## Trust Model

Current prototype trust model:

- MAC address: user-facing device hint and connection target
- PSK: used to authenticate and derive encrypted session keys
- device_id: internal stable LocalLink identity

Important distinction:

- MAC address = convenient user-facing target
- PSK/auth = actual cryptographic authorization
- device_id = internal LocalLink identity

MAC addresses can be spoofed. Do not treat MAC addresses as cryptographic proof.

## Core API

The core API is JSON-over-TCP on:

127.0.0.1:47900

Each request is a single JSON line. Each response is a single JSON line.

Success response:

{
  "ok": true,
  "data": {},
  "error": null
}

Error response:

{
  "ok": false,
  "data": null,
  "error": "message"
}

## Core API Commands

### status

Request:

{"cmd":"status"}

Returns core status.

### paths

Request:

{"cmd":"paths"}

Returns LocalLink storage paths.

### list_peers

Request:

{"cmd":"list_peers"}

Lists nearby discovered devices.

Example peer data:

{
  "device_id": "...",
  "device_name": "LAPTOP-L125KA6O",
  "addr": "[fe80::...%19]:47800",
  "macs": ["aa:bb:cc:dd:ee:ff"],
  "trusted": true,
  "trusted_name": "Other Laptop",
  "connected": false,
  "last_seen_ms_ago": 500
}

### list_trusted_devices

Request:

{"cmd":"list_trusted_devices"}

Lists devices saved by MAC address.

### add_trusted_device

Request:

{
  "cmd": "add_trusted_device",
  "name": "Other Laptop",
  "mac": "aa:bb:cc:dd:ee:ff"
}

Adds a device to the trusted device list.

### remove_trusted_device

Request:

{
  "cmd": "remove_trusted_device",
  "mac": "aa:bb:cc:dd:ee:ff"
}

Removes a trusted MAC. If a matching device is connected, the core should disconnect it.

### connect_device

Request by MAC:

{
  "cmd": "connect_device",
  "mac": "aa:bb:cc:dd:ee:ff"
}

Request by device ID:

{
  "cmd": "connect_device",
  "peer_id": "a198e488-3e32-4a9e-b409-ff43e4463630"
}

A device must be trusted before connecting.

### disconnect_device

Request by MAC:

{
  "cmd": "disconnect_device",
  "mac": "aa:bb:cc:dd:ee:ff"
}

Request by device ID:

{
  "cmd": "disconnect_device",
  "peer_id": "a198e488-3e32-4a9e-b409-ff43e4463630"
}

### list_connections

Request:

{"cmd":"list_connections"}

Lists active secure sessions.

### list_addons

Request:

{"cmd":"list_addons"}

Lists installed add-ons.

### reload_addons

Request:

{"cmd":"reload_addons"}

Reloads add-on manifests from storage.

### shutdown

Request:

{"cmd":"shutdown"}

Shuts down LocalLink Core.

## Add-on Messaging API

Add-ons send and receive messages through service names.

Example service names:

- test.echo
- clipboard-sync
- file-transfer
- notifications

### send_message

Sends a small message to a connected peer.

{
  "cmd": "send_message",
  "peer_id": "a198e488-3e32-4a9e-b409-ff43e4463630",
  "service": "test.echo",
  "data_b64": "SGVsbG8="
}

data_b64 is a base64-encoded payload.

### poll_events

Poll received events:

{"cmd":"poll_events"}

Poll one service:

{
  "cmd": "poll_events",
  "service": "test.echo.reply",
  "max_events": 50
}

### wait_events

Long-poll for events:

{
  "cmd": "wait_events",
  "service": "clipboard-sync",
  "wait_ms": 30000,
  "max_events": 50
}

This is preferred for add-ons because it avoids busy polling.

## Channel API

Channels are for multi-message interactions.

Lifecycle:

1. open_channel
2. channel_send
3. channel_close

### open_channel

{
  "cmd": "open_channel",
  "peer_id": "...",
  "service": "file-transfer"
}

### channel_send

{
  "cmd": "channel_send",
  "peer_id": "...",
  "service": "file-transfer",
  "channel_id": "...",
  "data_b64": "..."
}

### channel_close

{
  "cmd": "channel_close",
  "peer_id": "...",
  "service": "file-transfer",
  "channel_id": "...",
  "reason": "done"
}

Remote events:

- channel_open
- channel_data
- channel_close

## Add-on Manifest

Location:

%APPDATA%\LocalLink\addons\<addon-id>\manifest.json

Example:

{
  "id": "example-echo",
  "name": "Example Echo Addon",
  "version": "0.1.0",
  "description": "Simple addon that listens on test.echo and replies on test.echo.reply.",
  "executable": "locallink-addon-echo.exe",
  "services": [
    "test.echo",
    "test.echo.reply"
  ],
  "enabled": true
}

Fields:

- id: stable add-on identifier
- name: human-friendly display name
- version: add-on version
- description: short UI description
- executable: executable relative to the add-on folder
- services: LocalLink service names the add-on uses
- enabled: whether the UI should consider this add-on enabled

Current behaviour:

- UI toggles enabled=true/false in the manifest.
- UI may launch/stop the add-on process.

Future behaviour:

- Core manages add-on process lifecycle.
- UI asks core to enable/disable add-ons.

## Writing an Add-on

An add-on is a normal executable that talks to:

127.0.0.1:47900

It does not need to know about:

- IPv6
- MAC discovery
- PSK auth
- encryption
- routing
- reconnection

A basic add-on loop:

1. Call wait_events for your service name.
2. Handle incoming events.
3. Use send_message or channel_send to respond.

Minimal request:

{
  "cmd": "wait_events",
  "service": "my-service",
  "wait_ms": 30000,
  "max_events": 25
}

Minimal reply:

{
  "cmd": "send_message",
  "peer_id": "peer-id-from-event",
  "service": "my-service.reply",
  "data_b64": "..."
}

## Example Add-on: Echo

locallink-addon-echo listens on:

test.echo

and replies on:

test.echo.reply

Run manually:

./dist/LocalLink/addons/example-echo/locallink-addon-echo.exe

Send test message:

./dist/LocalLink/locallink-core.exe --api send --peer DEVICE_ID --service test.echo --text "hello echo addon"

Poll reply:

./dist/LocalLink/locallink-core.exe --api poll --service test.echo.reply

## API Helper CLI

locallink-core.exe can act as a CLI helper.

Examples:

./dist/LocalLink/locallink-core.exe --api status
./dist/LocalLink/locallink-core.exe --api peers
./dist/LocalLink/locallink-core.exe --api trusted
./dist/LocalLink/locallink-core.exe --api connections
./dist/LocalLink/locallink-core.exe --api addons

Add trusted device:

./dist/LocalLink/locallink-core.exe --api trust-mac --mac aa:bb:cc:dd:ee:ff --name "Other Laptop"

Connect:

./dist/LocalLink/locallink-core.exe --api connect --mac aa:bb:cc:dd:ee:ff

Disconnect:

./dist/LocalLink/locallink-core.exe --api disconnect --mac aa:bb:cc:dd:ee:ff

## Build and Run

From repo root:

cargo check
cargo build --release
powershell.exe -ExecutionPolicy Bypass -File scripts/build-release.ps1

Run UI:

./dist/LocalLink/LocalLink.exe

Run core directly:

./dist/LocalLink/locallink-core.exe

Shutdown core:

./dist/LocalLink/locallink-core.exe --api shutdown

## Current Limitations

This is still a prototype.

Current limitations:

- PSK is global rather than per trusted device.
- Add-on process management is partly handled by the UI.
- Service/channel events are queued in memory.
- Large binary transfers should not use base64 JSON.
- No installer yet.
- No code signing yet.
- No tray integration yet.
- No per-add-on permissions model yet.

Future improvements:

- Per-device secrets or public-key identities.
- Core-managed add-on lifecycle.
- Persistent event subscriptions.
- Named pipes instead of localhost TCP for local API.
- Binary streaming API for large files.
- OS keychain/credential storage.
- System tray mode.
- Installer and auto-start support.

## Security Notes

Discovery is not trusted.

Security comes from:

- shared PSK authentication
- session key derivation
- encrypted post-auth frames
- trusted device MAC list
- manual user connection

MAC addresses are user-friendly hints, not cryptographic proof.

The API should stay bound to:

127.0.0.1

Do not expose it to the LAN.

## Add-on Developer Guidelines

Add-ons should:

- Use LocalLink API instead of raw networking.
- Use service names consistently.
- Use wait_events instead of busy polling.
- Keep send_message payloads small.
- Use channels for multi-message operations.
- Treat remote data as untrusted input.
- Handle core disconnects.

Add-ons should not:

- Open their own LAN sockets for LocalLink transport.
- Implement their own peer discovery.
- Implement their own crypto over LocalLink.
- Assume MAC addresses are secure identity.
- Send huge files as one base64 JSON message.

## GitHub Notes

Commit the whole workspace:

- Cargo.toml
- Cargo.lock
- locallink-core/
- locallink-ui/
- locallink-addon-echo/
- scripts/
- docs/
- README.md

Do not commit:

- target/
- dist/
- %APPDATA% files
- config.json
- trusted-devices.json
- trusted-peers.json
- logs/
- runtime/
- state/
- PSKs/secrets
