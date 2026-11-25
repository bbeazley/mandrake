#!/usr/bin/env bash
# Builds the image myth2-server:latest (or IMAGE) from the files the build needs, as git lists
# them in this worktree (tracked and untracked, not ignored), sent as a small tar context.
# usage: docker/build.sh [extra docker build arguments...]
set -euo pipefail
REPO="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO"
IMAGE=${IMAGE:-myth2-server:latest}
REV=$(git rev-parse --short HEAD)$(git diff --quiet HEAD -- Cargo.toml Cargo.lock crates tools docker scripts/web || echo -dirty)
git ls-files -z -co --exclude-standard -- Cargo.toml Cargo.lock LICENSE-MIT LICENSE-APACHE crates tools docker \
    scripts/web/client.html scripts/web/client.js scripts/web/stamp.sh \
  | tar -c --null -T - \
  | docker build -f docker/Dockerfile -t "$IMAGE" --build-arg "REVISION=$REV" \
      --build-arg "REVISION_DATE=$(git log -1 --format=%cs)" "$@" -
docker image ls "$IMAGE"
