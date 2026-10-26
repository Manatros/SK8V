// Compile like menu_presets_tests.cpp. Arguments: <work dir> <UTF-16 INI to copy>.
// The text path must leave every section and key as the per-key profile API does.
#include "../src/menu.cpp"
#include <cassert>
#include <iostream>

namespace {
using menu::presets::Settings;

std::vector<std::wstring> Split(const wchar_t* list) {
    std::vector<std::wstring> out;
    for (const wchar_t* p = list; *p; p += wcslen(p) + 1) out.emplace_back(p);
    return out;
}

// Every section, in file order, with its key=value lines as the profile API reads them.
std::vector<std::pair<std::wstring, std::vector<std::wstring>>> Read(const std::wstring& path) {
    WritePrivateProfileStringW(nullptr, nullptr, nullptr, path.c_str());
    std::vector<wchar_t> names(1 << 16), body(1 << 20);
    GetPrivateProfileSectionNamesW(names.data(), static_cast<DWORD>(names.size()), path.c_str());
    std::vector<std::pair<std::wstring, std::vector<std::wstring>>> out;
    for (const auto& section : Split(names.data())) {
        GetPrivateProfileSectionW(section.c_str(), body.data(), static_cast<DWORD>(body.size()), path.c_str());
        out.push_back({section, Split(body.data())});
    }
    return out;
}

Settings Section(const std::wstring& path, const wchar_t* section) {
    std::vector<wchar_t> body(1 << 20);
    GetPrivateProfileSectionW(section, body.data(), static_cast<DWORD>(body.size()), path.c_str());
    Settings out;
    for (const auto& line : Split(body.data())) {
        const auto eq = line.find(L'=');
        if (line[0] == L';' || eq == std::wstring::npos) continue;
        const auto key = util::Narrow(line.substr(0, eq));
        if (key.rfind("__", 0) != 0) out.emplace(key, util::Narrow(line.substr(eq + 1)));
    }
    return out;
}

double SaveWith(bool text, const std::wstring& ini, const Settings* active, const Settings* user, const Settings* custom) {
    menu::g_ini = ini;
    const auto temp = ini + L".preset.tmp";
    LARGE_INTEGER f{}, a{}, b{};
    QueryPerformanceFrequency(&f), QueryPerformanceCounter(&a);
    WritePrivateProfileStringW(nullptr, nullptr, nullptr, ini.c_str());
    const bool staged = text ? menu::text::Stage(temp, active, user, custom) == menu::text::Staged::Done
                             : menu::StageProfile(temp, active, user, custom);
    assert(staged);
    assert(MoveFileExW(temp.c_str(), ini.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH));
    WritePrivateProfileStringW(nullptr, nullptr, nullptr, ini.c_str());
    QueryPerformanceCounter(&b);
    return 1000.0 * static_cast<double>(b.QuadPart - a.QuadPart) / static_cast<double>(f.QuadPart);
}

bool Bytes(const std::wstring& a, const std::wstring& b) {
    auto read = [](const std::wstring& p) {
        std::string s;
        const HANDLE h = CreateFileW(p.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr);
        LARGE_INTEGER n{};
        GetFileSizeEx(h, &n);
        s.resize(static_cast<std::size_t>(n.QuadPart));
        DWORD got = 0;
        ReadFile(h, s.data(), static_cast<DWORD>(s.size()), &got, nullptr);
        CloseHandle(h);
        return s;
    };
    return read(a) == read(b);
}

void Case(const std::wstring& dir, const std::wstring& source, const char* name, bool withUser, bool withCustom,
          bool newKey) {
    const auto a = dir + L"\\text.ini", b = dir + L"\\profile.ini";
    assert(CopyFileW(source.c_str(), a.c_str(), FALSE) && CopyFileW(source.c_str(), b.c_str(), FALSE));
    Settings active = Section(source, L"SkateV");
    assert(!active.empty());
    active.begin()->second = active.begin()->second == "1" ? "0" : "1";
    if (newKey) active["ZzNewKey"] = "7";
    Settings user = active, custom = active;
    custom.begin()->second = "custom";
    const double tText = SaveWith(true, a, &active, withUser ? &user : nullptr, withCustom ? &custom : nullptr);
    const double tProfile = SaveWith(false, b, &active, withUser ? &user : nullptr, withCustom ? &custom : nullptr);
    const bool same = Read(a) == Read(b);
    std::wcout << name << L": text " << tText << L" ms, profile API " << tProfile << L" ms, sections/keys "
               << (same ? L"equal" : L"DIFFERENT") << L", bytes " << (Bytes(a, b) ? L"identical" : L"differ") << L"\n";
    assert(same);
    // Unrelated content (comments, other sections) survives in the text path.
    assert(Read(a).size() == Read(source).size() + (withUser && Section(source, L"Preset.User").empty() ? 1 : 0) +
                                 (withCustom && Section(source, L"Preset.Custom").empty() ? 1 : 0));
}
} // namespace

int wmain(int argc, wchar_t** argv) {
    assert(argc == 3);
    const std::wstring dir = argv[1], source = argv[2];
    Case(dir, source, "user change", true, false, false);
    Case(dir, source, "custom save", false, true, false);
    Case(dir, source, "all sections + new key", true, true, true);
    Case(dir, source, "active only", false, false, false);

    // A non-UTF-16 INI is left to the profile API.
    const auto ansi = dir + L"\\ansi.ini";
    DeleteFileW(ansi.c_str());
    WritePrivateProfileStringW(L"SkateV", L"Hud", L"0", ansi.c_str());
    menu::g_ini = ansi;
    Settings one{{"Hud", "1"}};
    assert(menu::text::Stage(ansi + L".preset.tmp", &one, nullptr, nullptr) == menu::text::Staged::NotUtf16);

    // A game folder where existing files can be written but no new file can be
    // created (Manatros' GTA folder since 2026-10-23): Save still writes.
    const auto locked = dir + L"\\locked";
    _wsystem((L"icacls \"" + locked + L"\" /remove:d *S-1-1-0 >nul 2>&1").c_str());
    CreateDirectoryW(locked.c_str(), nullptr);
    const auto lockedIni = locked + L"\\SkateVLegacy.ini";
    assert(CopyFileW(source.c_str(), lockedIni.c_str(), FALSE));
    assert(_wsystem((L"icacls \"" + locked + L"\" /deny *S-1-1-0:(WD,AD) >nul").c_str()) == 0);
    const auto probe = locked + L"\\probe.tmp";
    const HANDLE h = CreateFileW(probe.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW, 0, nullptr);
    assert(h == INVALID_HANDLE_VALUE && "the folder must refuse new files for this case");
    menu::g_ini = lockedIni;
    menu::g_quiet = true;
    Settings hud{{"Hud", "7"}};
    const bool saved = menu::Save(&hud, nullptr, nullptr);
    wchar_t value[16]{};
    GetPrivateProfileStringW(L"SkateV", L"Hud", L"?", value, 16, lockedIni.c_str());
    _wsystem((L"icacls \"" + locked + L"\" /remove:d *S-1-1-0 >nul").c_str());
    assert(saved && std::wstring(value) == L"7");
    assert(GetFileAttributesW((lockedIni + L".preset.tmp").c_str()) == INVALID_FILE_ATTRIBUTES);
    std::wcout << L"menu save text tests passed\n";
    return 0;
}
