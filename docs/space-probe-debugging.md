# Space Probe Debugging

`space-probe` is a deliberately small add-on for validating LocalLink space add-on behavior before debugging real add-ons like clipboard sync.

It is not a product feature. It exists to answer these questions in order:

1. Did core launch the add-on inside a space context?
2. Did the add-on receive `LOCALLINK_SPACE_ID`, `LOCALLINK_SPACE_KIND`, and `LOCALLINK_ADDON_INSTANCE_ID`?
3. Does `send_space_message` succeed from that space context?
4. Which peer IDs did core try to deliver to?
5. Does the other device receive a `space_service_data` event with the expected `space_id`?
6. Do per-instance event cursors work correctly?

## Build/install

From the repo root:

```bash
bash scripts/build.sh
```

or on Windows PowerShell:

```powershell
.\scripts\build-release.ps1
```

Both scripts build `locallink-addon-space-probe.exe`, package it into `dist\LocalLink\addons\space-probe`, and install it into:

```text
%APPDATA%\LocalLink\addons\space-probe
```

Core loads add-ons from AppData, so the AppData install is the important part for live debugging.

## Usage

1. Start LocalLink core/UI normally.
2. Reload add-ons or restart core so `space-probe` appears in the add-on list.
3. In a group space, enable `Space Probe` on both devices.
4. Connect/activate the space if required by the current build.
5. Watch the logs.

## Logs

The probe writes logs because core suppresses add-on stdout/stderr.

Look under:

```text
%LOCALAPPDATA%\LocalLink\logs\space-probe-*.log
```

or, if `%LOCALAPPDATA%` is unavailable:

```text
%APPDATA%\LocalLink\logs\space-probe-*.log
```

Each log records:

- add-on ID
- instance ID
- core API address
- `LOCALLINK_SPACE_ID`
- `LOCALLINK_SPACE_KIND`
- `LOCALLINK_SPACE_NAME`
- `LOCALLINK_CONNECTED_MEMBERS`
- every `send_space_message` response
- every received event for service `locallink.debug.space.probe`
- ignored events and why they were ignored
- accepted `probe_ping` and `probe_pong` messages

## Expected healthy behavior

On each device, the log should show startup context similar to:

```text
LocalLink Space Probe starting
space_id=Some("...")
space_kind=Some("group")
connected_members=["..."]
```

Then every few seconds:

```text
sent probe_ping seq=1 response={..."deliveries":[{"peer_id":"...","ok":true,...}]}
event kind=space_service_data peer=... event_space=Some("...") expected_space=...
accepted probe_ping seq=1 from ...
sent probe_pong seq=1 response={...}
```

## Interpreting failures

### No log file

The add-on was not launched. Check whether it is installed in AppData, loaded by core, enabled on the space, and whether the space is active under the current runtime rules.

### Log exists but `space_id=None`

Core launched the executable without a space context. The issue is in add-on launch/runtime context, not packet delivery.

### `send_space_message failed`

The add-on has context and can reach core, but the space send API rejected the request. The error text in the log is the next thing to inspect.

### Deliveries contain `ok:false`

Core accepted the space send, but could not deliver to one or more peer IDs. This points at connection state or fanout membership shape.

### Sender logs `ok:true`, receiver sees nothing

The send path claims delivery, but the receiver did not push or expose the event. Inspect `FRAME_SPACE_SERVICE_DATA` receive handling and `poll_events` behavior.

### Receiver logs `space_id mismatch`

The receiver is seeing space events, but not for the space instance it is running in. That points to add-on event filtering or incorrect launch context.
