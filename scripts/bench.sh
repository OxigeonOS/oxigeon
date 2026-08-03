#!/usr/bin/env bash
# Run the Oxigeon benchmarks.
#
# See scripts/bench.ps1 for why PATH gets a "." prepended: LuaJIT's build
# invokes host tools it just built by bare name. Harmless on platforms where
# that was never a problem.

set -euo pipefail
cd "$(dirname "$0")/.."

export PATH=".:$PATH"

echo "Building and running benchmarks (release; the first run compiles LuaJIT)..."
echo

cargo bench --bench dispatch "$@"

echo
echo "Full report: target/criterion/report/index.html"
echo "Compare against a baseline:"
echo "    scripts/bench.sh -- --save-baseline main"
echo "    scripts/bench.sh -- --baseline main"
