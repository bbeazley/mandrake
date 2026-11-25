#!/bin/sh
set -eu
REG="${CARGO_HOME:-/usr/local/cargo}/registry/src"
LIST="${TMPDIR:-/tmp}/crates.$$.txt"
trap 'rm -f "$LIST"' EXIT
if [ "${1:-}" = client ]; then
  cat <<'HEAD'
THIRD-PARTY LICENSES: mandrake, the portable Linux build of the Myth II source port

Its own code (the mandrake-* crates) is MIT OR Apache-2.0: LICENSE-MIT and LICENSE-APACHE beside this
file. The binary also contains the Rust standard library (MIT OR Apache-2.0, The Rust Project
Developers) and the crates below, each under its own licence, whose texts follow.
No Bungie data and no plugins are in this archive.

HEAD
  cargo tree --locked -p mandrake-client -e normal --target x86_64-unknown-linux-gnu --prefix none --format '{p}|{l}' \
    | grep -v '(/' | sed -e 's/ (\*)$//' -e 's/ (proc-macro)//' | sort -u >"$LIST"
else
cat <<'HEAD'
THIRD-PARTY LICENSES: myth2-server image (mandrake-server, mandrake-ws-proxy, the web client's mandrake_web wasm)

The image's own code (the mandrake-* crates) is MIT OR Apache-2.0: LICENSE-MIT and LICENSE-APACHE beside
this file. The binaries and the wasm also contain the Rust standard library (MIT OR Apache-2.0, The Rust
Project Developers) and the crates below, each under its own licence, whose texts follow.
The Debian packages of the base image carry their copyright files in /usr/share/doc/<package>/.
No Bungie data and no plugins are in this image.

HEAD
{
  cargo tree --locked -p mandrake-server -e normal --prefix none --format '{p}|{l}'
  cargo tree --locked -p mandrake-web -e normal --target wasm32-unknown-unknown --prefix none --format '{p}|{l}'
} | grep -v '(/src/' | sed -e 's/ (\*)$//' -e 's/ (proc-macro)//' | sort -u >"$LIST"
fi
echo "Crates ($(wc -l <"$LIST")):"
sed 's/|/: /' "$LIST"
while IFS='|' read -r pkg lic; do
  name=${pkg% v*}; ver=${pkg##* v}
  dir=$(ls -d "$REG"/*/"$name-$ver" 2>/dev/null | head -1 || true)
  printf '\n\n==================== %s %s (%s) ====================\n' "$name" "$ver" "$lic"
  found=0
  if [ -n "$dir" ]; then
    for f in "$dir"/LICENSE* "$dir"/LICENCE* "$dir"/COPYING* "$dir"/NOTICE* "$dir"/UNLICENSE* "$dir"/license*; do
      [ -f "$f" ] || continue
      found=1
      printf '\n---- %s ----\n' "$(basename "$f")"
      cat "$f"
    done
  fi
  [ "$found" = 1 ] || printf '\n(no licence file in the published crate; its licence is %s, whose standard text applies)\n' "$lic"
done <"$LIST"
