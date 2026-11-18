#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

# Baseline reviewed 2026-11-18. Remove an ignore only after the upstream fix is adopted.
ignored=(
    RUSTSEC-2026-0543 # chrono: no fixed release; direct and transitive via gpui
    RUSTSEC-2024-0384 # instant: no fixed release; transitive via gpui-base
    RUSTSEC-2024-0436 # paste: build-time proc macro; no fixed release
    RUSTSEC-2024-0370 # proc-macro-error: build-only; no fixed release
    RUSTSEC-2026-0206 # rustybuzz: no fixed release; transitive via usvg
    RUSTSEC-2026-0192 # ttf-parser: no fixed release; transitive via cosmic-text/usvg
    RUSTSEC-2026-0371 # zbus_names: non-shipped target; no fixed release
    RUSTSEC-2026-0370 # zvariant: non-shipped target; no fixed release
    RUSTSEC-2026-0372 # zvariant_derive: non-shipped target; no fixed release
    RUSTSEC-2024-0429 # glib: non-shipped target; unsoundness fixed in 0.20
)

args=(--deny warnings)
if [[ -n "${CAPIN_AUDIT_DB:-}" ]]; then
    args+=(--db "$CAPIN_AUDIT_DB" --no-fetch)
fi
for id in "${ignored[@]}"; do
    args+=(--ignore "$id")
done

cargo audit "${args[@]}"
