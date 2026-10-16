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
constexpr int kVisibleRows = 12;
constexpr ULONGLONG kCaptureMs = 8000;

// GTA Interaction Menu proportions at 16:9 (width 432 px of 1920, rows 38 px of 1080).
constexpr float kW = 0.225f, kPad = 0.006f, kBanner = 0.09f, kSub = 0.03f, kRow = 0.035f, kChevron = 0.022f, kHelp = 0.06f;

std::wstring g_ini;
std::vector<Page> g_pages;
std::function<unsigned()> g_pad;
LogFn g_log = nullptr;
bool g_open = false;
bool g_closing = false;         // closed by Toggle / the pause menu this frame
int g_level = 0;                // 0 categories, 1 a page's items
int g_sel[2] = {0, 0};          // selection per level
int g_pageIndex = 0;
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

void Say(const std::string& text) {
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
    for (const auto& [key, unused] : Current()) {
        wchar_t value[1024]{};
        GetPrivateProfileStringW(section, Wide(key).c_str(), L"\x1", value, 1024, g_ini.c_str());
        if (value[0] == 1 && value[1] == 0) return false;
        out[key] = util::Narrow(value);
    }
    const auto count = GetPrivateProfileIntW(section, L"__count", -1, g_ini.c_str());
    std::string error;
    return count == out.size() && presets::Validate(g_pages, out, error);
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

// Stage all sections beside the INI, flush, then replace it once. No callbacks or
// active values change unless every write succeeds. Unrelated INI sections survive.
bool Save(const Settings* active, const Settings* user, const Settings* custom) {
    const auto temp = g_ini + L".preset.tmp";
    if (!CopyFileW(g_ini.c_str(), temp.c_str(), FALSE)) {
        if (GetLastError() != ERROR_FILE_NOT_FOUND) { Say("Could not stage settings; unchanged"); return false; }
        const HANDLE file = CreateFileW(temp.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (file == INVALID_HANDLE_VALUE) { Say("Could not stage settings; unchanged"); return false; }
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
    if (!written || !flushed || !MoveFileExW(temp.c_str(), g_ini.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
        DeleteFileW(temp.c_str()); Say("Could not save settings; unchanged"); return false;
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
    else Rect(x, y, kW, kRow, 0, 0, 0, 160);
    const int shade = selected ? 10 : 255;
    Text(label, x + kPad, y + 0.003f, 0.34f, shade);
    if (!value.empty()) Text(value, 0.0f, y + 0.003f, 0.34f, shade, Align::Right, x + kW - kPad);
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
    Rect(x, y, kW, kHelp, 0, 0, 0, 160);
    Text(help, x + kPad, y + 0.005f, 0.28f, 210, Align::Wrap, x + kW - kPad);
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
    const int count = g_level ? static_cast<int>(g_pages[g_pageIndex].items.size()) : static_cast<int>(g_pages.size());
    const int sel = g_sel[g_level];
    Rect(x, y, kW, kSub, 255, 110, 0, 235);
    Text(Upper(g_level ? g_pages[g_pageIndex].title : "Settings"), x + kPad, y + 0.0035f, 0.3f, 255);
    Text(Counter(sel, count), 0.0f, y + 0.0035f, 0.3f, 255, Align::Right, x + kW - kPad);
    y += kSub;

    const int first = std::clamp(sel - kVisibleRows / 2, 0, std::max(0, count - kVisibleRows));
    const int shown = std::min(kVisibleRows, count - first);
    for (int i = first; i < first + shown; ++i, y += kRow) {
        if (!g_level) {
            Row(x, y, g_pages[static_cast<std::size_t>(i)].title, ">", i == sel);
            continue;
        }
        const Item& it = g_pages[g_pageIndex].items[static_cast<std::size_t>(i)];
        std::string value = it.kind == Kind::Action ? ""
                            : it.kind == Kind::Key  ? (g_capturing && i == sel ? std::string("press a key (Esc: keep)") : it.show ? it.show() : "")
                            : i == sel              ? ShowSelected(it, g_cache[it.key])
                                                    : Show(it, g_cache[it.key]);
        if (it.kind == Kind::Chord && g_capturing && i == sel) value = ChordName(g_captureMask) + " ...";
        Row(x, y, it.label + (it.restart ? " *" : ""), value, i == sel);
    }

    // Chevron strip, then the description box.
    Rect(x, y, kW, kChevron, 0, 0, 0, 160);
    Chevron(x + kW / 2 - 0.008f, y + kChevron / 2, true);
    Chevron(x + kW / 2 + 0.008f, y + kChevron / 2, false);
    y += kChevron;
    Rect(x, y, kW, kHelp, 0, 0, 0, 160);
    std::string text;
    int shade = 210;
    if (g_capturing) {
        text = g_pages[g_pageIndex].items[static_cast<std::size_t>(sel)].allowOff
                   ? "Hold the buttons, release to set. Backspace = off, Esc = cancel"
                   : "Hold the buttons, release to set. Esc = cancel";
    } else if (GetTickCount64() < g_statusUntil) {
        text = g_status, shade = 255;
    } else {
        text = g_level ? kHint : PageHint(g_pages[static_cast<std::size_t>(sel)].title);
    }
    Text(text, x + kPad, y + 0.005f, 0.28f, shade, Align::Wrap, x + kW - kPad);
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
    const int count = g_level ? static_cast<int>(g_pages[g_pageIndex].items.size()) : static_cast<int>(g_pages.size());
    int& sel = g_sel[g_level];
    if (Fire(0, kUp)) sel = (sel + count - 1) % count, Beep("NAV_UP_DOWN");
    if (Fire(1, kDown)) sel = (sel + 1) % count, Beep("NAV_UP_DOWN");
    const bool left = Fire(2, kLeft), right = Fire(3, kRight);
    const bool accept = Call<BOOL>(gta::IS_DISABLED_CONTROL_JUST_PRESSED, 0, kAccept);
    if (Call<BOOL>(gta::IS_DISABLED_CONTROL_JUST_PRESSED, 0, kCancel)) {
        Beep("BACK");
        if (g_level) g_level = 0;
        else g_open = false;
        return;
    }
    if (!g_level) {
        if (accept) g_pageIndex = sel, g_level = 1, g_sel[1] = 0, Beep("SELECT");
        return;
    }
    Item& it = g_pages[g_pageIndex].items[static_cast<std::size_t>(sel)];
    if (it.kind == Kind::Action) {
        if (accept) {
            Beep("SELECT");
            g_open = false;  // actions act on the game: give the controls back first
            if (it.run) it.run();
        }
    } else if (it.kind == Kind::Chord || it.kind == Kind::Key) {
        if (accept) g_capturing = true, g_captureArmed = false, g_captureMask = 0, g_captureStart = GetTickCount64();
    } else if (left || right || accept) {
        Commit(it, Step(it, g_cache[it.key], left ? -1 : 1));
        Beep("NAV_LEFT_RIGHT");
    }
}
}  // namespace

void Start(const std::wstring& ini, std::vector<Page> pages, std::function<unsigned()> padButtons, LogFn log) {
    g_ini = ini, g_pages = std::move(pages), g_pad = std::move(padButtons), g_log = log;
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
            if (g_open) g_pageIndex = static_cast<int>(i), g_level = 1;
            return;
        }
}

bool IsOpen() { return g_open; }
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
