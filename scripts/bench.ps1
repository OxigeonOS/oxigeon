# Run the Oxigeon benchmarks.
#
# Wraps `cargo bench` with one workaround. LuaJIT's MSVC build script invokes
# the host tools it has just built (`minilua`, `buildvm`) by bare name, relying
# on cmd.exe searching the current directory. If NoDefaultCurrentDirectoryInExePath
# is set in the environment — some tool launchers set it — that search is off and
# the build fails with:
#
#   'minilua' is not recognized as an internal or external command
#
# Prepending "." to PATH restores it. Harmless when the variable is not set.

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$env:PATH = ".;$env:PATH"

Write-Host "Building and running benchmarks (release; the first run compiles LuaJIT)..." -ForegroundColor Cyan
Write-Host ""

cargo bench --bench dispatch @args

if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "Benchmarks failed (exit $LASTEXITCODE)." -ForegroundColor Red
    exit $LASTEXITCODE
}

Write-Host ""
Write-Host "Full report: target/criterion/report/index.html" -ForegroundColor Green
Write-Host "Compare against a baseline:" -ForegroundColor Green
Write-Host "    scripts/bench.ps1 -- --save-baseline main" -ForegroundColor DarkGray
Write-Host "    scripts/bench.ps1 -- --baseline main" -ForegroundColor DarkGray
