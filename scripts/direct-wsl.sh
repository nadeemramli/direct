#!/usr/bin/env bash
set -euo pipefail
repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cli="$repo/target/debug/direct.exe"
if [[ ! -f "$cli" ]]; then
  printf '%s\n' 'Build Direct on Windows first: scripts/start.ps1' >&2
  exit 1
fi
# Windows owns the service and data. Supply --data-dir as a Windows path when overriding it.
exec "$cli" "$@"
