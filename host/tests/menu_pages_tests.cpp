// Compile like menu_presets_tests.cpp. Argument: a work directory.
// Sub-pages are rows on their parent (not categories), and every row has help.
#include "../src/menu.cpp"
#include <cassert>
#include <iostream>

int wmain(int argc, wchar_t** argv) {
    assert(argc == 2);
    const std::wstring ini = std::wstring(argv[1]) + L"\\pages.ini";
    DeleteFileW(ini.c_str());
    std::vector<menu::Page> pages = {
        {"Skating", {menu::Flag("Hold fakie", "HoldFakie", false)}},
        {"Advanced", {}},
        {"Skate physics", {menu::Number("Grip", "Grip", 1, 0.5f, 2, 0.05f, {}, {}, true)}, "Advanced"},
        {"Developer", {menu::Action("Run", [] {})}, "Advanced"},
    };
    pages[0].items[0].help = "Keep the fakie stance.";
    menu::Start(ini, pages, {}, nullptr);

    // Dynamic labels are evaluated again, and transient flags never enter presets.
    bool compare = false;
    auto status = menu::Action("World collision: best", [] {});
    status.labelText = [&] { return compare ? "World collision: COMPARE (plain GTA)" : "World collision: best"; };
    auto transient = menu::Flag("Compare", "", false, [&](const std::string& value) { compare = value == "1"; });
    transient.current = [&] { return compare ? "1" : "0"; };
    assert(menu::Label(status) == "World collision: best");
    const auto saved = menu::Current();
    menu::Commit(transient, menu::Step(transient, transient.current(), 1));
    assert(compare && menu::Label(status) == "World collision: COMPARE (plain GTA)");
    assert(menu::Current() == saved);
    menu::Commit(transient, menu::Step(transient, transient.current(), 1));
    assert(!compare && menu::Current() == saved);

    // Categories: only pages without a parent.
    assert(menu::g_top.size() == 2);
    assert(menu::g_pages[static_cast<std::size_t>(menu::g_top[0])].title == "Skating");
    assert(menu::g_pages[static_cast<std::size_t>(menu::g_top[1])].title == "Advanced");
    // Each sub-page is a link row on its parent, in order, with no INI key.
    const auto& advanced = menu::g_pages[1].items;
    assert(advanced.size() == 2);
    assert(advanced[0].kind == menu::Kind::Link && advanced[0].target == "Skate physics" && advanced[0].key.empty());
    assert(advanced[1].kind == menu::Kind::Link && advanced[1].target == "Developer");
    // Links never enter presets: the settings are exactly the keyed rows.
    const auto current = menu::Current();
    assert(current.size() == 2 && current.count("HoldFakie") && current.count("Grip"));

    // Help: the row's text, its default and how to change it.
    const std::string fakie = menu::ItemHelp(menu::g_pages[0].items[0]);
    assert(fakie.find("Keep the fakie stance.") == 0);
    assert(fakie.find("Default: Off") != std::string::npos && fakie.find("Left/Right") != std::string::npos);
    const std::string grip = menu::ItemHelp(menu::g_pages[2].items[0]);
    assert(grip.find("Default: 1") != std::string::npos && grip.find("Restart GTA") != std::string::npos);
    assert(menu::ItemHelp(advanced[1]) == menu::PageHint("Developer"));
    assert(menu::ItemHelp(menu::g_pages[3].items[0]) == "Enter to run.");

    // A User snapshot saved before a setting existed still allows changes
    // (2026-10-17: the new BoardLight row made every change "invalid").
    WritePrivateProfileStringW(L"Preset.User", nullptr, nullptr, ini.c_str());
    WritePrivateProfileStringW(L"Preset.User", L"HoldFakie", L"0", ini.c_str());
    WritePrivateProfileStringW(L"Preset.User", L"__schema", L"1", ini.c_str());
    WritePrivateProfileStringW(L"Preset.User", L"__count", L"1", ini.c_str());
    menu::Commit(menu::g_pages[2].items[0], "1.5");  // Grip: not in the old snapshot
    wchar_t gripValue[32]{};
    GetPrivateProfileStringW(L"SkateV", L"Grip", L"", gripValue, 32, ini.c_str());
    assert(std::wstring(gripValue) == L"1.5");
    GetPrivateProfileStringW(L"Preset.User", L"Grip", L"", gripValue, 32, ini.c_str());
    assert(std::wstring(gripValue) == L"1.5");
    // A snapshot whose own count does not match its lines stays invalid.
    WritePrivateProfileStringW(L"Preset.User", L"__count", L"5", ini.c_str());
    menu::Commit(menu::g_pages[0].items[0], "1");
    wchar_t fakieValue[32]{};
    GetPrivateProfileStringW(L"SkateV", L"HoldFakie", L"", fakieValue, 32, ini.c_str());
    assert(std::wstring(fakieValue) != L"1");

    // A second Start (reload) builds the links again from scratch.
    menu::Start(ini, pages, {}, nullptr);
    assert(menu::g_pages[1].items.size() == 2 && menu::g_top.size() == 2);
    std::wcout << L"menu pages tests passed\n";
    return 0;
}
