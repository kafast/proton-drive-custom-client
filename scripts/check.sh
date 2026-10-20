#!/usr/bin/env bash
# Runs the same quality checks as the CI job "Format, Lint, Tests" locally.
#
# Usage: scripts/check.sh [--coverage]
#   --coverage  additionally measure test coverage (needs cargo-llvm-cov)
#
# Without Qt 6 installed, the tray app (gui-qt) is skipped.
set -euo pipefail
cd "$(dirname "$0")/.."

exclude=()
if ! command -v qmake6 >/dev/null 2>&1 && ! command -v qmake >/dev/null 2>&1; then
    echo "Qt 6 not found: skipping protonsync-gui-qt"
    exclude=(--exclude protonsync-gui-qt)
fi

step() { echo; echo "==> $*"; }

step "Format"
cargo fmt --all --check

step "Clippy"
cargo clippy --workspace "${exclude[@]}" --all-targets --locked -- -D warnings
cargo clippy -p protonsync-daemon --no-default-features --all-targets --locked -- -D warnings

step "Tests"
cargo test --workspace "${exclude[@]}" --locked

step "Documentation"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace "${exclude[@]}" --no-deps --locked

if [[ "${1:-}" == "--coverage" ]]; then
    step "Coverage"
    cargo llvm-cov --workspace --exclude protonsync-gui-qt --fail-under-lines 80 --summary-only
    cargo llvm-cov report --json --summary-only | python3 scripts/coverage_gate.py
fi

echo
echo "All checks passed."
