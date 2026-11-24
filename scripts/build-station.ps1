param(
    [ValidateRange(1, 64)][int]$Jobs = 4,
    [switch]$ProfileAllocations
)

$ErrorActionPreference = 'Stop'

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE."
    }
}

$target = 'aarch64-unknown-linux-musl'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$previous = @{}
try {
    Invoke-Checked npm @('ci', '--prefix', 'web')
    Invoke-Checked npm @('run', 'build', '--prefix', 'web')

    $toolsPath = (New-Item -ItemType Directory -Force 'target/goodwin-cross').FullName
    @'
import subprocess
import sys

tool = sys.argv[1]
args = [arg for arg in sys.argv[2:] if not arg.startswith("--target=")]
if tool == "cc":
    args[:0] = ["-target", "aarch64-linux-musl"]
sys.exit(subprocess.call(["zig", tool, *args]))
'@ | Set-Content -Encoding ascii (Join-Path $toolsPath 'zig-tool.py')
    @'
@echo off
python "%~dp0zig-tool.py" cc %*
'@ | Set-Content -Encoding ascii (Join-Path $toolsPath 'cc.cmd')
    @'
@echo off
python "%~dp0zig-tool.py" ar %*
'@ | Set-Content -Encoding ascii (Join-Path $toolsPath 'ar.cmd')

    $overrides = @{
        ZIG_GLOBAL_CACHE_DIR = (Join-Path $toolsPath 'zig-global')
        ZIG_LOCAL_CACHE_DIR = (Join-Path $toolsPath 'zig-local')
        CC_aarch64_unknown_linux_musl = (Join-Path $toolsPath 'cc.cmd')
        AR_aarch64_unknown_linux_musl = (Join-Path $toolsPath 'ar.cmd')
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER = (Join-Path $toolsPath 'cc.cmd')
    }
    foreach ($setting in $overrides.GetEnumerator()) {
        $previous[$setting.Key] = [Environment]::GetEnvironmentVariable($setting.Key, 'Process')
        [Environment]::SetEnvironmentVariable($setting.Key, $setting.Value, 'Process')
    }

    $sysroot = (& rustc --print sysroot).Trim()
    $hostTriple = (& rustc -vV | Select-String '^host: (.+)$').Matches[0].Groups[1].Value
    $linker = Join-Path $sysroot "lib/rustlib/$hostTriple/bin/rust-lld.exe"
    # Rust's LLD preserves the Cortex-A53 erratum workaround used by this target.
    Invoke-Checked python @('scripts/build-wake-tflite.py', '--target', 'aarch64-musl', '--jobs', "$Jobs")
    Invoke-Checked python @('scripts/build-wasm-plugins.py', '--jobs', "$Jobs")
    foreach ($package in @('goodwin', 'goodwin-timers', 'goodwin-openai-compatible', 'goodwin-wake-dataset')) {
        $features = @()
        if ($package -eq 'goodwin') {
            $goodwinFeatures = if ($ProfileAllocations) { 'wake-tflite,wasm-plugins,hotpath-alloc' } else { 'wake-tflite,wasm-plugins' }
            $features = @('--features', $goodwinFeatures)
        }
        Invoke-Checked cargo (@('rustc', '--locked', '--package', $package, '--bin', $package, '--release', '--target', $target, '--jobs', "$Jobs") +
            $features + @('--', '-C', "linker=$linker", '-C', 'linker-flavor=ld.lld'))
    }
    foreach ($plugin in @('timers', 'openai-compatible', 'wake-dataset')) {
        Invoke-Checked python @('scripts/package-plugin.py', "plugins/$plugin", "target/$target/release/goodwin-$plugin",
            'linux-aarch64', "target/plugins/$plugin-linux-aarch64.tar.gz")
    }
    Invoke-Checked python @('scripts/package-plugin.py', 'plugins/timers',
        'target/wasm32-unknown-unknown/release/goodwin-timers.wasm', 'wasm32',
        'target/plugins/timers-wasm32.tar.gz', '--wasm-state-file', 'timers.json')
    foreach ($plugin in @('openai-compatible', 'wake-dataset')) {
        Invoke-Checked python @('scripts/package-plugin.py', "plugins/$plugin",
            "target/wasm32-unknown-unknown/release/goodwin-$plugin.wasm", 'wasm32',
            "target/plugins/$plugin-wasm32.tar.gz")
    }
}
finally {
    foreach ($setting in $previous.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable($setting.Key, $setting.Value, 'Process')
    }
    Pop-Location
}
