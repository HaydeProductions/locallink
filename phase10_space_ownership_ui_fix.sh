#!/usr/bin/env bash
set -euo pipefail

if [ ! -f "Cargo.toml" ] || [ ! -f "locallink-ui/build.rs" ]; then
  echo "Run this from the LocalLink repo root."
  exit 1
fi

python - <<'PY'
from pathlib import Path

path = Path("locallink-ui/build.rs")
s = path.read_text(encoding="utf-8")
original = s

def must_replace_once(text: str, old: str, new: str, label: str) -> str:
    if new in text:
        return text
    if old not in text:
        raise SystemExit(f"Could not find expected build.rs pattern for {label}")
    return text.replace(old, new, 1)

# 1) The phase10 Spaces UI expects role/state/permission metadata on SpaceRow.
old_space_row = (
    'struct SpaceRow {\\n'
    '    id: String,\\n'
    '    name: String,\\n'
    '    kind: String,\\n'
    '    active: bool,\\n'
    '    members: Vec<String>,\\n'
    '    addon_count: usize,\\n'
    '}'
)
new_space_row = (
    'struct SpaceRow {\\n'
    '    id: String,\\n'
    '    name: String,\\n'
    '    kind: String,\\n'
    '    active: bool,\\n'
    '    members: Vec<String>,\\n'
    '    addon_count: usize,\\n'
    '    role: String,\\n'
    '    owner_device_id: String,\\n'
    '    local_state: String,\\n'
    '    can_accept_invite: bool,\\n'
    '    can_decline_invite: bool,\\n'
    '    can_connect: bool,\\n'
    '    can_disconnect: bool,\\n'
    '    can_leave: bool,\\n'
    '    can_invite_members: bool,\\n'
    '    can_remove_members: bool,\\n'
    '    can_manage_addons: bool,\\n'
    '}'
)
s = must_replace_once(s, old_space_row, new_space_row, "SpaceRow fields")

old_apply_fields = (
    '                    members: string_array_field(row, \\"members\\"),\\n'
    '                    addon_count,\\n'
)
new_apply_fields = (
    '                    members: string_array_field(row, \\"members\\"),\\n'
    '                    addon_count,\\n'
    '                    role: str_field(row, \\"role\\"),\\n'
    '                    owner_device_id: str_field(row, \\"owner_device_id\\"),\\n'
    '                    local_state: str_field(row, \\"local_state\\"),\\n'
    '                    can_accept_invite: bool_field(row, \\"can_accept_invite\\"),\\n'
    '                    can_decline_invite: bool_field(row, \\"can_decline_invite\\"),\\n'
    '                    can_connect: bool_field(row, \\"can_connect\\"),\\n'
    '                    can_disconnect: bool_field(row, \\"can_disconnect\\"),\\n'
    '                    can_leave: bool_field(row, \\"can_leave\\"),\\n'
    '                    can_invite_members: bool_field(row, \\"can_invite_members\\"),\\n'
    '                    can_remove_members: bool_field(row, \\"can_remove_members\\"),\\n'
    '                    can_manage_addons: bool_field(row, \\"can_manage_addons\\"),\\n'
)
s = must_replace_once(s, old_apply_fields, new_apply_fields, "apply_spaces fields")

# 2) The UI rendering block sends explicit invite/leave jobs; add those ApiJob variants.
old_job_variants = (
    '    RemoveSpaceMember {\\n'
    '        space_id: String,\\n'
    '        peer_id: String,\\n'
    '    },\\n'
    '    PollEvents {'
)
new_job_variants = (
    '    RemoveSpaceMember {\\n'
    '        space_id: String,\\n'
    '        peer_id: String,\\n'
    '    },\\n'
    '    AcceptSpaceInvite {\\n'
    '        space_id: String,\\n'
    '    },\\n'
    '    DeclineSpaceInvite {\\n'
    '        space_id: String,\\n'
    '    },\\n'
    '    LeaveSpace {\\n'
    '        space_id: String,\\n'
    '    },\\n'
    '    PollEvents {'
)
s = must_replace_once(s, old_job_variants, new_job_variants, "ApiJob variants")

old_job_json = (
    '            ApiJob::RemoveSpaceMember { space_id, peer_id } => json!({\\n'
    '                \\"cmd\\": \\"remove_space_member\\",\\n'
    '                \\"space_id\\": space_id,\\n'
    '                \\"peer_id\\": peer_id\\n'
    '            }),' 
)
new_job_json = old_job_json + (
    '\\n            ApiJob::AcceptSpaceInvite { space_id } => json!({\\n'
    '                \\"cmd\\": \\"accept_space_invite\\",\\n'
    '                \\"space_id\\": space_id\\n'
    '            }),\\n'
    '            ApiJob::DeclineSpaceInvite { space_id } => json!({\\n'
    '                \\"cmd\\": \\"decline_space_invite\\",\\n'
    '                \\"space_id\\": space_id\\n'
    '            }),\\n'
    '            ApiJob::LeaveSpace { space_id } => json!({\\n'
    '                \\"cmd\\": \\"leave_space\\",\\n'
    '                \\"space_id\\": space_id\\n'
    '            }),' 
)
s = must_replace_once(s, old_job_json, new_job_json, "ApiJob JSON mapping")

old_job_names = '        ApiJob::RemoveSpaceMember { .. } => \\"remove_space_member\\",'
new_job_names = (
    '        ApiJob::RemoveSpaceMember { .. } => \\"remove_space_member\\",\\n'
    '        ApiJob::AcceptSpaceInvite { .. } => \\"accept_space_invite\\",\\n'
    '        ApiJob::DeclineSpaceInvite { .. } => \\"decline_space_invite\\",\\n'
    '        ApiJob::LeaveSpace { .. } => \\"leave_space\\",'
)
s = must_replace_once(s, old_job_names, new_job_names, "ApiJob name mapping")

# 3) Optional result logging for the explicit invite/leave commands.
old_result_block = (
    '            \\"remove_space_member\\" => {\\n'
    '                self.log(\\"Space member removed.\\");\\n'
    '                self.send_job(ApiJob::Spaces);\\n'
    '            }'
)
new_result_block = old_result_block + (
    '\\n            \\"accept_space_invite\\" => {\\n'
    '                self.log(\\"Space invite accepted.\\");\\n'
    '                self.send_job(ApiJob::Spaces);\\n'
    '                self.send_job(ApiJob::Addons);\\n'
    '            }\\n'
    '            \\"decline_space_invite\\" => {\\n'
    '                self.log(\\"Space invite declined.\\");\\n'
    '                self.send_job(ApiJob::Spaces);\\n'
    '            }\\n'
    '            \\"leave_space\\" => {\\n'
    '                self.log(\\"Left space.\\");\\n'
    '                self.send_job(ApiJob::Spaces);\\n'
    '                self.send_job(ApiJob::Addons);\\n'
    '            }'
)
s = must_replace_once(s, old_result_block, new_result_block, "result logging")

if s != original:
    path.write_text(s, encoding="utf-8")
    print("patched locallink-ui/build.rs")
else:
    print("locallink-ui/build.rs already contained the UI fix")
PY

cargo check -p locallink-core
cargo check -p locallink-ui

git add locallink-core/src/api.rs \
        locallink-core/src/space_membership.rs \
        locallink-core/src/space_sync_live.rs \
        locallink-ui/build.rs \
        locallink-ui/src/core_control_main.rs

if git diff --cached --quiet; then
  echo "No staged changes to commit."
else
  git commit -m "Separate owned and joined spaces"
fi

git push -u origin phase10-space-ownership-model
