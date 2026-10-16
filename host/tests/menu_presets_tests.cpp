// Compile with menu.cpp's SDK include path, /Gy, ScriptHookV.lib, user32.lib and /OPT:REF.
// No GTA native is invoked; unused UI sections are discarded by the linker.
#include "../src/menu.cpp"
#include <cassert>
#include <fstream>
#include <iostream>

int wmain(int argc, wchar_t** argv) {
    assert(argc == 2);
    const std::wstring ini = std::wstring(argv[1]) + L"\\presets.ini";
    DeleteFileW(ini.c_str());
    WritePrivateProfileStringW(L"SkateV", L"Hud", L"0", ini.c_str());
    WritePrivateProfileStringW(L"SkateV", L"Pop", L"1.25", ini.c_str());
    WritePrivateProfileStringW(L"Other", L"Personal", L"keep", ini.c_str());
    int callbacks = 0, duplicateCallbacks = 0;
    std::vector<menu::Page> pages = {{"Test", {
        menu::Flag("Hud", "Hud", true, [&](const auto&) { ++callbacks; }),
        menu::Number("Pop", "Pop", 1, 0.5f, 3, 0.05f, {}, {}, true),
        menu::Choice("Radio", "Radio", {"", "Self"}, {"Keep", "Self"}),
        menu::Choice("Camera", "Camera", {"High", "Low"}, {"High", "Low"}),
        menu::Chord("Chord", "Chord", 0x100, false),
        menu::Key("Personal key", [] { return "G"; }, [](int) { assert(false); })}},
        {"Duplicate", {menu::Flag("Hud again", "Hud", true, [&](const auto&) { ++duplicateCallbacks; })}}};
    menu::Start(ini, pages, {}, nullptr);
    // Startup is read-only, and cannot silently convert a user's current settings.
    wchar_t marker[32]{};
    GetPrivateProfileStringW(L"Preset.User", L"__schema", L"missing", marker, 32, ini.c_str());
    assert(std::wstring(marker) == L"missing");
    const auto original = menu::Current();
    assert(menu::presets::Valid(menu::Number("Minimum", "Minimum", 0.3f, 0.3f, 2, 0.1f, {}), "0.3"));
    assert(!menu::ApplyPreset("Unknown", {{"Missing", "1"}}));
    assert(menu::Current() == original && callbacks == 0);
    assert(!menu::ApplyPreset("Invalid", {{"Pop", "nan"}}));
    assert(!menu::ApplyPreset("Invalid", {{"Hud", "yes"}}));
    assert(!menu::ApplyPreset("Invalid", {{"Camera", "sideways"}}));
    assert(!menu::ApplyPreset("Invalid", {{"Chord", "0x400"}}));
    assert(menu::Current() == original);
    menu::SetPresetDefaults({{"Camera", "Low"}});
    assert(menu::ApplyPreset("RP", {{"Hud", "0"}, {"Pop", "0.8"}}));
    assert(menu::Current().at("Camera") == "Low");
    assert(menu::Current().at("Radio").empty());
    assert(menu::g_status.find("restart GTA") != std::string::npos);
    assert(callbacks == duplicateCallbacks && callbacks > 0);
    assert(menu::ApplyPreset("Developer", {{"Pop", "3"}}));
    assert(menu::Current().at("Hud") == "1"); // Resets settings left by RP.
    // A manual tweak updates User alone, without copying Developer's other values.
    const int beforeManual = callbacks;
    menu::Commit(menu::g_pages[0].items[1], "1.5");
    assert(callbacks == beforeManual); // Unrelated live callbacks must stay untouched.
    assert(menu::RestoreUserPreset());
    assert(menu::Current().at("Pop") == "1.5");
    assert(menu::Current().at("Hud") == "0");
    assert(menu::Current().at("Camera") == "High");
    assert(!menu::RestoreCustomPreset());
    assert(menu::IsOpen()); // Errors remain visible after action navigation closes the page.
    assert(menu::SaveCustomPreset());
    assert(menu::ApplyPreset("Developer", {{"Pop", "3"}}));
    assert(menu::RestoreCustomPreset());
    assert(menu::Current().at("Pop") == "1.5");
    // Snapshots survive a fresh menu instance without changing active values.
    menu::Start(ini, pages, {}, nullptr);
    assert(menu::RestoreUserPreset());
    assert(menu::Current().at("Pop") == "1.5");
    auto before = menu::Current();
    WritePrivateProfileStringW(L"Preset.Custom", L"Pop", L"infinity", ini.c_str());
    assert(!menu::RestoreCustomPreset());
    assert(menu::Current() == before);
    WritePrivateProfileStringW(L"Preset.User", L"Camera", nullptr, ini.c_str());
    assert(!menu::RestoreUserPreset());
    assert(menu::Current() == before);
    wchar_t personal[32]{};
    GetPrivateProfileStringW(L"Other", L"Personal", L"", personal, 32, ini.c_str());
    assert(std::wstring(personal) == L"keep");
    // A failed disk replacement never applies live callbacks or changes values.
    WritePrivateProfileStringW(L"Preset.User", L"Camera", L"High", ini.c_str());
    HANDLE locked = CreateFileW(ini.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr);
    assert(locked != INVALID_HANDLE_VALUE);
    const int count = callbacks;
    assert(!menu::ApplyPreset("Locked", {{"Hud", "1"}}));
    assert(callbacks == count && menu::Current() == before);
    CloseHandle(locked);
    const auto missingIni = std::wstring(argv[1]) + L"\\new-presets.ini";
    DeleteFileW(missingIni.c_str());
    menu::Start(missingIni, pages, {}, nullptr);
    assert(menu::ApplyPreset("First run", {}));
    assert(menu::Current().at("Hud") == "1");
    std::cout << "PASS: production preset validation, complete reset, User isolation, Custom persistence, duplicate callbacks, restart reporting and transactional failure\n";
}
