#!/bin/sh
# Before every commit: tests (including a short fuzz run) and clippy, zero warnings.
# Longer fuzzing: FDL_FUZZ_SEEDS=2000 scripts/check.sh
set -e
cd "$(dirname "$0")/.."
cargo test --release --workspace --locked
FDL_ROUNDTRIP_DIR="$PWD/shared-mock/library/docs" cargo test --release --locked -p fdl_core library_round_trips -- --ignored
python3 - <<'PY'
from pathlib import Path
source = Path("shared-mock/guides/assets")
bundled = Path("shared-mock/library/assets/guides")
for image in source.rglob("*"):
    if image.is_file():
        target = bundled / image.relative_to(source)
        assert target.read_bytes() == image.read_bytes(), f"Bundled guide asset is stale: {target}"
print("Bundled guide assets match their sources")
PY
cargo clippy --release --workspace --locked --all-targets -- -D warnings
