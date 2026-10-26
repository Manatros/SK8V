#pragma once
// In-game settings menu, drawn with GTA's own rect/text natives. '/' opens it;
// it replaces every hotkey and the hand-edited SkateVLegacy.ini: each item is
// one [SkateV] INI key, saved the moment it changes and applied live unless
// marked restart. Dev items sit on their own page.
//
// The model (items, stepping, display text) is plain C++ here so
// host/tests/menu_tests.cpp checks it without GTA; menu.cpp draws and reads input.
#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <functional>
#include <string>
#include <utility>
#include <vector>

namespace menu {

enum class Kind { Flag, Number, Choice, Chord, Action, Key, Link, Heading };

struct Item {
    Kind kind = Kind::Action;
    std::string label, key, def;  // key: [SkateV] INI name; def: its value when unset
    bool restart = false;         // takes effect after GTA restarts
    float lo = 0, hi = 1, step = 1;                  // Number
    std::function<std::string(float)> fmt;           // Number display (default %g)
    std::vector<std::string> values, names;          // Choice: INI text, shown text
    bool allowOff = false;                           // Chord: 0 (off) is a valid value
    std::function<void(const std::string&)> apply;   // live effect of a new INI value
    std::function<void()> run;                       // Action
    std::function<std::string()> show;               // Key: the key bound now
    std::function<void(int)> bindKey;                // Key: rebind to this virtual key
    std::function<std::string()> labelText;            // evaluated each draw
    std::function<std::string()> current;              // transient value, never saved
    std::string help;                                // shown in the box under the rows when selected
    std::string target;                              // Link: the title of the page it opens
    std::function<bool()> visible;                   // unset: always; hidden rows keep their setting
};

// A keyboard key: Accept waits for the next key (Esc cancels) and hands its
// virtual-key code to `bind`; `show` names the key bound now.
inline Item Key(std::string label, std::function<std::string()> show, std::function<void(int)> bind) {
    Item i;
    i.kind = Kind::Key, i.label = std::move(label), i.show = std::move(show), i.bindKey = std::move(bind);
    return i;
}

// A section title inside a page: not selectable, no INI key.
inline Item Heading(std::string label) {
    Item i;
    i.kind = Kind::Heading, i.label = std::move(label);
    return i;
}

struct Page {
    std::string title;
    std::vector<Item> items;
    std::string parent;  // a sub-page: listed as a row on this page instead of in the categories
    std::function<bool()> visible;  // unset: always (developer options, uninstalled components)
};

using Apply = std::function<void(const std::string&)>;

inline Item Flag(std::string label, std::string key, bool def, Apply apply = {}, bool restart = false) {
    Item i;
    i.kind = Kind::Flag, i.label = std::move(label), i.key = std::move(key), i.def = def ? "1" : "0";
    i.apply = std::move(apply), i.restart = restart;
    return i;
}
inline Item Number(std::string label, std::string key, float def, float lo, float hi, float step,
                   std::function<std::string(float)> fmt, Apply apply = {}, bool restart = false) {
    Item i;
    i.kind = Kind::Number, i.label = std::move(label), i.key = std::move(key);
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%g", def);
    i.def = buf, i.lo = lo, i.hi = hi, i.step = step, i.fmt = std::move(fmt);
    i.apply = std::move(apply), i.restart = restart;
    return i;
}
inline Item Choice(std::string label, std::string key, std::vector<std::string> values,
                   std::vector<std::string> names, Apply apply = {}, bool restart = false) {
    Item i;
    i.kind = Kind::Choice, i.label = std::move(label), i.key = std::move(key), i.def = values.front();
    i.values = std::move(values), i.names = std::move(names), i.apply = std::move(apply), i.restart = restart;
    return i;
}
inline Item Chord(std::string label, std::string key, unsigned def, bool allowOff, Apply apply = {}) {
    Item i;
    i.kind = Kind::Chord, i.label = std::move(label), i.key = std::move(key), i.allowOff = allowOff;
    char buf[16];
    std::snprintf(buf, sizeof(buf), "0x%04X", def);
    i.def = buf, i.apply = std::move(apply);
    return i;
}
inline Item Action(std::string label, std::function<void()> run) {
    Item i;
    i.label = std::move(label), i.run = std::move(run);
    return i;
}

inline std::string Label(const Item& item) { return item.labelText ? item.labelText() : item.label; }

// XInput button mask -> "LB + Back" (bit order of XINPUT_GAMEPAD_*).
inline std::string ChordName(unsigned mask) {
    static const char* const kNames[16] = {"D-Up", "D-Down", "D-Left", "D-Right", "Start", "Back", "L3", "R3",
                                           "LB",   "RB",     "",       "",        "A",     "B",    "X",  "Y"};
    std::string s;
    for (int bit = 0; bit < 16; ++bit)
        if ((mask >> bit & 1) && *kNames[bit]) s += (s.empty() ? "" : " + ") + std::string(kNames[bit]);
    return s.empty() ? "Off" : s;
}

inline bool SameText(const std::string& a, const std::string& b) {
    return a.size() == b.size() && std::equal(a.begin(), a.end(), b.begin(), [](char x, char y) {
               return std::tolower(static_cast<unsigned char>(x)) == std::tolower(static_cast<unsigned char>(y));
           });
}

inline int ChoiceIndex(const Item& it, const std::string& cur) {
    // Keep legacy INIs and User/Custom snapshots valid without a duplicate row.
    const std::string value = it.key == "CameraView" && SameText(cur, "FirstPerson") ? "Head" : cur;
    for (std::size_t i = 0; i < it.values.size(); ++i)
        if (SameText(it.values[i], value)) return static_cast<int>(i);
    return -1;
}

// The INI text after one left (-1) / right (+1) / accept (+1) press.
inline std::string Step(const Item& it, const std::string& cur, int dir) {
    switch (it.kind) {
    case Kind::Flag:
        return std::atof(cur.c_str()) != 0.0 ? "0" : "1";
    case Kind::Number: {
        // Next grid point past the current value, so odd defaults (1.4 dB, 0.3 m) join the grid.
        const double v = cur.empty() ? std::atof(it.def.c_str()) : std::atof(cur.c_str());
        const double g = dir > 0 ? std::floor(v / it.step + 1e-4) + 1.0 : std::ceil(v / it.step - 1e-4) - 1.0;
        const double out = std::clamp(g * it.step, static_cast<double>(it.lo), static_cast<double>(it.hi));
        char buf[32];
        std::snprintf(buf, sizeof(buf), "%g", out);
        return buf;
    }
    case Kind::Choice: {
        const int n = static_cast<int>(it.values.size());
        return it.values[static_cast<std::size_t>((std::max(ChoiceIndex(it, cur), 0) + dir + n) % n)];
    }
    default:
        return cur;
    }
}

// What the row shows on its right.
inline std::string Show(const Item& it, const std::string& cur) {
    switch (it.kind) {
    case Kind::Flag:
        return std::atof(cur.c_str()) != 0.0 ? "On" : "Off";
    case Kind::Number: {
        const float v = static_cast<float>(std::atof(cur.c_str()));
        if (it.fmt) return it.fmt(v);
        char buf[32];
        std::snprintf(buf, sizeof(buf), "%g", v);
        return buf;
    }
    case Kind::Choice: {
        const int i = ChoiceIndex(it, cur);
        return i >= 0 && i < static_cast<int>(it.names.size()) ? it.names[static_cast<std::size_t>(i)] : cur;
    }
    case Kind::Chord:
        return ChordName(static_cast<unsigned>(std::strtoul(cur.c_str(), nullptr, 0)));
    default:
        return {};
    }
}

// "3 / 12" for the subtitle bar (selected is 0-based).
// A list in the menu's look (banner, title bar with a counter, rows with a
// value, help box), for other modules' menus (the board shop). Rows past
// the visible count are the caller's to scroll.
struct PanelRow {
    std::string label, value;
    bool selected = false;
};
void DrawPanel(const std::string& banner, const std::string& title, const std::string& counter,
               const std::vector<PanelRow>& rows, const std::string& help);

inline std::string Counter(int selected, int total) { return std::to_string(selected + 1) + " / " + std::to_string(total); }

inline std::string Upper(std::string s) {
    for (char& c : s) c = static_cast<char>(std::toupper(static_cast<unsigned char>(c)));
    return s;
}

// Selected rows show Flag/Number/Choice values as "< value >" like GTA's Interaction Menu.
inline std::string ShowSelected(const Item& it, const std::string& cur) {
    const std::string v = Show(it, cur);
    return it.kind == Kind::Flag || it.kind == Kind::Number || it.kind == Kind::Choice ? "< " + v + " >" : v;
}

// Footer description for an item row.
constexpr const char* kHint = "Left/Right change   Back: categories   * restart GTA";

// Footer description for a category row, by page title.
inline std::string PageHint(const std::string& title) {
    static const std::pair<const char*, const char*> kHints[] = {
        {"Presets", "GTAxSkate is the recommended default. SK8V, RP, User and Custom apply other complete setups. Keyboard bindings stay personal."},
        {"Board", "How the board looks and the marks it leaves; the board shop and the board in your hand."},
        {"Controls", "Keyboard, controller buttons and the keyboard key list."},
        {"World", "What the board hits (cars, objects, stairs), people, swimming and guns, the object dropper and GPS."},
        {"Audio and radio", "The radio while skating and the Skate sound mix."},
        {"Advanced", "Fine tuning in sub-pages: physics, peds and vehicles, Hall of Meat, board in hand, developer."},
        {"World and roleplay", "Gentle pedestrian collisions, vehicle damage, injuries and GTA integration."},
        {"Camera", "Choose the starting view, mouse camera and cinematic look."},
        {"Keyboard keys", "Choose a layout or press Enter on an action to bind its key."},
        {"GPS", "GPS routes to board shops and skate spots."},
        {"Controller", "Controller button combinations, missions and keyboard on/off."},
        {"HUD", "What Skate shows on screen: trick display, key list, Hall of Meat."},
        {"Skate physics", "Fine tuning of pushing, steering, grip and the Skate rules."},
        {"Hall of Meat details", "Hall of Meat panels, x-ray and skater voice lines."},
        {"Board in hand", "Where the carried board sits in the hand."},
        {"Developer", "Diagnostics and test switches. Leave these alone unless testing."},
        {"Skating", "Skate on or off, stance, difficulty, how the board feels, posture and emotes."},
        {"Hall of Meat and records", "Hall of Meat bail records and the records display."},
        {"Peds and vehicles", "Collision, launch and dent settings for pedestrians and vehicles."},
        {"BackwardsMan", "BackwardsMan trick model, direction and remount."},
        {"Audio", "Skate sound mix, played through GTA's audio."},
        {"Controller and HUD", "Controller chords and the on-screen HUD."},
        {"Dev", "Developer options. Leave these alone unless testing."},
    };
    for (const auto& h : kHints)
        if (title == h.first) return h.second;
    return "Back or F10 closes";
}

using LogFn = void (*)(const char*);
// Once: the INI the items read and write, the pages, the raw pad (XInput button mask,
// for chord capture; the game's own controls are disabled while the menu is open).
void Start(const std::wstring& ini, std::vector<Page> pages, std::function<unsigned()> padButtons, LogFn log);
// The '/' key: opens the menu, closes it when open (the closing frame still
// keeps the game's controls, so the key press does not reach GTA).
void Toggle();
// Complete named presets start from every keyed row's defaults. Personal key bindings
// and action-only state are separate. Unknown/invalid keys reject the whole change.
void SetPresetDefaults(std::vector<std::pair<std::string, std::string>> defaults);
bool ApplyPreset(const std::string& name, const std::vector<std::pair<std::string, std::string>>& overrides);
bool ApplySettings(const std::string& name, const std::vector<std::pair<std::string, std::string>>& overrides);
// ApplySettings without opening the menu, only when no User snapshot exists yet (a fresh install).
bool FirstRunPreset(const std::string& name, const std::vector<std::pair<std::string, std::string>>& overrides);
bool RestoreUserPreset();
bool SaveCustomPreset();
bool RestoreCustomPreset();
// Opens the menu straight on the page titled `title` (closes it when open).
void OpenPage(const std::string& title);
bool IsOpen();
// The INI key of the selected row on an open settings page ("" otherwise).
std::string SelectedKey();
// Feedback stays in the menu even when all Skate overlays are hidden.
void Status(const std::string& text);
// Script thread, once per frame: draws and handles navigation while open.
void Tick();

}  // namespace menu
