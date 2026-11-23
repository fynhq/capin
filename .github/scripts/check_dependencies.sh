#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

# Baseline reviewed 2026-11-23. Remove an ignore only after the upstream fix is adopted.
ignored=(
    RUSTSEC-2026-0543 # chrono: no fixed release; transitive via gpui only
    RUSTSEC-2024-0384 # instant: no fixed release; transitive via gpui-base
    RUSTSEC-2024-0436 # paste: build-time proc macro; no fixed release
    RUSTSEC-2024-0370 # proc-macro-error: build-only; no fixed release
    RUSTSEC-2026-0206 # rustybuzz: no fixed release; transitive via usvg
    RUSTSEC-2026-0192 # ttf-parser: no fixed release; transitive via cosmic-text/usvg
    RUSTSEC-2026-0371 # zbus_names: non-shipped target; no fixed release
    RUSTSEC-2026-0370 # zvariant: non-shipped target; no fixed release
    RUSTSEC-2026-0372 # zvariant_derive: non-shipped target; no fixed release
    RUSTSEC-2024-0429 # glib: non-shipped target; unsoundness fixed in 0.20
    # heapless 0.8 via rstar/geo in the vendored OCR engine: rstar uses only
    # binary_heap and the 0.8 line has no fixed release.
    RUSTSEC-2026-0574 # heapless: rstar binary heap only; no 0.8 fix
    RUSTSEC-2026-0575 # heapless: rstar binary heap only; no 0.8 fix
    RUSTSEC-2026-0576 # heapless: rstar binary heap only; no 0.8 fix
    RUSTSEC-2026-0577 # heapless: rstar binary heap only; no 0.8 fix
    RUSTSEC-2026-0578 # heapless: rstar binary heap only; no 0.8 fix
    RUSTSEC-2026-0579 # heapless: rstar binary heap only; no 0.8 fix
    RUSTSEC-2026-0580 # heapless: rstar binary heap only; no 0.8 fix
    RUSTSEC-2026-0581 # heapless: rstar binary heap only; no 0.8 fix
    RUSTSEC-2026-0582 # heapless: rstar binary heap only; no 0.8 fix
)

args=(--deny warnings)
if [[ -n "${CAPIN_AUDIT_DB:-}" ]]; then
    args+=(--db "$CAPIN_AUDIT_DB" --no-fetch)
fi
for id in "${ignored[@]}"; do
    args+=(--ignore "$id")
done

cargo audit "${args[@]}"
