# LocalLink startup architecture

This document defines the ownership boundaries for the main LocalLink executables. It is intentionally small and should guide later implementation work.

## Executable roles

### LocalLink.exe

`LocalLink.exe` is the product entrypoint and initial organiser.

It reads startup preferences and launches the entry components that the user has chosen. It is not the UI, it is not the runtime engine, and it does not own add-on processes.

For now, startup preferences are limited to:

- `launch_ui`: open the UI when LocalLink is launched.
- `use_tray`: start the tray entry point when LocalLink is launched.

At least one of these preferences must be enabled so the user always has an entry point back into LocalLink.

### LocalLinkUI.exe

`LocalLinkUI.exe` is the control interface.

It can let the user change preferences, start or stop Core, connect or disconnect devices, and enable or disable add-ons. It does not own Core, Tray, or add-on process lifetimes.

Closing the UI must not stop Core, Tray, or add-ons.

### LocalLinkTray.exe

`LocalLinkTray.exe` is an optional tray entry point and lightweight controller.

It can expose actions such as opening the UI or requesting Core start/stop. It does not own add-on processes.

If `LocalLink.exe` is configured to start only Tray and Tray fails to launch or initialise, `LocalLink.exe` must launch the UI as a fallback and pass enough context for the UI to show a clear error message.

### LocalLinkCore.exe

`LocalLinkCore.exe` is the runtime owner.

It owns discovery, trusted devices, secure transport, the local control API, device connections, and add-on process lifecycle.

If Core stops, all Core-owned add-ons stop. Desired add-on enabled/disabled state must remain persisted so the next Core launch can reconcile it.

### Add-ons

Add-ons are worker executables owned by Core.

The UI and Tray may request add-on changes, but only Core should start or stop add-on processes while Core is running.

## Startup preference invariant

The startup preference state is valid only when:

```text
launch_ui || use_tray
```

If a config file ever contains both as `false`, the app should repair it by enabling `launch_ui`.

## Current implementation steps

1. Store and validate the startup preferences in shared config.
2. Later, split the current UI binary from the product entrypoint.
3. Later, implement `LocalLink.exe` as the organiser using the shared preferences.
4. Later, move add-on enable/disable requests fully through Core ownership.
