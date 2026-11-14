# Optional Skate School & StartPark Tutorial Island installer/uninstaller for GTAxSkate.
# Keeps base GTAxSkate untouched unless opted in; cleanly removes only SK8V\tutorial and INI keys on Uninstall.
[CmdletBinding()]
param(
    [ValidateSet('Install', 'Verify', 'Uninstall')]
    [string]$Mode = 'Verify',
    [string]$SourceTutorialDir = '',
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

$targetDir = Join-Path $GtaDir 'SK8V\tutorial'
$iniPath = Join-Path $GtaDir 'SkateVLegacy.ini'

if ($Mode -eq 'Install') {
    $manifest = Join-Path $SourceTutorialDir 'm0-tutorial-manifest.json'
    if (-not (Test-Path -LiteralPath $manifest)) {
        throw "Prepared tutorial pack not found at $SourceTutorialDir. Run prepare-tutorial.py first."
    }
    python -B (Join-Path $PSScriptRoot 'prepare-tutorial.py') --verify-manifest $manifest
    if ($LASTEXITCODE) { throw 'Refusing to install unverified tutorial pack.' }

    New-Item -ItemType Directory -Force -Path $targetDir | Out-Null
    Copy-Item -Path (Join-Path $SourceTutorialDir '*') -Destination $targetDir -Recurse -Force
    $receipt = [ordered]@{
        schema = 1
        addon = 'gtaxskate-tutorial-island'
        installedUtc = [DateTime]::UtcNow.ToString('o')
        targetSubdir = 'SK8V\tutorial'
    }
    [IO.File]::WriteAllText((Join-Path $targetDir 'addon-installed.json'), ($receipt | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
    if (Test-Path -LiteralPath $iniPath) {
        [string[]]$lines = @(Get-Content -LiteralPath $iniPath | Where-Object { $_ -notmatch '^(TutorialRoot|SkateTutorialPack)=' })
        $lines += "TutorialRoot=$targetDir"
        $lines += 'SkateTutorialPack=1'
        [IO.File]::WriteAllLines($iniPath, $lines)
    }
    Write-Host "Installed optional Tutorial Island add-on cleanly into: $targetDir"
    return
}

if ($Mode -eq 'Uninstall') {
    if (Test-Path -LiteralPath $targetDir) {
        Remove-Item -LiteralPath $targetDir -Recurse -Force
    }
    if (Test-Path -LiteralPath $iniPath) {
        [string[]]$lines = @(Get-Content -LiteralPath $iniPath | Where-Object { $_ -notmatch '^(TutorialRoot|SkateTutorialPack)=' })
        [IO.File]::WriteAllLines($iniPath, $lines)
    }
    Write-Host "Uninstalled optional Tutorial Island add-on cleanly from: $GtaDir"
}
