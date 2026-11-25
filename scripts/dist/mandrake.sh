#!/bin/sh
here="$(cd "$(dirname "$0")" && pwd)"
conf="${XDG_CONFIG_HOME:-$HOME/.config}/myth2-sourceport"
mkdir -p "$conf" 2>/dev/null || true
for f in mandrake.original.toml mandrake.modernretro.toml; do
    [ -e "$conf/$f" ] || cp "$here/$f" "$conf/$f" 2>/dev/null || true
done
cd "$here" || exit 1
exec "$here/mandrake" "$@"
