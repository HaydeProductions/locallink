# LocalLink Diagnostics Debugging

Use this when a UI button appears to work on one laptop but not another, especially for space add-ons.

## What gets logged

### Dev launch log

`git launch` / `scripts/build-run.sh` writes a terminal transcript to:

```text
%APPDATA%\LocalLink\logs\dev-launch-*.log
```

This captures the build, add-on packaging, AppData add-on installation, and UI start command.

### UI process log

`scripts/run.sh` starts the UI with stdout/stderr redirected to:

```text
%APPDATA%\LocalLink\logs\ui-process-*.log
```

The phase17 UI build patch emits job-level diagnostics here. When a button queues an API command, you should see lines like:

```text
[ui] queue ApiJob for diagnostics: SetSpaceAddonEnabled { ... }
[ui-api] request job=set_space_addon_enabled
[ui-api] response job=set_space_addon_enabled ok=true
```

If the API rejects the action, the response line should be:

```text
[ui-api] response job=set_space_addon_enabled ok=false error=...
```

### Core diagnostics log

Core writes add-on runtime diagnostics to:

```text
%APPDATA%\LocalLink\logs\diagnostics.log
```

This records:

- core startup
- loaded add-on and space counts
- space add-on runtime action plans
- add-on executable path checks
- add-on launch current directory
- add-on process spawn success
- add-on process spawn failure
- add-on process exits
- skipped starts caused by suppression after failure

### Space probe logs

`space-probe` still writes its own per-instance logs to:

```text
%LOCALAPPDATA%\LocalLink\logs\space-probe-*.log
```

or the AppData fallback if LocalAppData is unavailable.

## Easiest command

From the repo root:

```powershell
.\scripts\show-logs.ps1
```

To follow logs live while clicking buttons:

```powershell
.\scripts\show-logs.ps1 -Follow
```

## How to debug an Enable click

1. Start log following:

   ```powershell
   .\scripts\show-logs.ps1 -Follow
   ```

2. In the UI, go to the space and click `Enable` for `Space Probe`.

3. Look for the UI command:

   ```text
   [ui] queue ApiJob for diagnostics: SetSpaceAddonEnabled
   [ui-api] request job=set_space_addon_enabled
   [ui-api] response job=set_space_addon_enabled ok=true
   ```

4. Then look in `diagnostics.log` for core runtime behavior:

   ```text
   [addon-manager] runtime action plan start=1 ...
   [addon-manager] starting add-on instance=...
   [addon-launch] checking executable ...
   [addon-manager] started add-on instance=... pid=...
   ```

## Interpreting failures

### No UI job line appears

The button did not queue an API job. This is a UI/control wiring issue.

### UI job appears but response is `ok=false`

The UI reached core, but core rejected the command. The error string in the same line is the next boundary to inspect.

### UI response is `ok=true`, but no add-on action plan appears

Core saved the add-on setting, but runtime planning did not decide to start an add-on. Check whether the space is active/connected and whether the add-on exists in `%APPDATA%\LocalLink\addons`.

### Action plan appears, but executable check fails

The add-on is enabled and the runtime wants to launch it, but the executable path is wrong or missing.

Check:

```text
%APPDATA%\LocalLink\addons\<addon-id>\<executable>.exe
```

### Spawn fails

Core found the executable but Windows rejected process creation. The error text should be in `diagnostics.log`.

Common causes include blocked executable, missing runtime dependency, path issue, or permissions.

### Spawn succeeds but process exits immediately

Core launched the add-on, but the add-on crashed or exited. Check `diagnostics.log` for the exit line and the add-on's own log, such as `space-probe-*.log`.
