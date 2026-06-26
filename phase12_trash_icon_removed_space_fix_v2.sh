#!/usr/bin/env bash
set -euo pipefail

if [[ ! -f Cargo.toml || ! -d locallink-core || ! -d locallink-ui ]]; then
  echo "Run this from the LocalLink repo root." >&2
  exit 1
fi

BRANCH="${1:-phase12-space-delete-leave-cleanup}"

git fetch origin
if git rev-parse --verify "$BRANCH" >/dev/null 2>&1; then
  git switch "$BRANCH"
else
  git switch -c "$BRANCH" "origin/$BRANCH"
fi

if command -v python3 >/dev/null 2>&1; then
  PYTHON_BIN="python3"
elif command -v py >/dev/null 2>&1; then
  PYTHON_BIN="py -3"
elif command -v python >/dev/null 2>&1; then
  PYTHON_BIN="python"
else
  echo "Python is required, but python3, py, and python were not found on PATH." >&2
  exit 1
fi

$PYTHON_BIN - <<'PY'
from pathlib import Path

# Keep kicked/removed foreign spaces registered until the user explicitly trashes them.
core_build = Path("locallink-core/build.rs")
text = core_build.read_text().replace("\r\n", "\n")
old = '''        if deletion_update || removed_by_owner {
            return Ok(self.purge_local_space(spaces, &update.space_id));
        }

        {
            let record = self.record_mut(&update.space_id)?;
            record.revision = update.revision;
            record.owner_enabled = update.owner_enabled;
            record.key_epoch = update.key_epoch;
        }

        let space = space_mut(spaces, &update.space_id)?;
        space.name = update.name;
        space.kind = update.kind;
        space.members = update.members;
        Ok(Some(space.clone()))
'''
new = '''        if deletion_update {
            return Ok(self.purge_local_space(spaces, &update.space_id));
        }

        {
            let record = self.record_mut(&update.space_id)?;
            record.revision = update.revision;
            record.owner_enabled = if removed_by_owner { false } else { update.owner_enabled };
            record.key_epoch = if removed_by_owner { 0 } else { update.key_epoch };
            if removed_by_owner {
                record.left = true;
                record.invite_state = None;
            }
        }

        let space = space_mut(spaces, &update.space_id)?;
        space.name = update.name;
        space.kind = update.kind;
        space.members = update.members;
        if removed_by_owner {
            space.active = false;
            space.addons.clear();
        }
        Ok(Some(space.clone()))
'''
if old not in text:
    raise SystemExit("Could not find old removed-space purge block in locallink-core/build.rs")
text = text.replace(old, new)
core_build.write_text(text)

# Remove explanatory action copy from Spaces and rely on the icon button.
ui_build = Path("locallink-ui/build.rs")
text = ui_build.read_text().replace("\r\n", "\n")
text = text.replace('''                        ui.label(
                            egui::RichText::new("Disconnect only affects local activity. Leave exits a foreign group.")
                                .color(color_muted())
                                .size(12.5),
                        );
''', "")
ui_build.write_text(text)

# Make the generated control icon-only and include registered removed/left spaces.
scroll_patch = Path("locallink-ui/build_scroll_patch.rs")
text = scroll_patch.read_text().replace("\r\n", "\n")
text = text.replace(
    '.add(danger_button(if space.role == \\"owner\\" { \\"Delete Space\\" } else { \\"Delete Local Copy\\" }))',
    '.add(danger_button(\\"🗑\\"))',
)
text = text.replace(
    '.add(danger_button(\\"Delete Space\\"))',
    '.add(danger_button(\\"🗑\\"))',
)
text = text.replace(
    'let can_delete_local_copy = space.local_state == \\"removed\\" || space.local_state == \\"left\\";\\n                        if (space.role == \\"owner\\" || can_delete_local_copy)',
    'let can_clear_registered_space = matches!(space.local_state.as_str(), \\"removed\\" | \\"left\\");\\n                        if (space.role == \\"owner\\" || can_clear_registered_space)',
)
text = text.replace(
    'Delete removes an owned space for everyone. Delete Local Copy only clears a removed foreign space from this device.',
    '',
)
text = text.replace(
    'Deleted/removed foreign spaces can be cleared locally.',
    '',
)
scroll_patch.write_text(text)
PY

cargo check -p locallink-core
cargo check -p locallink-ui

git add locallink-core/build.rs locallink-ui/build.rs locallink-ui/build_scroll_patch.rs
if git diff --cached --quiet; then
  echo "No changes to commit."
else
  git commit -m "Use trash icon for registered space cleanup"
fi

git push origin HEAD:"$BRANCH"
