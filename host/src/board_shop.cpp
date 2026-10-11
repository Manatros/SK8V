#include "board_shop.h"
#include "board_parts.h"
#include "host_util.h"
#include "menu.h"
#include "natives.h"
#include "runtime_bridge.h"
#include <windows.h>
#include <cmath>
#include <cstring>
#include <cstdlib>
#include <cstdio>
#include <sstream>
#include <vector>

namespace boardshop {
namespace {
using namespace gta;

// GTA's controls: G (INPUT_DETONATE, D-pad left) opens the boards; E stays
// the store's own clothing counter. The menu reads the frontend group.
constexpr int kOpen = 47;
constexpr int kUp = 172, kDown = 173, kLeft = 174, kRight = 175, kAccept = 201, kCancel = 202, kReset = 203;

RuntimeBridge* g_rt = nullptr;
void (*g_log)(const char*) = nullptr;
void (*g_wheels)(float) = nullptr;
bool g_ready = false, g_open = false;
std::uint32_t g_model = 0;
int g_tierShown = -1;

void Log(const std::string& s) {
    if (g_log) g_log(s.c_str());
}

// Binco and the Discount Store (0), Suburban (1); Ponsonbys (2) sells no boards.
int TierHere(int ped) {
    const int interior = Call<int>(GET_INTERIOR_FROM_ENTITY, ped);
    if (!interior) return -1;
    Vector3 at{};
    std::uint32_t hash = 0;
    Call<void>(GET_INTERIOR_LOCATION_AND_NAMEHASH, interior, &at, &hash);
    if (hash == util::Joaat("v_clotheslo")) return 0;
    if (hash == util::Joaat("v_clothesmid")) return 1;
    if (hash == util::Joaat("v_clotheshi")) return 2;
    return -1;
}

// Michael, Franklin and Trevor by model (each keeps their own boards and cash).
const char* Who(std::uint32_t model) {
    if (model == util::Joaat("player_zero")) return "michael";
    if (model == util::Joaat("player_one")) return "franklin";
    if (model == util::Joaat("player_two")) return "trevor";
    return nullptr;
}

std::uint32_t CashStat(std::uint32_t model) {
    if (model == util::Joaat("player_zero")) return util::Joaat("SP0_TOTAL_CASH");
    if (model == util::Joaat("player_one")) return util::Joaat("SP1_TOTAL_CASH");
    if (model == util::Joaat("player_two")) return util::Joaat("SP2_TOTAL_CASH");
    return 0;
}

std::vector<std::string> Split(const std::string& text, char by) {
    std::vector<std::string> out;
    std::string item;
    std::istringstream in(text);
    while (std::getline(in, item, by))
        if (!item.empty()) out.push_back(item);
    return out;
}

// A line at the bottom right: GTA's help box stays the store's own (its
// clothing counter prompt), which a help text of ours would replace.
void Hint(const char* text) {
    gta::Text(text, 0.0f, 0.93f, 0.36f, 255, 255, 255, 230, gta::Align::Right, 0.985f, true);
}

void Sound(const char* name, const char* set = "HUD_FRONTEND_DEFAULT_SOUNDSET") {
    Call<void>(PLAY_SOUND_FRONTEND, -1, name, set, 1);
}

void Ride() {
    boardparts::SetLoadout(Split(g_rt->ShopText(1), ','));
    if (g_wheels) {
        const std::string h = g_rt->ShopText(4);
        g_wheels(h.empty() ? -1.0f : static_cast<float>(std::atof(h.c_str())));
    }
}

void Close() {
    g_rt->ShopClose();
    g_open = false;
    boardparts::Preview({}, SvBoardPose{});
}

// The preview board's axes (rows: board X, Y, Z) in the camera's frame
// (right, toward the camera, up): first the deck tilted 35 degrees toward
// the camera; turned all ways round with the mouse while the left button is
// held (it never turns by itself).
float g_orient[3][3];

void ResetOrient() {
    const float t = 0.61f, c = std::cos(t), s = std::sin(t);
    const float o[3][3] = {{1.0f, 0.0f, 0.0f}, {0.0f, c, s}, {0.0f, -s, c}};
    std::memcpy(g_orient, o, sizeof(o));
}

// Turns every row by `angle` about camera axis `axis` (0 right, 2 up).
void Turn(int axis, float angle) {
    const int a = axis == 0 ? 1 : 0, b = axis == 0 ? 2 : 1;
    const float c = std::cos(angle), s = std::sin(angle);
    for (auto& row : g_orient) {
        const float u = row[a], v = row[b];
        row[a] = c * u - s * v;
        row[b] = s * u + c * v;
    }
}

void ShowPreview() {
    if (Call<BOOL>(IS_DISABLED_CONTROL_PRESSED, 0, 24)) {
        Turn(2, Call<float>(GET_DISABLED_CONTROL_NORMAL, 0, 1) * 4.0f);
        Turn(0, Call<float>(GET_DISABLED_CONTROL_NORMAL, 0, 2) * 4.0f);
    }
    const Vector3 cam = Call<Vector3>(GET_GAMEPLAY_CAM_COORD);
    const Vector3 rot = Call<Vector3>(GET_GAMEPLAY_CAM_ROT, 2);
    const float yaw = rot.z * 0.017453292f, pitch = rot.x * 0.017453292f;
    const float fx = -std::sin(yaw) * std::cos(pitch), fy = std::cos(yaw) * std::cos(pitch), fz = std::sin(pitch);
    const float rx = std::cos(yaw), ry = std::sin(yaw);
    const float right[3] = {rx, ry, 0.0f}, back[3] = {std::sin(yaw), -std::cos(yaw), 0.0f}, up[3] = {0.0f, 0.0f, 1.0f};
    float at[16];
    for (int r = 0; r < 3; ++r) {
        for (int i = 0; i < 3; ++i) at[r * 4 + i] = g_orient[r][0] * right[i] + g_orient[r][1] * back[i] + g_orient[r][2] * up[i];
        at[r * 4 + 3] = 0.0f;
    }
    // 1.6 m ahead and 0.5 m right of the camera, a little below its line.
    at[12] = cam.x + fx * 1.6f + rx * 0.5f;
    at[13] = cam.y + fy * 1.6f + ry * 0.5f;
    at[14] = cam.z + fz * 1.6f - 0.2f;
    at[15] = 1.0f;
    SvBoardPose pose{};
    if (g_rt->ShopRestPose(at, pose)) boardparts::Preview(Split(g_rt->ShopText(2), ','), pose);
}

void Menu(int ped) {
    Call<void>(DISABLE_ALL_CONTROL_ACTIONS, 0);
    Call<void>(INVALIDATE_IDLE_CAM);
    struct Key { int control, input; const char* sound; };
    constexpr Key keys[] = {{kUp, 0, "NAV_UP_DOWN"},       {kDown, 1, "NAV_UP_DOWN"},   {kLeft, 2, "NAV_LEFT_RIGHT"},
                            {kRight, 3, "NAV_LEFT_RIGHT"}, {kAccept, 4, nullptr},       {kCancel, 5, "BACK"},
                            {kReset, 6, "SELECT"}};
    const std::uint32_t model = Call<std::uint32_t>(GET_ENTITY_MODEL, ped);
    const std::uint32_t stat = CashStat(model);
    for (const Key& k : keys) {
        if (!Call<BOOL>(IS_DISABLED_CONTROL_JUST_PRESSED, 2, k.control)) continue;
        int cash = 0;
        if (stat) Call<BOOL>(STAT_GET_INT, stat, &cash, -1);
        const SvShopResult r = g_rt->ShopInput(k.input, stat ? cash : 0);
        if (k.sound) Sound(k.sound);
        switch (r.kind) {
        case 1: Close(); return;
        case 2:
            Call<BOOL>(STAT_SET_INT, stat, cash - static_cast<int>(r.price), true);
            Sound("PURCHASE", "HUD_LIQUOR_STORE_SOUNDSET");
            Ride();
            break;
        case 3: case 4: Sound("SELECT"); Ride(); break;
        case 5: Sound("ERROR"); break;
        default: break;
        }
        break;
    }
    const auto lines = Split(g_rt->ShopText(0), '\n');
    if (lines.size() < 2) {
        Close();
        return;
    }
    const auto head = Split(lines[0], '\t');
    std::vector<menu::PanelRow> rows;
    for (std::size_t i = 2; i < lines.size(); ++i) {
        const auto cells = Split(lines[i], '\t');
        menu::PanelRow row;
        row.label = cells.size() > 0 ? cells[0] : "";
        row.value = cells.size() > 1 ? cells[1] : "";
        row.selected = cells.size() > 2 && cells[2] == "1";
        rows.push_back(row);
    }
    menu::DrawPanel("Boards", head.empty() ? "" : head[0], head.size() > 1 ? head[1] : "", rows,
                    lines[1] + "  Left mouse button + mouse: turn the board");
    ShowPreview();
}

} // namespace

bool Start(RuntimeBridge& runtime, const std::string& dataRoot, const std::string& gameDir, void (*log)(const char*),
           void (*wheels)(float hardness)) {
    g_rt = &runtime;
    g_log = log;
    g_wheels = wheels;
    g_model = 0;
    if (!runtime.HasShop()) {
        Log("board shop: the runtime has none");
        return g_ready = false;
    }
    g_ready = runtime.ShopStart(dataRoot, gameDir);
    Log("board shop: " + runtime.ShopText(3));
    return g_ready;
}

void Tick(bool busy) {
    if (!g_ready) return;
    const int ped = Call<int>(PLAYER_PED_ID);
    const std::uint32_t model = Call<std::uint32_t>(GET_ENTITY_MODEL, ped);
    if (model != g_model) {
        g_model = model;
        if (g_open) Close();
        char hex[16];
        std::snprintf(hex, sizeof(hex), "%08x", model);
        const char* who = Who(model);
        g_rt->ShopCharacter(who ? who : hex);
        Ride();
    }
    const int tier = TierHere(ped);
    if (tier != g_tierShown) {
        g_tierShown = tier;
        if (tier >= 0) Log("board shop: in a clothing store (tier " + std::to_string(tier) + (tier < 2 ? ", G opens the boards)" : ", sells no boards)"));
    }
    const bool can = !busy && (tier == 0 || tier == 1);
    if (g_open) {
        if (!can) {
            Close();
            return;
        }
        Menu(ped);
        return;
    }
    if (!can || menu::IsOpen() || Call<BOOL>(IS_PED_IN_ANY_VEHICLE, ped, false) ||
        Call<BOOL>(IS_PLAYER_WANTED_LEVEL_GREATER, Call<int>(PLAYER_ID), 0))
        return;
    Hint("G / D-pad left:  Skate 3 boards (complete or build your own)");
    // Skate mode disables GTA's controls; the key is read either way.
    if ((Call<BOOL>(IS_CONTROL_JUST_PRESSED, 0, kOpen) || Call<BOOL>(IS_DISABLED_CONTROL_JUST_PRESSED, 0, kOpen)) &&
        g_rt->ShopOpen(tier)) {
        g_open = true;
        ResetOrient();
        Sound("SELECT");
        Log("board shop: opened");
    }
}

bool IsOpen() {
    return g_open;
}

} // namespace boardshop
