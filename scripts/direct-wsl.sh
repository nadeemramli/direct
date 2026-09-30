#!/usr/bin/env bash
set -euo pipefail
repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cli="$repo/target/debug/direct.exe"
if [[ ! -f "$cli" ]]; then
  common_dir="$(git -c "safe.directory=$repo" -C "$repo" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
  if [[ -n "$common_dir" ]]; then
    cli="$(dirname "$common_dir")/target/debug/direct.exe"
  fi
fi
if [[ ! -f "$cli" ]]; then
  printf '%s\n' 'Direct CLI not found in this checkout or its primary Git worktree. Build Direct on Windows first: scripts/start.ps1' >&2
  exit 1
fi
# Windows owns the service and data. Supply --data-dir as a Windows path when overriding it.
exec "$cli" "$@"
