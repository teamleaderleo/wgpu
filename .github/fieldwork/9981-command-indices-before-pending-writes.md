# wgpu #9981 command-index and pending-write lock order

Exact source base: `7a655581ff7d3fd9f38e4ede2bdd9c16bfcba899`.

The current `compact_blas_inner` path acquires `Queue::pending_writes` and later acquires `Device::command_indices`. `Queue::submit` acquires those locks in the opposite order, and `wgpu-core/src/lock/rank.rs` explicitly ranks `DEVICE_COMMAND_INDICES` before `QUEUE_PENDING_WRITES`.

The staged candidate keeps all fallible resource checks before either contested lock, then acquires locks in the declared order:

1. `Device::snatchable_lock`
2. `Device::command_indices`
3. `Queue::pending_writes`

The source BLAS raw handle remains protected by the snatch guard through command encoding. The pending-write guard is declared after the command-index guard, so normal reverse drop order releases `pending_writes` first.

Files:

- `9981-command-indices-before-pending-writes.patch`: minimal source candidate.
- `9981-check-candidate.sh`: verifies that the patch applies to the exact base, checks acquisition order, runs formatting, and checks `wgpu-core`.

Run from the repository root:

```console
bash .github/fieldwork/9981-check-candidate.sh
```

This is not ready for upstream submission. Required remaining evidence is a timeout-bounded concurrent `Queue::compact_blas`/`Queue::submit` target test or an equivalent ranked-lock execution test, followed by the repository's native `cargo clippy --tests`, `cargo xtask test`, and relevant backend gates.
