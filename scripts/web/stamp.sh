#!/bin/sh
set -eu
W=$1
V=$(cd "$W" && find client.js pkg js -type f ! -name '*.br' ! -name '*.gz' | LC_ALL=C sort \
  | xargs cat | sha256sum | cut -c1-12)
sed -i "s|src=\"./client.js\"|src=\"./client.js?v=$V\"|" "$W/index.html"
grep -q "src=\"./client.js?v=$V\"" "$W/index.html" \
  || { echo "stamp.sh: no ./client.js script in $W/index.html" >&2; exit 1; }
echo "$V"
