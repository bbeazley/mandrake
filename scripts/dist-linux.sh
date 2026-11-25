#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
glibc=2.31
triple=x86_64-unknown-linux-gnu
dist="$root/dist"
target_dir="$root/target/dist-linux"

die() { echo "dist-linux.sh: $*" >&2; exit 1; }

max_glibc() {
    objdump -T "$1" | grep -o 'GLIBC_[0-9][0-9.]*' | sort -u -t_ -k2,2V | tail -1
}

check() {
    local tarball
    tarball="$(ls -t "$dist"/mandrake-linux-x86_64-*.tar.gz 2>/dev/null | head -1)"
    [ -n "$tarball" ] || die "no tarball in $dist; run scripts/dist-linux.sh first"
    local data="${MYTH2_DATA_DIR:-}"
    if [ -z "$data" ]; then
        data="$(nix develop "$root" -c printenv MYTH2_DATA_DIR 2>/dev/null | tail -1)"
    fi
    [ -d "$data/tags" ] || die "MYTH2_DATA_DIR ($data) has no tags/; set it to the game data"
    command -v docker >/dev/null || die "--check needs docker"
    echo "check: $tarball in archlinux:latest, game data $data (read-only)"
    docker run --rm -i \
        -v "$tarball:/in/mandrake.tar.gz:ro" \
        -v "$data:/game/data:ro" \
        archlinux:latest bash -s <<'IN'
set -euo pipefail
pacman -Sy --noconfirm --needed binutils file alsa-lib vulkan-icd-loader vulkan-swrast >/dev/null 2>&1
cd /tmp && tar xzf /in/mandrake.tar.gz && cd mandrake
ln -s /game/data data
echo "--- contents"; ls -l
if [ -e /nix ]; then echo "FAIL: /nix exists in the container"; exit 1; fi
echo "--- ldd mandrake"; ldd ./mandrake
if ldd ./mandrake | grep -q '/nix/store\|not found'; then echo "FAIL: ldd"; exit 1; fi
interp="$(readelf -l ./mandrake | sed -n 's/.*interpreter: \(.*\)]/\1/p')"
echo "interpreter: $interp"
[ "$interp" = /lib64/ld-linux-x86-64.so.2 ] || { echo "FAIL: interpreter"; exit 1; }
if readelf -d ./mandrake | grep -qi 'rpath\|runpath'; then echo "FAIL: rpath"; exit 1; fi
max="$(objdump -T ./mandrake | grep -o 'GLIBC_[0-9][0-9.]*' | sort -u -t_ -k2,2V | tail -1)"
echo "max glibc symbol: $max"
[ "$(printf '%s\n' "$max" GLIBC_2.31 | sort -t_ -k2,2V | tail -1)" = GLIBC_2.31 ] \
    || { echo "FAIL: $max is newer than GLIBC_2.31"; exit 1; }
echo "--- ./mandrake.sh --help"; ./mandrake.sh --help | head -5
# The headless run draws one frame of Down a Broken Path (le3e) after 30 ticks on Mesa's lavapipe
# (the container has no GPU), which loads the tags, the sim and wgpu's dlopened Vulkan.
echo "--- headless: ./mandrake.sh --mesh le3e --headless --mute --ticks 30 --screenshot"
HOME=/tmp/home timeout 300 ./mandrake.sh --mesh le3e --headless --mute --width 640 --height 480 \
    --ticks 30 --screenshot /tmp/le3e.png >/tmp/run.log 2>&1 || { tail -30 /tmp/run.log; echo "FAIL: headless run"; exit 1; }
[ -s /tmp/le3e.png ] || { echo "FAIL: no screenshot"; exit 1; }
file /tmp/le3e.png
# A blank frame compresses to a few KB; the map's terrain and units to far more.
size="$(stat -c %s /tmp/le3e.png)"
echo "screenshot: $size bytes"
[ "$size" -gt 100000 ] || { echo "FAIL: the screenshot looks blank"; exit 1; }
echo "CHECK PASSED"
IN
}

build() {
    if ! command -v cargo-zigbuild >/dev/null; then
        exec nix develop "$root" -c "$0" "$@"
    fi
    local ver name stage
    ver="$(git describe --tags --always --dirty)"
    name="mandrake-linux-x86_64-$ver"
    echo "build: mandrake $ver for $triple, glibc $glibc"
    export ZIG_GLOBAL_CACHE_DIR="$target_dir/zig-cache"
    export ZIG_LOCAL_CACHE_DIR="$target_dir/zig-cache"
    unset RUSTC_WRAPPER
    unset NIX_LDFLAGS NIX_CFLAGS_COMPILE
    CARGO_TARGET_DIR="$target_dir" cargo-zigbuild rustc --locked --release -p mandrake-client \
        --bin mandrake --target "$triple.$glibc" -- -C link-arg=-Wl,--allow-shlib-undefined
    local bin="$target_dir/$triple/release/mandrake"
    [ -x "$bin" ] || die "no binary at $bin"

    stage="$target_dir/stage"
    rm -rf "$stage"
    mkdir -p "$stage/mandrake"
    local out="$stage/mandrake"
    cp "$bin" "$out/mandrake"
    strip "$out/mandrake"
    cp crates/mandrake-client/mandrake.default.toml crates/mandrake-client/mandrake.original.toml \
        crates/mandrake-client/mandrake.modernretro.toml "$out/"
    cp scripts/dist/mandrake.sh "$out/mandrake.sh"
    chmod 755 "$out/mandrake.sh"
    sed "s/@VERSION@/$ver/" scripts/dist/README.txt >"$out/README.txt"
    cp LICENSE-MIT LICENSE-APACHE "$out/"
    CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" sh docker/third-party-licenses.sh client \
        >"$out/THIRD-PARTY-LICENSES"

    if readelf -l "$out/mandrake" | grep -q /nix/store; then die "interpreter is in /nix/store"; fi
    if readelf -d "$out/mandrake" | grep -qi 'rpath\|runpath'; then die "binary has an rpath"; fi
    local max
    max="$(max_glibc "$out/mandrake")"
    [ "$(printf '%s\n' "$max" "GLIBC_$glibc" | sort -t_ -k2,2V | tail -1)" = "GLIBC_$glibc" ] \
        || die "$max is newer than GLIBC_$glibc"

    mkdir -p "$dist"
    tar -C "$stage" --owner=0 --group=0 -czf "$dist/$name.tar.gz" mandrake
    echo "max glibc symbol: $max"
    echo "needed: $(readelf -d "$out/mandrake" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p' | xargs)"
    ls -l "$dist/$name.tar.gz"
}

case "${1:-}" in
    --check) check ;;
    "") build ;;
    *) die "usage: scripts/dist-linux.sh [--check]" ;;
esac
