# Optional Skate School & StartPark Tutorial Island installer/uninstaller for GTAxSkate.
# Keeps base GTAxSkate untouched unless opted in; cleanly removes only SK8V\tutorial and INI keys on Uninstall.
[CmdletBinding()]
param(
    [ValidateSet('Install', 'Verify', 'Uninstall')]
    [string]$Mode = 'Verify',
    [string]$SourceTutorialDir = '',
    [string]$SourceDlcRpf = '',
    [string]$GtaDir = ''
)

$ErrorActionPreference = 'Stop'

function Assert-GtaClosed {
    if (Get-Process GTA5, GTA5_Enhanced, FiveM, OpenIV, CodeWalker -ErrorAction SilentlyContinue) {
        throw 'Close GTA V, OpenIV, and CodeWalker first.'
    }
}

if ([string]::IsNullOrWhiteSpace($SourceTutorialDir)) {
    $SourceTutorialDir = Join-Path (Split-Path -Parent $PSScriptRoot) 'local\m0-tutorial'
}

if ($Mode -eq 'Verify') {
    $manifest = Join-Path $SourceTutorialDir 'm0-tutorial-manifest.json'
    if (-not (Test-Path -LiteralPath $manifest)) {
        throw "Optional tutorial manifest not found: $manifest"
    }
    python -B (Join-Path $PSScriptRoot 'prepare-tutorial.py') --verify-manifest $manifest
    if ($LASTEXITCODE) { throw 'Tutorial manifest verification failed.' }
    return
}

if ([string]::IsNullOrWhiteSpace($GtaDir) -or -not (Test-Path -LiteralPath (Join-Path $GtaDir 'GTA5.exe'))) {
    throw 'Pass -GtaDir <GTA V folder containing GTA5.exe>.'
}
Assert-GtaClosed

$iniPath = Join-Path $GtaDir 'SkateVLegacy.ini'
$cacheRoot = Join-Path $GtaDir 'SK8V'
if (Test-Path -LiteralPath $iniPath) {
    $wcLine = Get-Content -LiteralPath $iniPath | Where-Object { $_ -match '^WorldCache=' } | Select-Object -First 1
    if ($wcLine) {
        $wcVal = ($wcLine -replace '^WorldCache=', '').Trim().Trim('"')
        if ($wcVal) {
            if ($wcVal.EndsWith('.svwc', [StringComparison]::OrdinalIgnoreCase)) {
                $cacheRoot = Split-Path -Parent $wcVal
            } else {
                $cacheRoot = $wcVal
            }
        }
    }
}
$targetDir = Join-Path $cacheRoot 'addons\skate_school'
$dlcTarget = Join-Path $GtaDir 'update\x64\dlcpacks\skate_school\dlc.rpf'
if ([string]::IsNullOrWhiteSpace($SourceDlcRpf)) {
    $candidateDlc = Join-Path (Split-Path -Parent $PSScriptRoot) 'local\m1-tutorial-dlc\dlc.rpf'
    if (Test-Path -LiteralPath $candidateDlc) { $SourceDlcRpf = $candidateDlc }
}

if ($Mode -eq 'Install') {
    $manifest = Join-Path $SourceTutorialDir 'm0-tutorial-manifest.json'
    if (-not (Test-Path -LiteralPath $manifest)) {
        throw "Prepared tutorial pack not found at $SourceTutorialDir. Run prepare-tutorial.py first."
    }
    python -B (Join-Path $PSScriptRoot 'prepare-tutorial.py') --verify-manifest $manifest
    if ($LASTEXITCODE) { throw 'Refusing to install unverified tutorial pack.' }

    New-Item -ItemType Directory -Force -Path $targetDir | Out-Null
    Copy-Item -Path (Join-Path $SourceTutorialDir '*') -Destination $targetDir -Recurse -Force
    if (-not [string]::IsNullOrWhiteSpace($SourceDlcRpf) -and (Test-Path -LiteralPath $SourceDlcRpf)) {
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $dlcTarget) | Out-Null
        Copy-Item -LiteralPath $SourceDlcRpf -Destination $dlcTarget -Force
    }
    $receipt = [ordered]@{
        schema = 1
        addon = 'gtaxskate-tutorial-island'
        installedUtc = [DateTime]::UtcNow.ToString('o')
        targetSubdir = 'addons\skate_school'
    }
    [IO.File]::WriteAllText((Join-Path $targetDir 'addon-installed.json'), ($receipt | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
    if (Test-Path -LiteralPath $iniPath) {
        # Keys go into [SkateV]. Appending them at the end of the file put them into the last
        # section ([Preset.User]), whose __count then no longer matched, so the F10 menu refused
        # every change ("User snapshot invalid; settings unchanged", test 2026-11-16 evening).
        [string[]]$lines = @(Get-Content -LiteralPath $iniPath | Where-Object { $_ -notmatch '^(TutorialRoot|SkateTutorialPack)=' })
        $keys = @("TutorialRoot=$targetDir", 'SkateTutorialPack=1')
        $at = [Array]::IndexOf($lines, '[SkateV]')
        $lines = if ($at -ge 0) { @($lines[0..$at]) + $keys + @($lines | Select-Object -Skip ($at + 1)) } else { @('[SkateV]') + $keys + $lines }
        # Keep the file UTF-16 when it was: the host's fast menu save needs UTF-16 (menu.cpp text::Stage);
        # a UTF-8 rewrite made every F10 save fall back to the profile API (~1.8 s instead of ~14 ms).
        $head = [IO.File]::ReadAllBytes($iniPath) | Select-Object -First 2
        $enc = if ($head.Count -eq 2 -and $head[0] -eq 0xFF -and $head[1] -eq 0xFE) { [Text.UnicodeEncoding]::new($false, $true) } else { [Text.UTF8Encoding]::new($false) }
        [IO.File]::WriteAllLines($iniPath, $lines, $enc)
    }
    Write-Host "Installed optional Tutorial Island add-on cleanly into: $targetDir"
    return
}

if ($Mode -eq 'Uninstall') {
    if (Test-Path -LiteralPath $targetDir) {
        Remove-Item -LiteralPath $targetDir -Recurse -Force
    }
    if (Test-Path -LiteralPath $dlcTarget) {
        Remove-Item -LiteralPath (Split-Path -Parent $dlcTarget) -Recurse -Force
    }
    if (Test-Path -LiteralPath $iniPath) {
        [string[]]$lines = @(Get-Content -LiteralPath $iniPath | Where-Object { $_ -notmatch '^(TutorialRoot|SkateTutorialPack)=' })
        [IO.File]::WriteAllLines($iniPath, $lines)
    }
    Write-Host "Uninstalled optional Tutorial Island add-on cleanly from: $GtaDir"
}
