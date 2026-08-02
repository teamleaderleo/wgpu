#!/usr/bin/env python3
from pathlib import Path

SOURCE = Path("wgpu-core/src/device/queue.rs")
text = SOURCE.read_text(encoding="utf-8")


def section(start_marker: str, end_markers: tuple[str, ...]) -> str:
    start = text.index(start_marker)
    ends = [text.find(marker, start + len(start_marker)) for marker in end_markers]
    ends = [end for end in ends if end != -1]
    end = min(ends) if ends else len(text)
    return text[start:end]


allocate = section(
    "fn allocate_submission<'a>",
    ("\n    fn ", "\n    pub fn ", "\n    pub(crate) fn "),
)
compact = section(
    "pub fn compact_blas",
    ("\n    fn ", "\n    pub fn ", "\n    pub(crate) fn "),
)

allocate_command = allocate.index("command_indices.write()")
compact_pending = compact.index("pending_writes.lock()")
compact_command = compact.index("command_indices.write()")

if compact_pending >= compact_command:
    raise SystemExit(
        "compact_blas no longer acquires pending_writes before command_indices"
    )

rank_source = Path("wgpu-core/src/lock/rank.rs").read_text(encoding="utf-8")
command_rank = rank_source.index("DEVICE_COMMAND_INDICES")
pending_rank = rank_source.index("QUEUE_PENDING_WRITES")
if command_rank >= pending_rank:
    raise SystemExit("declared rank order is not command_indices before pending_writes")

print(f"source={SOURCE}")
print(f"allocate_submission command_indices offset={allocate_command}")
print(f"compact_blas pending_writes offset={compact_pending}")
print(f"compact_blas command_indices offset={compact_command}")
print("declared hierarchy: DEVICE_COMMAND_INDICES before QUEUE_PENDING_WRITES")
print("current compact_blas order: QUEUE_PENDING_WRITES before DEVICE_COMMAND_INDICES")
print("lock-order inversion reproduced from exact source")
