# Builds and runs the host test suites (host/tests/*_tests.cpp) without the game.
# Run tools/build-host.ps1 first: tests that include ../src/main.cpp link the
# host's other objects from build/host. Each suite gets its own scratch folder.
#   powershell -ExecutionPolicy Bypass -File tools\build-host-tests.ps1 [-Only world_settings_tests] [-NoRun]
param([string[]]$Only = @(), [switch]$NoRun)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual Studio C++ build tools not found' }
$vcvars = Join-Path $vs 'VC\Auxiliary\Build\vcvars64.bat'
$shv = Join-Path $Root 'third_party\scripthookv'
$objDir = Join-Path $Root 'build\host\host\SkateVLegacy.dir\Release'
$out = Join-Path $Root 'build\host-tests'
New-Item -ItemType Directory -Force $out | Out-Null
$hostObjs = @(Get-ChildItem $objDir -Filter *.obj -ErrorAction SilentlyContinue | Where-Object Name -ne 'main.obj' | ForEach-Object FullName)
$libs = "`"$shv\lib\ScriptHookV.lib`" version.lib xinput.lib hid.lib setupapi.lib shell32.lib user32.lib d3d11.lib dxgi.lib d3dcompiler.lib windowscodecs.lib ole32.lib advapi32.lib gdi32.lib"
# Arguments per suite: 'dir' = a fresh scratch folder.
$runArgs = @{ menu_pages_tests = @('dir'); menu_presets_tests = @('dir'); world_settings_tests = @('dir'); menu_save_text_tests = @('dir', 'ini') }
$suites = Get-ChildItem (Join-Path $Root 'host\tests') -Filter '*_tests.cpp' | Where-Object { -not $Only -or $Only -contains $_.BaseName }
$failed = @()
foreach ($s in $suites) {
    $name = $s.BaseName
    $exe = Join-Path $out "$name.exe"
    $src = Get-Content -Raw $s.FullName
    $link = if ($src -match '#include "../src/main.cpp"') { ($hostObjs | ForEach-Object { "`"$_`"" }) -join ' ' } else { '' }
    $cmd = "`"$vcvars`" >nul && cl /nologo /std:c++20 /EHsc /O2 /MD /utf-8 /DNOMINMAX /DWIN32_LEAN_AND_MEAN /Gy /I`"$shv\inc`" /I`"$Root\host\include`" /I`"$Root\host\src`" `"$($s.FullName)`" /Fo`"$out\\`" /Fe`"$exe`" /link /OPT:REF /DELAYLOAD:ScriptHookV.dll delayimp.lib $link $libs"
    $log = Join-Path $out "$name.build.log"
    $ErrorActionPreference = 'Continue'
    cmd /c $cmd *> $log
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path $exe)) { Write-Host "BUILD FAIL $name (see $log)"; $failed += $name; continue }
    if ($NoRun) { Write-Host "built $name"; continue }
    $scratch = Join-Path $out "$name.scratch"; Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue; New-Item -ItemType Directory $scratch | Out-Null
    $a = @(@($runArgs[$name]) | Where-Object { $_ } | ForEach-Object { if ($_ -eq 'dir') { $scratch } elseif ($_ -eq 'ini') { $i = Join-Path $scratch 'source.ini'; Set-Content $i "[SkateV]`r`nDifficulty=Easy`r`nStepRails=1`r`nDynamicWorld=1`r`n" -Encoding Unicode; $i } })
    $ErrorActionPreference = 'Continue'
    & $exe @a *> (Join-Path $out "$name.run.log")
    $ErrorActionPreference = 'Stop'
    if ($LASTEXITCODE -ne 0) { Write-Host "FAIL $name exit $LASTEXITCODE"; $failed += $name } else { Write-Host "PASS $name" }
}
if ($failed) { Write-Host "FAILED: $($failed -join ', ')"; exit 1 }
Write-Host "All host suites pass ($(@($suites).Count))."
