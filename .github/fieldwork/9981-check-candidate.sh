#!/usr/bin/env bash
set -euo pipefail

repo_root=$(git rev-parse --show-toplevel)
patch_file="$repo_root/.github/fieldwork/9981-command-indices-before-pending-writes.patch"
source_file="$repo_root/wgpu-core/src/device/queue.rs"

cd "$repo_root"
git diff --quiet
git diff --cached --quiet

git apply --check "$patch_file"
git apply "$patch_file"
trap 'git checkout -- wgpu-core/src/device/queue.rs' EXIT

python3 - "$source_file" <<'PY'
from pathlib import Path
import sys

source = Path(sys.argv[1]).read_text(encoding="utf-8")
start = source.index("    pub(crate) fn compact_blas_inner(")
end = source.index("\n    }\n}\n\nimpl Global", start)
body = source[start:end]

command_indices = body.index("device.command_indices.write()")
pending_writes = body.index("self.pending_writes.lock()")
copy_command = body.index("copy_acceleration_structure_to_acceleration_structure")

assert command_indices < pending_writes, (
    "compact_blas_inner still acquires pending_writes before command_indices"
)
assert pending_writes < copy_command, "copy command must be encoded while pending_writes is held"
assert "drop(snatch_guard);" not in body, (
    "the outer snatch guard must remain alive until after the copied source handle is used"
)
print("candidate lock order: snatchable_lock -> command_indices -> pending_writes")
PY

cargo fmt --check -- wgpu-core/src/device/queue.rs
cargo check -p wgpu-core

printf '%s\n' 'wgpu #9981 candidate applies, formats, and checks'
