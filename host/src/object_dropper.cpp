#include "object_dropper.h"
#include "object_dropper_logic.h"
#include "natives.h"
#include <windows.h>
#include <cmath>
#include <cstdio>
#include <iterator>
#include <map>
#include <string>
#include <vector>

using gta::Call;

namespace objectdropper {
namespace {

// What the dropper offers: a name and the GTA models that make it (the first
// one this game has). Every model has an exact template in the world cache
// (sk8v.prop-models), so Skate rides, grinds and launches it as it looks.
// `sink`: metres the object goes into the ground beyond its bounding box, so
// the board rolls onto it instead of hitting a corner (Manatros 2026-10-19:
// "slightly above ground so I hit the corner"). A raised floor (the half
// pipe's stands 0.217 m on its base) gets a collision approach slope in the
// runtime instead (dynamic.rs entry_slopes): sunk, the ground showed through it.
constexpr float kSink = 0.02f;
struct Offer { const char* name; const char* models[2]; float sink; };
constexpr Offer kOffers[] = {
    {"Kicker", {"prop_skate_kickers", nullptr}, kSink},
    {"Flat ramp", {"prop_skate_flatramp", nullptr}, kSink},
    {"Quarter pipe", {"prop_skate_quartpipe", nullptr}, kSink},
    {"Half pipe", {"prop_skate_halfpipe", nullptr}, kSink},
    {"Spine", {"prop_skate_spiner", nullptr}, kSink},
    {"Funbox", {"prop_skate_funbox", nullptr}, kSink},
    {"Rail", {"prop_skate_rail", nullptr}, kSink},
    {"Bench (ledge)", {"prop_bench_01a", "prop_bench_02"}, kSink},
    {"Picnic table", {"prop_picnictable_01", nullptr}, kSink},
    {"Concrete barrier", {"prop_mp_barrier_02b", nullptr}, kSink},
    {"Wooden crate", {"prop_box_wood02a", "prop_box_wood01a"}, kSink},
    {"Stunt ramp (small)", {"prop_mp_ramp_01", nullptr}, kSink},
    {"Stunt ramp (medium)", {"prop_mp_ramp_02", nullptr}, kSink},
    {"Stunt ramp (large)", {"prop_mp_ramp_03", nullptr}, kSink},
};

struct Kind { const char* name; Hash model; float bottom; float sink; };

dropper::State g_state;
std::vector<Kind> g_kinds;
bool g_resolved = false, g_openRequested = false, g_clearRequested = false, g_closeRequested = false;
int g_ghost = 0;
Hash g_ghostModel = 0;
std::map<std::uint32_t, int> g_objects;  // dropped id -> GTA object
const char* g_notice = nullptr;
DWORD g_pauseBlockUntil = 0;
DWORD g_clearHeldSince = 0;
// The dropper's own camera: orbits the ghost (mouse turns it, the wheel zooms),
// so W A S D move only the ghost and nothing follows the player.
int g_cam = 0;
float g_camYaw = 0.0f, g_camPitch = -35.0f, g_camDistance = 9.0f;

void Resolve(util::LogFn log) {
    if (g_resolved) return;
    g_resolved = true;
    for (const Offer& o : kOffers) {
        for (const char* m : o.models) {
            if (!m) continue;
            const Hash h = util::Joaat(m);
            if (!Call<BOOL>(gta::IS_MODEL_IN_CDIMAGE, h) || !Call<BOOL>(gta::IS_MODEL_VALID, h)) continue;
            Vector3 lo{}, hi{};
            Call<void>(gta::GET_MODEL_DIMENSIONS, h, &lo, &hi);
            g_kinds.push_back({o.name, h, lo.z, o.sink});
            break;
        }
    }
    util::Logf(log, "object dropper: %zu of %zu objects in this game", g_kinds.size(), std::size(kOffers));
}

void Delete(int& object) {
    if (object && Call<BOOL>(gta::DOES_ENTITY_EXIST, object)) Call<void>(gta::DELETE_OBJECT, &object);
    object = 0;
}

int Create(Hash model, float x, float y, float z, float heading, bool ghost) {
    int object = Call<int>(gta::CREATE_OBJECT_NO_OFFSET, model, x, y, z, false, false, false, 0);
    if (!object) return 0;
    Call<void>(gta::SET_ENTITY_AS_MISSION_ENTITY, object, true, false);
    Call<void>(gta::SET_ENTITY_HEADING, object, heading);
    Call<void>(gta::FREEZE_ENTITY_POSITION, object, true);
    if (ghost) {
        Call<void>(gta::SET_ENTITY_COLLISION, object, false, false);
        Call<void>(gta::SET_ENTITY_ALPHA, object, 150, false);
    }
    return object;
}

// The GTA objects for what the dropper holds: picked up ones go.
void Sync() {
    for (auto it = g_objects.begin(); it != g_objects.end();) {
        const bool kept = std::any_of(g_state.placed.begin(), g_state.placed.end(),
                                      [&](const dropper::Placed& p) { return p.id == it->first; });
        if (kept) { ++it; continue; }
        Delete(it->second);
        it = g_objects.erase(it);
    }
}

void CloseCamera() {
    if (!g_cam) return;
    Call<void>(gta::SET_CAM_ACTIVE, g_cam, false);
    Call<void>(gta::RENDER_SCRIPT_CAMS, false, true, 300, true, false, 0);
    Call<void>(gta::DESTROY_CAM, g_cam, false);
    g_cam = 0;
}

void OpenCamera(float yaw) {
    if (g_cam) return;
    g_cam = Call<int>(gta::CREATE_CAM, "DEFAULT_SCRIPTED_CAMERA", true);
    g_camYaw = yaw, g_camPitch = -35.0f, g_camDistance = 9.0f;
    Call<void>(gta::SET_CAM_ACTIVE, g_cam, true);
    Call<void>(gta::RENDER_SCRIPT_CAMS, true, true, 300, true, false, 0);
}

// Places the orbit camera around (x, y, z): mouse turns and tilts it, the wheel zooms.
void UpdateCamera(bool controller, float x, float y, float z) {
    if (!g_cam) return;
    if (!controller) {
        g_camYaw -= Call<float>(gta::GET_DISABLED_CONTROL_NORMAL, 0, 1) * 8.0f;
        g_camPitch -= Call<float>(gta::GET_DISABLED_CONTROL_NORMAL, 0, 2) * 6.0f;
        if (Call<BOOL>(gta::IS_DISABLED_CONTROL_PRESSED, 0, 241)) g_camDistance -= 0.6f;  // wheel up
        if (Call<BOOL>(gta::IS_DISABLED_CONTROL_PRESSED, 0, 242)) g_camDistance += 0.6f;  // wheel down
    }
    g_camPitch = (std::max)(-85.0f, (std::min)(-5.0f, g_camPitch));
    g_camDistance = (std::max)(3.0f, (std::min)(30.0f, g_camDistance));
    const float yaw = g_camYaw * 3.14159265f / 180.0f, pitch = g_camPitch * 3.14159265f / 180.0f;
    // GTA heading convention: forward (-sin, cos); the camera sits behind and above.
    const float fx = -std::sin(yaw) * std::cos(pitch), fy = std::cos(yaw) * std::cos(pitch), fz = std::sin(pitch);
    Call<void>(gta::SET_CAM_COORD, g_cam, x - fx * g_camDistance, y - fy * g_camDistance, z + 0.5f - fz * g_camDistance);
    Call<void>(gta::SET_CAM_ROT, g_cam, g_camPitch, 0.0f, g_camYaw, 2);
}

void CloseGhost() {
    CloseCamera();
    Delete(g_ghost);
    if (g_ghostModel) Call<void>(gta::SET_MODEL_AS_NO_LONGER_NEEDED, g_ghostModel);
    g_ghostModel = 0;
}

// The object origin's height for `kind` at (x, y): the ground (or a dropped
// object) under it, plus the lift.
float GroundZ(const Kind& k, float x, float y, float fromZ) {
    float ground = 0.0f;
    if (!Call<BOOL>(gta::GET_GROUND_Z_FOR_3D_COORD, x, y, fromZ + 4.0f, &ground, 0, 0)) ground = fromZ - 1.0f; // the player's feet when GTA finds no ground
    return ground - k.bottom - k.sink + g_state.lift;
}

bool Held(int vk) { return (GetAsyncKeyState(vk) & 0x8000) != 0; }

bool Focused() {
    DWORD process = 0;
    const HWND window = GetForegroundWindow();
    if (window) GetWindowThreadProcessId(window, &process);
    return window && process == GetCurrentProcessId();
}

// Edges of the keys and buttons the dropper uses.
struct Edges {
    std::map<int, bool> was;
    bool Pressed(int id, bool down) {
        const bool before = was[id];
        was[id] = down;
        return down && !before;
    }
} g_edges;

float Stick(short v) {
    const float f = v / 32767.0f;
    return std::fabs(f) < 0.24f ? 0.0f : (std::max)(-1.0f, (std::min)(1.0f, f));
}

dropper::Input ReadInput(bool controller, const SvPad& pad) {
    dropper::Input in;
    const DWORD now = GetTickCount();
    bool clearHeld = false;
    if (controller) {
        const unsigned b = pad.buttons;
        in.moveX = Stick(pad.left_x), in.moveY = Stick(pad.left_y);
        in.turn = Stick(pad.right_x);
        in.lift = (pad.right_trigger - pad.left_trigger) / 255.0f;
        in.fast = (b & 0x0040) != 0;                         // L3
        in.previous = g_edges.Pressed(1, (b & 0x0100) != 0);  // LB
        in.next = g_edges.Pressed(2, (b & 0x0200) != 0);      // RB
        in.drop = g_edges.Pressed(3, (b & 0x1000) != 0);      // A
        in.close = g_edges.Pressed(4, (b & 0x2000) != 0);     // B
        in.pickUp = g_edges.Pressed(5, (b & 0x4000) != 0);    // X
        clearHeld = (b & 0x8000) != 0;                         // Y held
    } else if (Focused()) {
        in.moveX = (Held('D') ? 1.0f : 0.0f) - (Held('A') ? 1.0f : 0.0f);
        in.moveY = (Held('W') ? 1.0f : 0.0f) - (Held('S') ? 1.0f : 0.0f);
        in.turn = (Held('E') ? 1.0f : 0.0f) - (Held('Q') ? 1.0f : 0.0f);
        in.lift = (Held('R') ? 1.0f : 0.0f) - (Held('F') ? 1.0f : 0.0f);
        in.fast = Held(VK_SHIFT);
        in.previous = g_edges.Pressed(11, Held(VK_LEFT));
        in.next = g_edges.Pressed(12, Held(VK_RIGHT));
        in.drop = g_edges.Pressed(13, Held(VK_SPACE) || Held(VK_RETURN));
        in.close = g_edges.Pressed(14, Held(VK_ESCAPE));
        in.pickUp = g_edges.Pressed(15, Held(VK_BACK));
        clearHeld = Held(VK_DELETE);
    }
    // Removing everything needs a one-second hold.
    if (!clearHeld) g_clearHeldSince = 0;
    else if (!g_clearHeldSince) g_clearHeldSince = now;
    else if (g_clearHeldSince != 1 && now - g_clearHeldSince >= 1000) in.clearAll = true, g_clearHeldSince = 1;
    return in;
}

void BlockGtaControls(bool) {
    // Everything, mouse look too: the dropper's camera reads the mouse itself.
    Call<void>(gta::DISABLE_ALL_CONTROL_ACTIONS, 0);
}

void DrawPanel(bool controller) {
    if (g_kinds.empty()) return;
    const int n = static_cast<int>(g_kinds.size());
    const int k = g_state.kind;
    const float x = 0.015f;
    float y = 0.30f;
    gta::Text("OBJECT DROPPER", x, y, 0.45f, 255, 217, 77);
    y += 0.035f;
    gta::Text(g_kinds[k].name, x, y, 0.5f, 255, 255, 255);
    y += 0.035f;
    char line[160];
    std::snprintf(line, sizeof(line), "%s  |  %s", g_kinds[(k + n - 1) % n].name, g_kinds[(k + 1) % n].name);
    gta::Text(line, x, y, 0.3f, 190, 190, 190);
    y += 0.025f;
    std::snprintf(line, sizeof(line), "Placed %zu / %zu   Height %.1f m", g_state.placed.size(), dropper::kMaxObjects, g_state.lift);
    gta::Text(line, x, y, 0.3f, 190, 190, 190);
    y += 0.035f;
    static const char* const kPad[] = {"Left stick  move (L3 faster)", "The camera follows the object", "Right stick  turn", "LT / RT  lower / raise",
                                       "LB / RB  previous / next object", "A  drop", "X  pick up the nearest",
                                       "Y (hold)  remove all", "B  close"};
    static const char* const kKeys[] = {"W A S D  move (Shift faster)", "Mouse  turn the camera, wheel  zoom", "Q / E  turn", "F / R  lower / raise",
                                        "Left / Right  previous / next object", "Space or Enter  drop",
                                        "Backspace  pick up the nearest", "Delete (hold)  remove all", "Esc  close"};
    for (const char* hint : controller ? kPad : kKeys) {
        gta::Text(hint, x, y, 0.28f, 255, 255, 255);
        y += 0.022f;
    }
}

}  // namespace

void RequestOpen() { g_openRequested = true; }
void RequestClose() { g_closeRequested = true; }
void RequestClear() { g_clearRequested = true; }
bool IsOpen() { return g_state.open; }
int Ghost() { return g_ghost; }
std::size_t Count() { return g_objects.size(); }
std::vector<int> Objects() {
    std::vector<int> out;
    for (const auto& [id, object] : g_objects) out.push_back(object);
    return out;
}
const char* TakeNotice() {
    const char* n = g_notice;
    g_notice = nullptr;
    return n;
}

void Tick(int ped, bool allowed, bool controller, const SvPad& pad, util::LogFn log) {
    const DWORD now = GetTickCount();
    if (static_cast<int>(now - g_pauseBlockUntil) < 0)
        for (int group : {0, 2})
            for (int control : {199, 200}) Call<void>(gta::DISABLE_CONTROL_ACTION, group, control, 1);
    if (g_clearRequested) {
        g_clearRequested = false;
        g_state.placed.clear();
        Sync();
        util::Logf(log, "object dropper: all objects removed (menu)");
        g_notice = "Object dropper: all objects removed";
    }
    if (g_openRequested) {
        g_openRequested = false;
        if (!g_state.open) {
            Resolve(log);
            if (g_kinds.empty()) {
                g_notice = "Object dropper: this game has none of its objects";
            } else if (!allowed) {
                g_notice = "Object dropper: only on foot or while skating";
            } else {
                const Vector3 p = Call<Vector3>(gta::GET_ENTITY_COORDS, ped, 1);
                const Vector3 cam = Call<Vector3>(gta::GET_FINAL_RENDERED_CAM_ROT, 2);
                g_state.Open(static_cast<int>(g_kinds.size()), p.x, p.y, cam.z);
                OpenCamera(cam.z);
                g_edges.was.clear();
                // Keys held from the menu (Enter, Esc) count only once let go.
                for (int id : {3, 4, 13, 14}) g_edges.was[id] = true;
                g_clearHeldSince = 1;
                util::Logf(log, "object dropper: open at (%.1f, %.1f), %zu objects placed", g_state.x, g_state.y, g_state.placed.size());
            }
        }
    }
    if (g_closeRequested) {
        g_closeRequested = false;
        if (g_state.open) {
            g_state.open = false;
            CloseGhost();
            util::Logf(log, "object dropper: closed (hotkey), %zu placed", g_state.placed.size());
            return;
        }
    }
    if (!g_state.open) return;
    if (!allowed) {
        g_state.open = false;
        CloseGhost();
        util::Logf(log, "object dropper: closed (the player can't drop here now)");
        return;
    }
    g_pauseBlockUntil = now + 400;
    BlockGtaControls(controller);

    const Vector3 p = Call<Vector3>(gta::GET_ENTITY_COORDS, ped, 1);
    Vector3 cam = Call<Vector3>(gta::GET_FINAL_RENDERED_CAM_ROT, 2);
    if (g_cam) cam.z = g_camYaw;  // W A S D move the ghost along the dropper's camera
    static DWORD s_last = now;
    const float dt = (std::min)(0.1f, static_cast<float>(now - s_last) / 1000.0f);
    s_last = now;
    const dropper::Input in = ReadInput(controller, pad);
    const int before = g_state.kind;
    const dropper::Event e = g_state.UpdateResolved(in, cam.z, p.x, p.y, dt, [&](const dropper::State& state) {
        return GroundZ(g_kinds[state.kind], state.x, state.y, p.z);
    });
    switch (e) {
    case dropper::Event::Dropped: {
        const dropper::Placed& d = g_state.placed.back();
        const Kind& k = g_kinds[d.kind];
        const int object = Call<BOOL>(gta::HAS_MODEL_LOADED, k.model) ? Create(k.model, d.x, d.y, d.z, d.heading, false) : 0;
        if (!object) {
            g_state.placed.pop_back();
            g_notice = "Object dropper: GTA has not loaded that object yet";
            util::Logf(log, "object dropper: %s not created (model 0x%08X loaded %d)", k.name, k.model,
                       static_cast<int>(Call<BOOL>(gta::HAS_MODEL_LOADED, k.model)));
            break;
        }
        g_objects[d.id] = object;
        util::Logf(log, "object dropper: %s (0x%08X) dropped at (%.2f, %.2f, %.2f) heading %.0f, %zu placed",
                   k.name, k.model, d.x, d.y, d.z, d.heading, g_state.placed.size());
        break;
    }
    case dropper::Event::PickedUp:
    case dropper::Event::Cleared:
        Sync();
        util::Logf(log, "object dropper: %s, %zu placed", e == dropper::Event::Cleared ? "all removed" : "picked up", g_state.placed.size());
        break;
    case dropper::Event::Full: g_notice = "Object dropper: that's the most objects; pick one up first"; break;
    case dropper::Event::NothingNear: g_notice = "Object dropper: nothing near the ghost to pick up"; break;
    case dropper::Event::Closed:
        CloseGhost();
        util::Logf(log, "object dropper: closed, %zu placed", g_state.placed.size());
        return;
    default: break;
    }

    // The ghost: the object picked, see-through and without collision.
    const Kind& shown = g_kinds[g_state.kind];
    if (g_state.kind != before || (g_ghost && !Call<BOOL>(gta::DOES_ENTITY_EXIST, g_ghost))) Delete(g_ghost);
    if (g_ghostModel != shown.model) {
        Delete(g_ghost);
        if (g_ghostModel) Call<void>(gta::SET_MODEL_AS_NO_LONGER_NEEDED, g_ghostModel);
        g_ghostModel = shown.model;
    }
    Call<void>(gta::REQUEST_MODEL, shown.model);
    const float gz = GroundZ(shown, g_state.x, g_state.y, p.z);
    if (!g_ghost && Call<BOOL>(gta::HAS_MODEL_LOADED, shown.model))
        g_ghost = Create(shown.model, g_state.x, g_state.y, gz, g_state.heading, true);
    if (g_ghost) {
        Call<void>(gta::SET_ENTITY_COORDS_NO_OFFSET, g_ghost, g_state.x, g_state.y, gz, 0, 0, 0);
        Call<void>(gta::SET_ENTITY_HEADING, g_ghost, g_state.heading);
    }
    UpdateCamera(controller, g_state.x, g_state.y, gz);
    if (!gta::FrontendActive()) DrawPanel(controller);
}

}  // namespace objectdropper
