#!/usr/bin/env sh
# Validates every lighting prototype with naga-cli and translates it to SPIR-V, HLSL and MSL.
# Files named atmosphere_*.wgsl (and sky.wgsl, sky_ambient.wgsl) get atmosphere_common.wgsl prepended,
# the way the engine's loader will concatenate them.
#
#   cargo install naga-cli --version 30.0.1 --locked   # same naga as wgpu 30 in Task 3
#   sh docs/research/lighting-shaders/validate.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$(mktemp -d)
status=0
for f in "$here"/*.wgsl; do
    name=$(basename "$f" .wgsl)
    case "$name" in
        atmosphere_common) src="$f" ;;
        atmosphere_*|sky|sky_ambient)
            src="$out/$name.full.wgsl"
            cat "$here/atmosphere_common.wgsl" "$f" > "$src" ;;
        *) src="$f" ;;
    esac
    printf '%-28s' "$name"
    if naga "$src" >/dev/null 2>"$out/$name.err"; then
        printf 'valid'
        for ext in spv hlsl metal; do
            if naga "$src" "$out/$name.$ext" >/dev/null 2>>"$out/$name.err"; then
                printf ' %s' "$ext"
            else
                printf ' %s:FAILED' "$ext"
                status=1
            fi
        done
        echo
    else
        echo 'INVALID'
        cat "$out/$name.err"
        status=1
    fi
done
exit $status
