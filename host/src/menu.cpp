// In-game settings menu (see menu.h).
#include "menu.h"
#include "menu_presets.h"
#include <windows.h>
#include <main.h>
#include <map>
#include <string>
#include "host_util.h"
#include "natives.h"

namespace menu {
namespace {
using gta::Call;

// Frontend controls: arrows / d-pad, Enter / A, Backspace / B.
constexpr int kUp = 172, kDown = 173, kLeft = 174, kRight = 175, kAccept = 176, kCancel = 177;
constexpr int kVisibleRows = 14;
constexpr ULONGLONG kCaptureMs = 8000;

// GTA Interaction Menu look, a little wider and taller for long labels and values
// (Manatros 2026-10-19: more readable). Heading rows are shorter.
constexpr float kW = 0.27f, kPad = 0.007f, kBanner = 0.09f, kSub = 0.032f, kRow = 0.036f, kHeading = 0.03f, kChevron = 0.022f,
                kHelp = 0.095f;
constexpr float kRowText = 0.36f, kHelpText = 0.31f;

std::wstring g_ini;
std::vector<Page> g_pages;
std::function<unsigned()> g_pad;
LogFn g_log = nullptr;
bool g_open = false;
bool g_closing = false;         // closed by Toggle / the pause menu this frame
int g_level = 0;                // 0 categories, 1 a page's items
int g_sel[2] = {0, 0};          // selection per level
int g_pageIndex = 0;
std::vector<int> g_top;                     // the categories: pages without a parent
std::vector<std::pair<int, int>> g_back;    // sub-pages: (page, selection) Back returns to
std::map<std::string, std::string> g_cache;  // INI text per key, refreshed on open
std::vector<std::pair<std::string, std::string>> g_presetDefaults;
std::string g_status;
ULONGLONG g_statusUntil = 0;

// Chord capture: hold the buttons, release to set.
bool g_capturing = false, g_captureArmed = false;
unsigned g_captureMask = 0;
ULONGLONG g_captureStart = 0;

struct Repeat {
    ULONGLONG since = 0, next = 0;
} g_repeat[4];

using util::Wide;

std::string ReadIni(const Item& it) {
    const std::string v = util::IniString(g_ini, Wide(it.key));
    return v.empty() ? it.def : v;
}

void Refresh() {
    for (const Page& p : g_pages)
        for (const Item& it : p.items)
            if (!it.key.empty()) g_cache[it.key] = ReadIni(it);
}

bool g_quiet = false;  // FirstRunPreset: apply without opening the menu

void Say(const std::string& text) {
    if (g_quiet) return;
    // Menu actions release controls before invoking their callback. Preset/save
    // confirmations and failures need to restore the page so the result is visible.
    g_open = true;
    g_status = text;
    g_statusUntil = GetTickCount64() + 2500;
}

using presets::Settings;
Settings Current() {
    Settings out;
    for (const auto& page : g_pages) for (const auto& item : page.items)
        if (!item.key.empty()) out[item.key] = ReadIni(item);
    return out;
}

// Snapshots are complete, versioned sections; missing/old/corrupt snapshots never
// become a partial preset. Empty choice values (e.g. SkateRadio) are legitimate.
bool Snapshot(const wchar_t* section, Settings& out, bool& exists) {
    wchar_t marker[32]{};
    GetPrivateProfileStringW(section, L"__schema", L"", marker, 32, g_ini.c_str());
    exists = *marker != 0;
    if (!exists || std::wstring(marker) != L"1") return false;
    // A setting added after the snapshot was saved keeps its current value
    // (2026-10-17: the new BoardLight row made every change "invalid"). The
    // snapshot itself must still be complete: its keys match its own count.
    std::size_t present = 0;
    for (const auto& [key, current] : Current()) {
        wchar_t value[1024]{};
        GetPrivateProfileStringW(section, Wide(key).c_str(), L"\x1", value, 1024, g_ini.c_str());
        if (value[0] == 1 && value[1] == 0) { out[key] = current; continue; }
        out[key] = util::Narrow(value);
        ++present;
    }
    std::vector<wchar_t> lines(1 << 16);
    GetPrivateProfileSectionW(section, lines.data(), static_cast<DWORD>(lines.size()), g_ini.c_str());
    std::size_t stored = 0;
    for (const wchar_t* line = lines.data(); *line; line += wcslen(line) + 1)
        if (wcsncmp(line, L"__", 2) != 0 && wcschr(line, L'=')) ++stored;
    const auto count = GetPrivateProfileIntW(section, L"__count", -1, g_ini.c_str());
    std::string error;
    return present && count >= 0 && static_cast<std::size_t>(count) == stored && presets::Validate(g_pages, out, error);
}

bool User(Settings& out) {
    bool exists = false;
    if (Snapshot(L"Preset.User", out, exists)) return true;
    if (exists) { Say("User snapshot invalid; settings unchanged"); return false; }
    out = Current();
    std::string error;
    if (!presets::Validate(g_pages, out, error)) { Say(error); return false; }
    return true;
}

bool WriteSection(const std::wstring& path, const wchar_t* section, const Settings& values, bool snapshot) {
    if (snapshot && !WritePrivateProfileStringW(section, nullptr, nullptr, path.c_str())) return false;
    for (const auto& [key, value] : values)
        if (!WritePrivateProfileStringW(section, Wide(key).c_str(), Wide(value).c_str(), path.c_str())) return false;
    return !snapshot || (WritePrivateProfileStringW(section, L"__schema", L"1", path.c_str()) &&
        WritePrivateProfileStringW(section, L"__count", std::to_wstring(values.size()).c_str(), path.c_str()));
}

// The profile API rewrites the whole file for every key: one menu change wrote
// ~240 keys and held the script thread ~2 s on a 27 KB INI. A UTF-16 INI (what
// GTAxSkate writes) is edited as text with the same rules and staged in one write.
namespace text {
std::wstring_view Trim(std::wstring_view s) {
    const auto first = s.find_first_not_of(L" \t");
    if (first == std::wstring_view::npos) return {};
    return s.substr(first, s.find_last_not_of(L" \t") - first + 1);
}

bool Same(std::wstring_view a, std::wstring_view b) {
    return CompareStringOrdinal(a.data(), static_cast<int>(a.size()), b.data(), static_cast<int>(b.size()), TRUE) == CSTR_EQUAL;
}

bool Header(std::wstring_view line, std::wstring_view& name) {
    const auto t = Trim(line);
    const auto close = t.find(L']');
    if (t.empty() || t[0] != L'[' || close == std::wstring_view::npos) return false;
    name = Trim(t.substr(1, close - 1));
    return true;
}

bool KeyOf(std::wstring_view line, std::wstring_view& key) {
    const auto t = Trim(line);
    const auto eq = t.find(L'=');
    if (t.empty() || t[0] == L';' || t[0] == L'[' || eq == std::wstring_view::npos) return false;
    key = Trim(t.substr(0, eq));
    return true;
}

using Lines = std::vector<std::wstring>;

// [header, end) of the first section with this name; header == npos when absent.
std::pair<std::size_t, std::size_t> Find(const Lines& lines, std::wstring_view section) {
    std::size_t header = std::wstring::npos;
    for (std::size_t i = 0; i < lines.size(); ++i) {
        std::wstring_view name;
        if (!Header(lines[i], name)) continue;
        if (header != std::wstring::npos) return {header, i};
        if (Same(name, section)) header = i;
    }
    return {header, lines.size()};
}

// WritePrivateProfileString per key: replace the first matching line, add missing keys after the section's last line.
void Set(Lines& lines, const std::wstring& section, const Settings& values) {
    auto [header, end] = Find(lines, section);
    if (header == std::wstring::npos) {
        lines.push_back(L"[" + section + L"]");
        header = lines.size() - 1, end = lines.size();
    }
    Lines added;
    for (const auto& [k, v] : values) {
        const auto key = Wide(k);
        auto line = key + L"=" + Wide(v);
        bool found = false;
        for (std::size_t i = header + 1; i < end && !found; ++i) {
            std::wstring_view existing;
            if (KeyOf(lines[i], existing) && Same(existing, key)) lines[i] = std::move(line), found = true;
        }
        if (!found) added.push_back(std::move(line));
    }
    std::size_t at = end;
    while (at > header + 1 && Trim(lines[at - 1]).empty()) --at;
    lines.insert(lines.begin() + static_cast<std::ptrdiff_t>(at), added.begin(), added.end());
}

// Deleting the section, then writing every key, appends a fresh section at the end.
void Replace(Lines& lines, const std::wstring& section, const Settings& values) {
    const auto [header, end] = Find(lines, section);
    if (header != std::wstring::npos)
        lines.erase(lines.begin() + static_cast<std::ptrdiff_t>(header), lines.begin() + static_cast<std::ptrdiff_t>(end));
    Settings snapshot = values;
    Set(lines, section, snapshot);
    Set(lines, section, {{"__schema", "1"}});
    Set(lines, section, {{"__count", std::to_string(values.size())}});
}

enum class Staged { Done, Failed, NotUtf16 };

Staged Stage(const std::wstring& temp, const Settings* active, const Settings* user, const Settings* custom) {
    const HANDLE in = CreateFileW(g_ini.c_str(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr,
                                  OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (in == INVALID_HANDLE_VALUE) return GetLastError() == ERROR_FILE_NOT_FOUND ? Staged::NotUtf16 : Staged::Failed;
    LARGE_INTEGER size{};
    std::wstring all;
    bool read = GetFileSizeEx(in, &size) && size.QuadPart >= 2 && size.QuadPart < (16 << 20) && size.QuadPart % 2 == 0;
    if (read) {
        all.resize(static_cast<std::size_t>(size.QuadPart / 2));
        DWORD got = 0;
        read = ReadFile(in, all.data(), static_cast<DWORD>(size.QuadPart), &got, nullptr) && got == size.QuadPart;
    }
    CloseHandle(in);
    if (!read || all[0] != 0xFEFF) return Staged::NotUtf16;

    Lines lines;
    for (std::size_t pos = 1; pos < all.size();) {
        auto next = all.find(L'\n', pos);
        if (next == std::wstring::npos) next = all.size();
        std::wstring line = all.substr(pos, next - pos);
        if (!line.empty() && line.back() == L'\r') line.pop_back();
        lines.push_back(std::move(line));
        pos = next + 1;
    }
    if (active) Set(lines, L"SkateV", *active);
    if (user) Replace(lines, L"Preset.User", *user);
    if (custom) Replace(lines, L"Preset.Custom", *custom);

    std::wstring out(1, wchar_t{0xFEFF});
    for (const auto& line : lines) out += line, out += L"\r\n";
    const HANDLE file = CreateFileW(temp.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file == INVALID_HANDLE_VALUE) return Staged::Failed;
    DWORD wrote = 0;
    const DWORD bytes = static_cast<DWORD>(out.size() * sizeof(wchar_t));
    const bool ok = WriteFile(file, out.data(), bytes, &wrote, nullptr) && wrote == bytes;
    CloseHandle(file);
    return ok ? Staged::Done : Staged::Failed;
}
} // namespace text

// Older, non-UTF-16 INIs: the per-key profile API on the staged copy.
bool StageProfile(const std::wstring& temp, const Settings* active, const Settings* user, const Settings* custom) {
    if (!CopyFileW(g_ini.c_str(), temp.c_str(), FALSE)) {
        if (GetLastError() != ERROR_FILE_NOT_FOUND) return false;
        const HANDLE file = CreateFileW(temp.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (file == INVALID_HANDLE_VALUE) return false;
        CloseHandle(file);
    }
    const bool written = (!active || WriteSection(temp, L"SkateV", *active, false)) &&
        (!user || WriteSection(temp, L"Preset.User", *user, true)) &&
        (!custom || WriteSection(temp, L"Preset.Custom", *custom, true));
    // The profile-cache flush operation has no useful success return value.
    WritePrivateProfileStringW(nullptr, nullptr, nullptr, temp.c_str());
    const HANDLE file = CreateFileW(temp.c_str(), GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                                   nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    const bool flushed = file != INVALID_HANDLE_VALUE && FlushFileBuffers(file);
    if (file != INVALID_HANDLE_VALUE) CloseHandle(file);
    return written && flushed;
}

// Stage all sections beside the INI, flush, then replace it once. No callbacks or
// active values change unless every write succeeds. Unrelated INI sections survive.
// Stages the whole new INI at `temp` (a copy of the current one, rewritten).
bool StageAt(const std::wstring& temp, const Settings* active, const Settings* user, const Settings* custom) {
    auto staged = text::Stage(temp, active, user, custom);
    if (staged == text::Staged::NotUtf16)
        staged = StageProfile(temp, active, user, custom) ? text::Staged::Done : text::Staged::Failed;
    return staged == text::Staged::Done;
}

// Writes `from`'s bytes over the existing `to` (no new file in its folder).
bool OverwriteInPlace(const std::wstring& from, const std::wstring& to) {
    HANDLE in = CreateFileW(from.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr);
    if (in == INVALID_HANDLE_VALUE) return false;
    LARGE_INTEGER size{};
    std::string bytes;
    bool ok = GetFileSizeEx(in, &size) && size.QuadPart < (64ll << 20);
    if (ok) {
        bytes.resize(static_cast<std::size_t>(size.QuadPart));
        DWORD got = 0;
        ok = ReadFile(in, bytes.data(), static_cast<DWORD>(bytes.size()), &got, nullptr) && got == bytes.size();
    }
    CloseHandle(in);
    if (!ok) return false;
    HANDLE out = CreateFileW(to.c_str(), GENERIC_WRITE, 0, nullptr, OPEN_EXISTING, 0, nullptr);
    if (out == INVALID_HANDLE_VALUE) return false;
    DWORD put = 0;
    ok = WriteFile(out, bytes.data(), static_cast<DWORD>(bytes.size()), &put, nullptr) && put == bytes.size() &&
         SetEndOfFile(out) && FlushFileBuffers(out);
    CloseHandle(out);
    return ok;
}

// Settings are staged next to the INI and swapped in, so a failed write never
// leaves half a file. A game folder where new files cannot be created (only
// existing ones written; Manatros' since 2026-10-23, and installs under
// Program Files) stages under %LOCALAPPDATA%\SkateV and writes the result
// over the existing INI instead (2026-10-24: every change said "Could not
// save settings" for two days).
bool Save(const Settings* active, const Settings* user, const Settings* custom) {
    WritePrivateProfileStringW(nullptr, nullptr, nullptr, g_ini.c_str());
    const auto temp = g_ini + L".preset.tmp";
    bool saved = StageAt(temp, active, user, custom) && MoveFileExW(temp.c_str(), g_ini.c_str(), MOVEFILE_REPLACE_EXISTING);
    if (!saved) {
        DeleteFileW(temp.c_str());
        wchar_t base[MAX_PATH]{};
        const DWORD n = GetEnvironmentVariableW(L"LOCALAPPDATA", base, MAX_PATH);
        if (n > 0 && n < MAX_PATH) {
            const std::wstring dir = std::wstring(base) + L"\\SkateV";
            CreateDirectoryW(dir.c_str(), nullptr);
            const std::wstring local = dir + L"\\SkateVLegacy.ini.preset.tmp";
            // Stage works on a copy of the current INI.
            saved = CopyFileW(g_ini.c_str(), local.c_str(), FALSE) && StageAt(local, active, user, custom) &&
                    OverwriteInPlace(local, g_ini);
            DeleteFileW(local.c_str());
            if (saved && g_log) g_log("SkateV menu: game folder not writable for new files; settings written in place");
        }
    }
    if (!saved) {
        if (g_log) g_log(("SkateV menu: could not save settings (error " + std::to_string(GetLastError()) + ")").c_str());
        Say("Could not save settings; unchanged");
        return false;
    }
    WritePrivateProfileStringW(nullptr, nullptr, nullptr, g_ini.c_str());
    return true;
}

bool Activate(const std::string& name, const Settings& next, const Settings* user, bool changedOnly = false) {
    std::string error;
    if (!presets::Validate(g_pages, next, error)) { Say(error); return false; }
    const auto previous = Current();
    if (!Save(&next, user, nullptr)) return false;
    bool restart = false;
    g_cache = next;
    for (auto& page : g_pages) for (auto& item : page.items) {
        if (item.key.empty()) continue;
        const auto& value = next.at(item.key);
        if (item.restart && previous.at(item.key) != value) restart = true;
        if (item.apply && (!changedOnly || previous.at(item.key) != value)) item.apply(value);
    }
    Say(name + (restart ? " saved - restart GTA required" : " applied"));
    if (g_log) g_log(("SkateV preset: " + name + (restart ? " (restart required)" : "")).c_str());
    return true;
}

void Commit(Item& it, const std::string& value) {
    if (it.key.empty()) { if (it.apply) it.apply(value); return; }
    Settings user;
    if (!User(user)) return;
    user[it.key] = value;
    auto next = Current();
    next[it.key] = value;
    Activate("User change", next, &user, true);
}

void Beep(const char* name) { Call<void>(gta::PLAY_SOUND_FRONTEND, -1, name, "HUD_FRONTEND_DEFAULT_SOUNDSET", 1); }

bool Fire(int slot, int control) {
    Repeat& r = g_repeat[slot];
    const ULONGLONG now = GetTickCount64();
    if (Call<BOOL>(gta::IS_DISABLED_CONTROL_JUST_PRESSED, 0, control)) {
        r.since = now, r.next = now + 350;
        return true;
    }
    if (!Call<BOOL>(gta::IS_DISABLED_CONTROL_PRESSED, 0, control)) {
        r.since = 0;
        return false;
    }
    if (r.since && now >= r.next) {
        r.next = now + 70;
        return true;
    }
    return false;
}

using gta::Align;

// Right: text ends at `edge`. Wrap: text starts at x and wraps at `edge`.
void Text(const std::string& s, float x, float y, float scale, int shade, Align align = Align::Left, float edge = 0.0f) {
    gta::Text(s.c_str(), x, y, scale, shade, shade, shade, 255, align, edge, false);
}

void Rect(float x, float y, float w, float h, int r, int g, int b, int a) {
    Call<void>(gta::DRAW_RECT, x + w / 2, y + h / 2, w, h, r, g, b, a);
}

// Small pixel-stepped triangle (1080p pixels), the chevron in the footer strip.
void Chevron(float cx, float cy, bool up) {
    for (int i = 0; i < 5; ++i) {
        const float w = (2.0f + 2.0f * i) / 1920.0f, h = 2.0f / 1080.0f;
        Call<void>(gta::DRAW_RECT, cx, cy + (up ? i - 2 : 2 - i) * h, w, h, 200, 200, 200, 255);
    }
}

void Row(float x, float y, const std::string& label, const std::string& value, bool selected) {
    if (selected) Rect(x, y, kW, kRow, 240, 240, 240, 235);
    else Rect(x, y, kW, kRow, 0, 0, 0, 175);
    const int shade = selected ? 10 : 255;
    Text(label, x + kPad, y + 0.003f, kRowText, shade);
    if (!value.empty()) Text(value, 0.0f, y + 0.003f, kRowText, selected ? 10 : 200, Align::Right, x + kW - kPad);
}

// A section title: darker strip, orange capitals, a thin rule.
void HeadingRow(float x, float y, const std::string& label) {
    Rect(x, y, kW, kHeading, 0, 0, 0, 215);
    Rect(x, y + kHeading - 0.002f, kW, 0.002f, 255, 110, 0, 200);
    gta::Text(Upper(label).c_str(), x + kPad, y + 0.004f, 0.28f, 255, 150, 60, 255, Align::Left, 0.0f, false);
}

bool Shown(const Item& it) { return !it.visible || it.visible(); }
bool PageShown(const Page& p) { return !p.visible || p.visible(); }
bool Selectable(const Item& it) { return it.kind != Kind::Heading && Shown(it); }

// The rows drawn on a page: shown items; a heading only with a shown row
// after it before the next heading.
std::vector<int> Rows(const std::vector<Item>& items) {
    std::vector<int> rows;
    for (std::size_t i = 0; i < items.size(); ++i) {
        if (!Shown(items[i])) continue;
        if (items[i].kind == Kind::Heading) {
            bool any = false;
            for (std::size_t j = i + 1; j < items.size() && items[j].kind != Kind::Heading; ++j) any = any || Shown(items[j]);
            if (!any) continue;
        }
        rows.push_back(static_cast<int>(i));
    }
    return rows;
}

// The categories shown on the first level (indices into g_pages).
std::vector<int> Tops();

// The next selectable row from `from` in direction `dir` (wrapping); `from` itself when none.
int NextSelectable(const std::vector<Item>& items, int from, int dir) {
    const int n = static_cast<int>(items.size());
    for (int step = 1; step <= n; ++step) {
        const int i = ((from + dir * step) % n + n) % n;
        if (Selectable(items[static_cast<std::size_t>(i)])) return i;
    }
    return from;
}

std::vector<int> Tops() {
    std::vector<int> tops;
    for (int i : g_top)
        if (PageShown(g_pages[static_cast<std::size_t>(i)])) tops.push_back(i);
    return tops;
}

int FirstSelectable(const std::vector<Item>& items) {
    for (std::size_t i = 0; i < items.size(); ++i)
        if (Selectable(items[i])) return static_cast<int>(i);
    return 0;
}

void Panel(const std::string& banner, const std::string& title, const std::string& counter,
           const std::vector<PanelRow>& rows, const std::string& help) {
    const float margin = (1.0f - Call<float>(gta::GET_SAFE_ZONE_SIZE)) / 2;
    const float x = margin + 0.02f;
    float y = margin + 0.04f;
    Rect(x, y, kW, kBanner, 12, 12, 12, 235);
    Rect(x, y + kBanner - 0.004f, kW, 0.004f, 255, 110, 0, 255);
    Text(banner, x + kPad, y + 0.008f, 0.8f, 255);
    y += kBanner;
    Rect(x, y, kW, kSub, 255, 110, 0, 235);
    Text(Upper(title), x + kPad, y + 0.0035f, 0.3f, 255);
    Text(counter, 0.0f, y + 0.0035f, 0.3f, 255, Align::Right, x + kW - kPad);
    y += kSub;
    for (const PanelRow& r : rows) {
        Row(x, y, r.label, r.value, r.selected);
        y += kRow;
    }
    Rect(x, y, kW, kChevron, 0, 0, 0, 160);
    Chevron(x + kW / 2 - 0.008f, y + kChevron / 2, true);
    Chevron(x + kW / 2 + 0.008f, y + kChevron / 2, false);
    y += kChevron;
    Rect(x, y, kW, kHelp, 0, 0, 0, 175);
    Text(help, x + kPad, y + 0.005f, kHelpText, 220, Align::Wrap, x + kW - kPad);
}

// The box under the rows: what the selected row does, its default, how to change it.
std::string ItemHelp(const Item& it) {
    if (it.kind == Kind::Link) return it.help.empty() ? PageHint(it.target) : it.help;
    std::string text = it.help;
    if (it.kind == Kind::Flag || it.kind == Kind::Number || it.kind == Kind::Choice) {
        text += (text.empty() ? "" : "  ") + std::string("Default: ") + Show(it, it.def) + ".";
        text += it.restart ? " Restart GTA to apply." : " Left/Right change.";
    } else if (it.kind == Kind::Chord || it.kind == Kind::Key) {
        text += (text.empty() ? "" : "  ") + std::string("Enter, then press the new button.");
    } else if (text.empty()) {
        text = "Enter to run.";
    }
    return text;
}

void Draw() {
    // Left-aligned inside GTA's safe zone, like the Interaction Menu.
    const float margin = (1.0f - Call<float>(gta::GET_SAFE_ZONE_SIZE)) / 2;
    const float x = margin + 0.02f;
    float y = margin + 0.04f;

    // Banner: black block, big title, orange rule under it.
    Rect(x, y, kW, kBanner, 12, 12, 12, 235);
    Rect(x, y + kBanner - 0.004f, kW, 0.004f, 255, 110, 0, 255);
    Text("GTAxSkate", x + kPad, y + 0.008f, 0.8f, 255);
    y += kBanner;

    // Subtitle bar: page title left, "n / total" right.
    const std::vector<int> tops = Tops();
    const std::vector<int> rows = g_level ? Rows(g_pages[g_pageIndex].items) : std::vector<int>{};
    const int count = g_level ? static_cast<int>(rows.size()) : static_cast<int>(tops.size());
    const int sel = g_sel[g_level];
    // Position of the selection among the drawn rows.
    const int at = g_level ? static_cast<int>(std::find(rows.begin(), rows.end(), sel) - rows.begin()) : sel;
    // The counter counts settings, not headings.
    int number = at, total = count;
    if (g_level) {
        const auto& items = g_pages[g_pageIndex].items;
        total = static_cast<int>(std::count_if(items.begin(), items.end(), Selectable));
        number = static_cast<int>(std::count_if(items.begin(), items.begin() + sel, Selectable));
    }
    Rect(x, y, kW, kSub, 255, 110, 0, 235);
    Text(Upper(g_level ? g_pages[g_pageIndex].title : "GTAxSkate Settings"), x + kPad, y + 0.004f, 0.32f, 255);
    Text(Counter(number, total), 0.0f, y + 0.004f, 0.32f, 255, Align::Right, x + kW - kPad);
    y += kSub;

    const int first = std::clamp(at - kVisibleRows / 2, 0, std::max(0, count - kVisibleRows));
    const int shown = std::min(kVisibleRows, count - first);
    for (int r = first; r < first + shown; ++r) {
        if (!g_level) {
            Row(x, y, g_pages[static_cast<std::size_t>(tops[static_cast<std::size_t>(r)])].title, ">", r == sel);
            y += kRow;
            continue;
        }
        const int i = rows[static_cast<std::size_t>(r)];
        const Item& it = g_pages[g_pageIndex].items[static_cast<std::size_t>(i)];
        if (it.kind == Kind::Heading) {
            HeadingRow(x, y, it.label);
            y += kHeading;
            continue;
        }
        std::string value = it.kind == Kind::Link ? ">"
                            : it.kind == Kind::Action ? ""
                            : it.kind == Kind::Key  ? (g_capturing && i == sel ? std::string("press a key (Esc: keep)") : it.show ? it.show() : "")
                            : i == sel              ? ShowSelected(it, (it.current ? it.current() : g_cache[it.key]))
                                                    : Show(it, (it.current ? it.current() : g_cache[it.key]));
        if (it.kind == Kind::Chord && g_capturing && i == sel) value = ChordName(g_captureMask) + " ...";
        Row(x, y, Label(it) + (it.restart ? " *" : ""), value, i == sel);
        y += kRow;
    }

    // Chevron strip, then the description box.
    Rect(x, y, kW, kChevron, 0, 0, 0, 160);
    Chevron(x + kW / 2 - 0.008f, y + kChevron / 2, true);
    Chevron(x + kW / 2 + 0.008f, y + kChevron / 2, false);
    y += kChevron;
    Rect(x, y, kW, kHelp, 0, 0, 0, 175);
    std::string text;
    int shade = 210;
    if (g_capturing) {
        text = g_pages[g_pageIndex].items[static_cast<std::size_t>(sel)].allowOff
                   ? "Hold the buttons, release to set. Backspace = off, Esc = cancel"
                   : "Hold the buttons, release to set. Esc = cancel";
    } else if (GetTickCount64() < g_statusUntil) {
        text = g_status, shade = 255;
    } else {
        text = g_level ? ItemHelp(g_pages[g_pageIndex].items[static_cast<std::size_t>(sel)])
                       : PageHint(g_pages[static_cast<std::size_t>(Tops()[static_cast<std::size_t>(sel)])].title);
    }
    Text(text, x + kPad, y + 0.005f, kHelpText, shade, Align::Wrap, x + kW - kPad);
}

void Capture(Item& it) {
    if (it.kind == Kind::Key) {
        // The next key that goes down (after the Enter that started this is up).
        bool any = false;
        int pressed = 0;
        for (int vk = 0x08; vk <= 0xFE && !pressed; ++vk) {
            if (vk == VK_LBUTTON || vk == VK_RBUTTON || vk == VK_MBUTTON) continue;
            if (GetAsyncKeyState(vk) & 0x8000) {
                any = true;
                if (g_captureArmed && vk != VK_SHIFT && vk != VK_CONTROL && vk != VK_MENU) pressed = vk;
            }
        }
        if (!g_captureArmed) {
            g_captureArmed = !any;
            return;
        }
        if (pressed == VK_ESCAPE || GetTickCount64() - g_captureStart > kCaptureMs) {
            g_capturing = false;
        } else if (pressed) {
            g_capturing = false;
            if (it.bindKey) it.bindKey(pressed);
            Beep("SELECT");
        }
        return;
    }
    const unsigned pad = g_pad ? g_pad() & 0xF3FFu : 0;
    if (!g_captureArmed) {  // the press that started the capture is still down
        g_captureArmed = pad == 0;
        return;
    }
    g_captureMask |= pad;
    if (GetAsyncKeyState(VK_ESCAPE) & 0x8000 || GetTickCount64() - g_captureStart > kCaptureMs) {
        g_capturing = false;
    } else if (it.allowOff && (GetAsyncKeyState(VK_BACK) & 0x8000)) {
        g_capturing = false;
        Commit(it, "0x0000");
    } else if (pad == 0 && g_captureMask) {
        g_capturing = false;
        char text[16];
        std::snprintf(text, sizeof(text), "0x%04X", g_captureMask);
        Commit(it, text);
    }
}

void Navigate() {
    const std::vector<int> tops = Tops();
    const int count = g_level ? static_cast<int>(g_pages[g_pageIndex].items.size()) : static_cast<int>(tops.size());
    int& sel = g_sel[g_level];
    if (!g_level && sel >= count) sel = 0;
    if (g_level) {
        const auto& items = g_pages[g_pageIndex].items;
        if (!Selectable(items[static_cast<std::size_t>(sel)])) sel = NextSelectable(items, sel, 1);
        if (Fire(0, kUp)) sel = NextSelectable(items, sel, -1), Beep("NAV_UP_DOWN");
        if (Fire(1, kDown)) sel = NextSelectable(items, sel, 1), Beep("NAV_UP_DOWN");
    } else {
        if (Fire(0, kUp)) sel = (sel + count - 1) % count, Beep("NAV_UP_DOWN");
        if (Fire(1, kDown)) sel = (sel + 1) % count, Beep("NAV_UP_DOWN");
    }
    const bool left = Fire(2, kLeft), right = Fire(3, kRight);
    const bool accept = Call<BOOL>(gta::IS_DISABLED_CONTROL_JUST_PRESSED, 0, kAccept);
    if (Call<BOOL>(gta::IS_DISABLED_CONTROL_JUST_PRESSED, 0, kCancel)) {
        Beep("BACK");
        if (g_level && !g_back.empty()) g_pageIndex = g_back.back().first, g_sel[1] = g_back.back().second, g_back.pop_back();
        else if (g_level) g_level = 0;
        else g_open = false;
        return;
    }
    if (!g_level) {
        if (accept || right) {
            g_pageIndex = tops[static_cast<std::size_t>(sel)], g_level = 1, g_back.clear(), Beep("SELECT");
            g_sel[1] = FirstSelectable(g_pages[static_cast<std::size_t>(g_pageIndex)].items);
        }
        return;
    }
    Item& it = g_pages[g_pageIndex].items[static_cast<std::size_t>(sel)];
    if (it.kind == Kind::Link) {
        if (accept || right)
            for (std::size_t i = 0; i < g_pages.size(); ++i)
                if (g_pages[i].title == it.target) {
                    g_back.emplace_back(g_pageIndex, sel);
                    g_pageIndex = static_cast<int>(i), g_sel[1] = FirstSelectable(g_pages[i].items), Beep("SELECT");
                    break;
                }
    } else if (it.kind == Kind::Heading) {
    } else if (it.kind == Kind::Action) {
        if (accept) {
            Beep("SELECT");
            g_open = false;  // actions act on the game: give the controls back first
            if (it.run) it.run();
        }
    } else if (it.kind == Kind::Chord || it.kind == Kind::Key) {
        if (accept) g_capturing = true, g_captureArmed = false, g_captureMask = 0, g_captureStart = GetTickCount64();
    } else if (left || right || accept) {
        Commit(it, Step(it, it.current ? it.current() : g_cache[it.key], left ? -1 : 1));
        Beep("NAV_LEFT_RIGHT");
    }
}
}  // namespace

void Start(const std::wstring& ini, std::vector<Page> pages, std::function<unsigned()> padButtons, LogFn log) {
    g_ini = ini, g_pages = std::move(pages), g_pad = std::move(padButtons), g_log = log;
    g_top.clear(), g_back.clear();
    for (std::size_t i = 0; i < g_pages.size(); ++i) {
        if (g_pages[i].parent.empty()) { g_top.push_back(static_cast<int>(i)); continue; }
        for (Page& parent : g_pages)
            if (parent.title == g_pages[i].parent) {
                Item link;
                link.kind = Kind::Link, link.label = g_pages[i].title, link.target = g_pages[i].title;
                link.visible = g_pages[i].visible;
                parent.items.push_back(std::move(link));
            }
    }
    g_level = 0, g_sel[0] = g_sel[1] = 0;
    g_cache.clear();
    g_presetDefaults.clear();
}

void SetPresetDefaults(std::vector<std::pair<std::string, std::string>> defaults) {
    g_presetDefaults = std::move(defaults);
}

bool ApplyPreset(const std::string& name, const std::vector<std::pair<std::string, std::string>>& settings) {
    Settings next, user;
    std::string error;
    if (!presets::Build(g_pages, g_presetDefaults, settings, next, error)) { Say(error); return false; }
    return User(user) && Activate(name, next, &user);
}

bool ApplySettings(const std::string& name, const std::vector<std::pair<std::string, std::string>>& settings) {
    auto next = Current();
    for (const auto& [key, value] : settings) {
        if (!next.count(key)) { Say("Setting unavailable: " + key); return false; }
        next[key] = value;
    }
    Settings user;
    return User(user) && Activate(name, next, &user);
}

bool FirstRunPreset(const std::string& name, const std::vector<std::pair<std::string, std::string>>& settings) {
    // A menu-saved User snapshot means the player has settings already: never overwrite them.
    wchar_t marker[8]{};
    GetPrivateProfileStringW(L"Preset.User", L"__schema", L"", marker, 8, g_ini.c_str());
    if (*marker) return false;
    g_quiet = true;
    const bool ok = ApplySettings(name, settings);
    g_quiet = false;
    return ok;
}

bool RestoreUserPreset() {
    Settings user;
    return User(user) && Activate("User", user, &user);
}

bool SaveCustomPreset() {
    const auto current = Current();
    std::string error;
    if (!presets::Validate(g_pages, current, error)) { Say(error); return false; }
    if (!Save(nullptr, nullptr, &current)) return false;
    Say("Custom preset saved (keyboard bindings stay personal)");
    return true;
}

bool RestoreCustomPreset() {
    Settings custom, user;
    bool exists = false;
    if (!Snapshot(L"Preset.Custom", custom, exists)) {
        Say(exists ? "Custom snapshot invalid; settings unchanged" : "Save a Custom preset first"); return false;
    }
    return User(user) && Activate("Custom", custom, &user);
}

void Toggle() {
    if (g_pages.empty()) return;
    if (g_open) {
        g_open = g_capturing = false;
        g_closing = true;
        return;
    }
    if (gta::FrontendActive()) return;
    Refresh();
    g_open = true;
}

void OpenPage(const std::string& title) {
    if (g_open || g_pages.empty()) return Toggle();
    for (std::size_t i = 0; i < g_pages.size(); ++i)
        if (g_pages[i].title == title) {
            Toggle();
            if (g_open) g_pageIndex = static_cast<int>(i), g_level = 1, g_sel[1] = FirstSelectable(g_pages[i].items), g_back.clear();
            return;
        }
}

bool IsOpen() { return g_open; }

std::string SelectedKey() {
    if (!g_open || g_level != 1 || g_capturing) return {};
    const auto& items = g_pages[static_cast<std::size_t>(g_pageIndex)].items;
    const int i = g_sel[1];
    return i >= 0 && i < static_cast<int>(items.size()) ? items[static_cast<std::size_t>(i)].key : std::string{};
}
void Status(const std::string& text) { Say(text); }

void DrawPanel(const std::string& banner, const std::string& title, const std::string& counter,
               const std::vector<PanelRow>& rows, const std::string& help) {
    Panel(banner, title, counter, rows, help);
}

void Tick() {
    // Pause (199, 200) stays GTA's; one control at a time, never DISABLE_ALL_CONTROL_ACTIONS
    // (it stops Rockstar Editor recording, see main.cpp DisableSkateControls). While open
    // this is the only control loop (DisableSkateControls skips), so it also runs on the
    // frame the menu closes.
    const auto disableControls = [] {
        for (int c = 0; c < 360; ++c)
            if (c != 199 && c != 200) Call<void>(gta::DISABLE_CONTROL_ACTION, 0, c, 0); // each named: no related walk (main.cpp)
    };
    if (!g_open) {
        if (g_closing) disableControls();
        g_closing = false;
        return;
    }
    if (gta::FrontendActive()) {
        g_open = g_capturing = false;
        disableControls();
        return;
    }
    disableControls();
    if (g_capturing) Capture(g_pages[g_pageIndex].items[static_cast<std::size_t>(g_sel[1])]);
    else Navigate();
    if (g_open) Draw();
}

}  // namespace menu
