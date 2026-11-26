param(
    [ValidateRange(1, 64)][int]$Jobs = 4,
    [switch]$ProfileAllocations,
    [switch]$CleanWebDependencies
)

$ErrorActionPreference = 'Stop'

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE."
    }
}

function Get-WebDependencyStamp {
    $node = (& node -p '[process.version, process.platform, process.arch].join("/")').Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not read the Node.js version' }
    $files = 'web/package.json', 'web/package-lock.json', 'web/node_modules/.package-lock.json'
    $hashes = Get-FileHash -LiteralPath $files -Algorithm SHA256
    return (@($node) + @($hashes.Hash)) -join '/'
}

function Get-PanelBuildStamp {
    param([string]$DependencyStamp)
    $files = @(
        Get-ChildItem -LiteralPath 'web' -File
        Get-ChildItem -LiteralPath 'web/src' -File -Recurse
        if (Test-Path -LiteralPath 'web/public') {
            Get-ChildItem -LiteralPath 'web/public' -File -Recurse
        }
        Get-ChildItem -LiteralPath 'web/dist' -File -Recurse |
            Where-Object Name -ne '.goodwin-build-stamp'
    ) | Sort-Object FullName
    $lines = Get-FileHash -LiteralPath $files.FullName -Algorithm SHA256 |
        ForEach-Object { "$($_.Hash) $($_.Path)" }
    return (@($DependencyStamp) + @($lines)) -join "`n"
}

$target = 'aarch64-unknown-linux-musl'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$previous = @{}
try {
    $webStamp = 'web/node_modules/.goodwin-build-stamp'
    $installWebDependencies = $CleanWebDependencies -or
        -not (Test-Path -LiteralPath $webStamp) -or
        -not (Test-Path -LiteralPath 'web/node_modules/.package-lock.json')
    if (-not $installWebDependencies) {
        $webDependencyStamp = Get-WebDependencyStamp
        $installWebDependencies = (Get-Content -LiteralPath $webStamp -Raw).Trim() -ne $webDependencyStamp
    }
    if ($installWebDependencies) {
        Invoke-Checked npm @('ci', '--prefix', 'web')
        $webDependencyStamp = Get-WebDependencyStamp
        $webDependencyStamp | Set-Content -LiteralPath $webStamp -Encoding ascii
    }
    $panelStamp = 'web/dist/.goodwin-build-stamp'
    # Vite rewrites the gzip inputs Cargo embeds, even when their bytes match.
    if ($installWebDependencies -or
        -not (Test-Path -LiteralPath $panelStamp) -or
        (Get-Content -LiteralPath $panelStamp -Raw).TrimEnd() -ne (Get-PanelBuildStamp $webDependencyStamp)) {
        Invoke-Checked npm @('run', 'build', '--prefix', 'web')
        Get-PanelBuildStamp $webDependencyStamp |
            Set-Content -LiteralPath $panelStamp -Encoding utf8
    }

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

    $sysroot = (& rustc --print sysroot).Trim()
    $hostTriple = (& rustc -vV | Select-String '^host: (.+)$').Matches[0].Groups[1].Value
    $linker = Join-Path $sysroot "lib/rustlib/$hostTriple/bin/rust-lld.exe"
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

    # Rust's LLD preserves the Cortex-A53 erratum workaround used by this target.
    Invoke-Checked python @('scripts/build-wake-tflite.py', '--target', 'aarch64-musl', '--jobs', "$Jobs")
    Invoke-Checked python @('scripts/build-wasm-plugins.py', '--jobs', "$Jobs")
    $previous['CARGO_ENCODED_RUSTFLAGS'] = [Environment]::GetEnvironmentVariable('CARGO_ENCODED_RUSTFLAGS', 'Process')
    # Cargo separates encoded flags with ASCII unit separators, preserving paths with spaces.
    $env:CARGO_ENCODED_RUSTFLAGS = (@('-C', "linker=$linker", '-C', 'linker-flavor=ld.lld') -join [char]31)
    $features = 'goodwin/wake-tflite,goodwin/wasm-plugins'
    if ($ProfileAllocations) {
        $features += ',goodwin/hotpath-alloc'
    }
    Invoke-Checked cargo @('build', '--locked', '--workspace', '--bins', '--release', '--target', $target,
        '--jobs', "$Jobs", '--features', $features)
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
