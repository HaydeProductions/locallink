# LocalLink add-ons

This folder contains add-ons that ship with LocalLink as source code.

Each add-on is a normal Cargo crate and owns its runtime manifest:

```text
addons/<addon-id>/
  Cargo.toml
  manifest.json
  src/
```

Build and launch scripts discover default add-ons by scanning `addons/*/manifest.json`. Scripts should not keep a separate hardcoded add-on list.

The manifest `id` controls the install folder under:

```text
%APPDATA%\LocalLink\addons\<id>
```

The manifest `executable` must match the binary produced by the crate in `target/release`.

Current add-ons:

- `clipboard-sync` - current clipboard sync add-on.
- `echo` - simple direct service echo/reference add-on.
- `space-probe` - debug add-on for validating space-scoped add-on launch and messaging.
