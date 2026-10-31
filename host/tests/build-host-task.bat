@echo off
setlocal
set PATH=C:\Program Files (x86)\Microsoft Visual Studio\Installer;%PATH%
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
rem Paths relative to this folder (host\tests of the checkout).
cd /d %~dp0
set SDK=%~dp0..\..\third_party\scripthookv
for %%n in (menu_pages_tests menu_presets_tests menu_save_text_tests prop_collision_tests marks_tests) do (
 cl /nologo /std:c++20 /EHsc /MD /W4 /DNOMINMAX /I..\src /I..\include /I%SDK%\inc %%n.cpp /Gy /O2 /Zc:inline /Fe:%%n.exe /link /OPT:REF user32.lib %SDK%\lib\ScriptHookV.lib >%%n.build.log 2>&1
 if errorlevel 1 exit /b 1
)
cl /nologo /std:c++20 /EHsc /MD /W4 /DNOMINMAX /I..\src /I..\include /I%SDK%\inc /c world_settings_tests.cpp /Gy /O2 >world_settings_tests.compile.log 2>&1
if errorlevel 1 exit /b 1
powershell -NoProfile -Command "Get-ChildItem ../../build/host/host/SkateVLegacy.dir/Release/*.obj | Where-Object Name -ne 'main.obj' | ForEach-Object { $_.FullName } | Set-Content world-settings-objects.rsp"
link /nologo world_settings_tests.obj @world-settings-objects.rsp /OUT:world_settings_tests.exe /OPT:REF /DELAYLOAD:ScriptHookV.dll delayimp.lib user32.lib %SDK%\lib\ScriptHookV.lib version.lib xinput.lib hid.lib setupapi.lib shell32.lib d3d11.lib dxgi.lib d3dcompiler.lib >world_settings_tests.link.log 2>&1
exit /b %errorlevel%

