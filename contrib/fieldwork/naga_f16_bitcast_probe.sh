#!/usr/bin/env bash
set -euo pipefail

output_dir=${1:-fieldwork-artifacts/naga-f16-bitcast}
mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/naga-f16-bitcast.XXXXXX")

cleanup() {
    rm -rf -- "$work"
}
trap cleanup EXIT HUP INT TERM

cat >"$work/scalar-control.wgsl" <<'WGSL'
@compute @workgroup_size(1)
fn main() {
    let bits = bitcast<u32>(1.0);
    _ = bits;
}
WGSL

cat >"$work/vec-to-scalar.wgsl" <<'WGSL'
enable f16;

@compute @workgroup_size(1)
fn main() {
    let halves = vec2<f16>(f16(1.0), f16(2.0));
    let bits = bitcast<u32>(halves);
    _ = bits;
}
WGSL

cat >"$work/scalar-to-vec.wgsl" <<'WGSL'
enable f16;

@compute @workgroup_size(1)
fn main() {
    let halves = bitcast<vec2<f16>>(0x3c004000u);
    _ = halves;
}
WGSL

cargo build --locked -p naga-cli
naga="$PWD/target/debug/naga"
test -x "$naga"

run_case() {
    local name=$1 expected=$2
    local shader="$work/$name.wgsl"
    local stdout="$output_dir/$name.stdout"
    local stderr="$output_dir/$name.stderr"
    local status=0

    "$naga" "$shader" >"$stdout" 2>"$stderr" || status=$?
    printf '%s\t%d\t%s\t%s\n' \
        "$name" "$status" \
        "$(sha256sum "$stdout" | cut -d' ' -f1)" \
        "$(sha256sum "$stderr" | cut -d' ' -f1)" \
        >>"$output_dir/results.tsv"

    case "$expected" in
        success)
            test "$status" -eq 0
            ;;
        unable-to-cast)
            test "$status" -ne 0
            grep -Fq "Unable to cast" "$stderr"
            ;;
        *)
            echo "unknown expectation: $expected" >&2
            exit 2
            ;;
    esac
}

: >"$output_dir/results.tsv"
run_case scalar-control success
run_case vec-to-scalar unable-to-cast
run_case scalar-to-vec unable-to-cast

{
    echo "head=$(git rev-parse HEAD)"
    echo "naga_cli=$(git hash-object naga-cli/src/bin/naga.rs)"
    echo "validator=$(git hash-object naga/src/valid/expression.rs)"
    echo "ir=$(git hash-object naga/src/ir/mod.rs)"
    echo "rustc=$(rustc --version)"
    echo "cargo=$(cargo --version)"
    echo "naga_sha256=$(sha256sum "$naga" | cut -d' ' -f1)"
} >"$output_dir/identity.txt"

cp "$work"/*.wgsl "$output_dir/"
cat "$output_dir/identity.txt"
cat "$output_dir/results.tsv"
