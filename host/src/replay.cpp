#include "replay.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <atomic>
#include <ctime>
#include <fstream>
#include <iterator>
#include <map>
#include <thread>
#include <utility>

#include <windows.h>
#include <shlobj.h>

#include "board_native.h"
#include "board_parts.h"
#include "fisheye.h"
#include "gta_audio.h"
#include "natives.h"
#include "pose_hook.h"
#include "pose_math.h"
#include "replay_edit.h"
#include "replay_logic.h"
#include "replay_store.h"

namespace replay {
namespace {
using namespace gta;

constexpr double kTickSeconds = 1.0 / 60.0;  // Skate's fixed step
constexpr float kDeg = 0.01745329f;

// Instant Replay's cameras (design 2026-11-02-replay-editor, 2026-11-09-replay-look-sound).
// Skate 3's editor (design 2026-11-09-skate-replay-editor).
enum class Mode { Instant, Timeline, EditMenu, CameraMenu, CameraEdit, Finished, Prompt, Help, Reel, Loading };
enum class PromptAction { None, PlaceMarker, DeleteMarker, DeleteKey, ResetTimeline, ChangeCamera, CancelCameraEdit, SavePhoto, SaveReplay, DeleteSaved, ExitReplay };

void (*g_log)(const char*) = nullptr;
bool g_enabled = true;
double g_seconds = 30.0;
float g_worldScale = 0.0f;
Recording g_rec;
int g_bones = -1;            // bone count the slots were made for
std::uint64_t g_lastTick = 0;
Frame g_scratch;             // built in place each tick (no allocation once sized)
std::vector<float> g_pose;   // the presented pose
bool g_active = false;
Playhead g_head;
int g_cam = 0;               // the replay's own camera
Input g_input;               // both devices; the first frame takes what is held as it is
unsigned g_applyCalls = 0;
// Look and sound (design 2026-11-09-replay-look-sound).
bool g_sound = true, g_fisheyeOk = true;
float g_soundGainDb = -4.0f, g_filterStrength = 0.85f;
bool g_hudHidden = false;    // H / right stick click
bool g_keyboard = false;     // the help names the keyboard's keys

// What is applied to GTA now (changed only on a difference).
int g_appliedFilter = 0;
int g_appliedStrength = -1;
bool g_appliedFisheye = false;
int g_appliedShake = 0;

// The editor.
Mode g_mode = Mode::Instant;
Edit g_edit;
bool g_editValid = false;    // g_edit belongs to this recording
int g_menuSel = 0;
int g_editClip = 0;          // the clip the Edit Menu / camera mode edits
Clip g_clipBefore;           // camera mode: the clip as it was (Cancel)
CameraParams g_work;         // camera mode: the camera being set
bool g_workDirty = false;    // the player moved it since the last key/evaluation
float g_stepHeld = 0.0f;     // LB/RB / , . held: repeat after 0.35 s
struct PromptState {
    PromptAction action = PromptAction::None;
    Mode back = Mode::Timeline;
    std::string title, text;
    double time = 0.0;
    CameraKind kind = CameraKind::Game;
} g_prompt;
std::string g_notice;        // Skate's short messages ("Too close ..."), shown for a moment
double g_noticeUntil = 0.0;
double g_clock = 0.0;        // real seconds in the replay
int g_promptPed = 0;         // the ped for actions a prompt starts
int g_ped = 0;               // the skater (the camera's line tests ignore it)
// Cameras never inside geometry (chat 2026-11-10: "make sure no cam can clip through geometry").
float g_lastEye[3] = {};
bool g_lastEyeSet = false;
float g_skaterLen = -1.0f;   // Skater mode: the eye's distance (in at once, out eased)
int g_lastCamKey = -1;       // camera or clip the path belongs to
double g_lastPathTime = 0.0;
float g_dt = 0.0f;
// The fisheye's sharp middle while paused (fisheye.h kDetailFov).
int g_detailState = 0;       // 0 none, 1 capturing (camera at the detail FOV), 3 returning to 120, 2 in use
float g_detailCam[7] = {};   // eye, rot, fov the detail belongs to
double g_detailTime = -1.0, g_stableSince = 0.0;
std::wstring g_photoPath;    // a photo waiting for the sharp middle
// Skate's texts, prepared from the player's game (replay-text.txt); our short English otherwise.
std::map<std::string, std::string> g_textTable;
// Photos and saved replays (P2c, P3).
bool g_photoTaking = false;
double g_photoSince = 0.0;
std::atomic<int> g_saveState{0};  // 0 idle, 1 writing, 2 saved, 3 failed
std::vector<SavedInfo> g_reel;
int g_reelSel = 0;
Mode g_reelBack = Mode::Finished;
bool g_openReel = false;          // F10 "Saved replays": the replay opens on the list
DWORD g_openReelAt = 0;           // ... when the replay opens within 10 s of the request
bool g_exitAfterLoad = false;     // "Exit replay" from a saved replay: leave once the skater's area is back
DWORD g_graceUntil = 0;           // after a forced exit from a saved replay: invincible while the area streams in
bool g_graceInvincible = false;
bool g_loaded = false;            // a saved replay plays; the live recording waits in g_liveRec
Recording g_liveRec;
Edit g_liveEdit;
bool g_liveEditValid = false;
double g_liveTime = 0.0;
bool g_invincibleWas = false;
int g_loadStep = 0;               // Loading: 0 fade out, 1 scene loading
double g_loadUntil = 0.0;
float g_loadAt[3] = {};
Mode g_afterLoad = Mode::Timeline;
std::vector<std::pair<std::string, std::string>> g_help;  // pages: title, text
int g_helpPage = 0;
Mode g_helpBack = Mode::Timeline;
bool g_exitPending = false;  // Exit waits for the selection or Back buttons to be released
bool g_editWasDirty = false; // restore the original dirty state on camera cancel
bool g_camChanged = false;   // camera mode: something was set (Cancel asks only then)
float g_followHeading = 0.0f;  // the editor's Follow cameras: a steady heading (radians)
bool g_followHeadingSet = false;
int g_stickPrev = 0;         // Edit Menu / Finished: the left stick's last up/down step

// ---- look -----------------------------------------------------------------

int StrengthPercent(const Clip& clip) {
    return clip.filterStrength < 0 ? static_cast<int>(std::lround(g_filterStrength * 100.0f)) : clip.filterStrength;
}

void ApplyLook(int filter, bool fisheyeWanted, int shake, int strength) {
    if (!FilterModifier(filter)) filter = 0;
    if (filter != g_appliedFilter || (filter && strength != g_appliedStrength)) {
        if (filter != g_appliedFilter) {
            if (g_appliedFilter) Call<void>(CLEAR_TIMECYCLE_MODIFIER);
            if (filter) Call<void>(SET_TIMECYCLE_MODIFIER, FilterModifier(filter));
        }
        if (filter) Call<void>(SET_TIMECYCLE_MODIFIER_STRENGTH, strength / 100.0f);
        g_appliedFilter = filter;
        g_appliedStrength = strength;
        fisheye::SetFiltered(g_appliedFilter != 0);
        if (g_detailState) fisheye::DropDetail(), g_detailState = 0;
        g_stableSince = g_clock;
    }
    const bool fish = g_active && fisheyeWanted && g_fisheyeOk;
    if (fish != g_appliedFisheye) {
        fisheye::SetEnabled(fish);
        g_appliedFisheye = fish;
        if (g_detailState) fisheye::DropDetail(), g_detailState = 0;
        g_stableSince = g_clock;
    }
    if (shake != g_appliedShake && g_cam) {
        if (shake > 0) Call<void>(SHAKE_CAM, g_cam, "HAND_SHAKE", ShakeAmplitude(shake));
        else Call<void>(STOP_CAM_SHAKING, g_cam, 1);
        g_appliedShake = shake;
    }
}

void Log(const char* text) {
    if (g_log) g_log(text);
}

// ---- HUD (GTA's drawing, or the fisheye overlay's unbent bands) -------------

std::vector<fisheye::HudRect> g_rects;
std::vector<fisheye::HudText> g_texts;

void Rect(float x0, float y0, float x1, float y1, float r, float g, float b, float a) {
    if (x1 > x0 && y1 > y0) g_rects.push_back({x0, y0, x1, y1, r, g, b, a});
}
void Txt(const std::string& s, float x, float y, float scale, bool centre = false) { g_texts.push_back({s, x, y, scale, centre}); }

// Sends the items to the overlay while the fisheye bends the picture, else draws them with GTA.
void FlushHud() {
    if (!g_hudHidden && fisheye::Drawing()) {
        fisheye::SetHudItems(g_rects, g_texts, true);
    } else {
        fisheye::SetHudItems({}, {}, false);
        if (!g_hudHidden) {
            for (const auto& r : g_rects)
                Call<void>(DRAW_RECT, (r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2, r.x1 - r.x0, r.y1 - r.y0, static_cast<int>(r.r * 255),
                           static_cast<int>(r.g * 255), static_cast<int>(r.b * 255), static_cast<int>(r.a * 255), 0);
            for (const auto& t : g_texts) gta::Text(t.text.c_str(), t.x, t.y, t.scale, 255, 255, 255, 255, t.centre ? Align::Centre : Align::Left);
        }
    }
    g_rects.clear();
    g_texts.clear();
}

std::string Clock(double s) {
    char t[32];
    const int m = static_cast<int>(std::max(0.0, s) / 60.0);
    std::snprintf(t, sizeof(t), "%d:%04.1f", m, std::max(0.0, s) - m * 60.0);
    return t;
}

void Notice(const std::string& text) { g_notice = text, g_noticeUntil = g_clock + 3.0; }

std::string T(const char* id, const char* fallback) {
    const auto it = g_textTable.find(id);
    return it != g_textTable.end() ? it->second : fallback;
}

// Documents\GTAxSkate\<sub>\ (made when missing); empty when Documents is unknown.
std::wstring Folder(const wchar_t* sub) {
    wchar_t docs[MAX_PATH]{};
    if (FAILED(SHGetFolderPathW(nullptr, CSIDL_PERSONAL, nullptr, 0, docs))) return {};
    std::wstring dir = std::wstring(docs) + L"\\GTAxSkate\\";
    CreateDirectoryW(dir.c_str(), nullptr);
    dir += sub;
    dir += L"\\";
    CreateDirectoryW(dir.c_str(), nullptr);
    return dir;
}

std::wstring Stamp(const wchar_t* ext) {
    SYSTEMTIME t;
    GetLocalTime(&t);
    wchar_t name[64];
    swprintf(name, 64, L"GTAxSkate-%04u%02u%02u-%02u%02u%02u-%03u%s", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds, ext);
    return name;
}

// ---- presentation -----------------------------------------------------------

posemath::M ToM(const float* f) {
    posemath::M m;
    std::memcpy(m.r, f, sizeof(m.r));
    return m;
}

void Lerp3(const float* a, const float* b, float t, float* out) {
    for (int i = 0; i < 3; ++i) out[i] = a[i] + (b[i] - a[i]) * t;
}

// Angles in degrees by the shorter way round (179 to -179 passes 180, not 0).
void LerpAngles3(const float* a, const float* b, float t, float* out) {
    for (int i = 0; i < 3; ++i) out[i] = a[i] + std::remainder(b[i] - a[i], 360.0f) * t;
}

// The frame at the playhead, blended (translations lerped, rotations slerped).
void Present(int ped, const Recording::Sample& s) {
    const Frame& a = *s.a;
    const Frame& b = *s.b;
    const float t = s.t;
    float root[3];
    Lerp3(a.root, b.root, t, root);
    Call<void>(SET_ENTITY_COORDS_NO_OFFSET, ped, root[0], root[1], root[2], 0, 0, 0);
    const std::size_t n = a.bones.size() / 16;
    g_pose.resize(a.bones.size());
    for (std::size_t i = 0; i < n; ++i) {
        const posemath::M m = posemath::Blend(ToM(&a.bones[16 * i]), ToM(&b.bones[16 * i]), t);
        std::memcpy(&g_pose[16 * i], m.r, sizeof(m.r));
    }
    if (n) posehook::PublishPose(g_pose.data(), static_cast<int>(n));
    if (a.board) {
        SvBoardPose pose{};
        pose.size = sizeof(pose);
        pose.bone_count = 7;
        const posemath::M e = posemath::Blend(ToM(a.boardEntity), ToM(b.board ? b.boardEntity : a.boardEntity), t);
        std::memcpy(pose.entity, e.r, sizeof(e.r));
        for (int k = 0; k < 7; ++k) {
            const posemath::M w = posemath::Blend(ToM(a.boardWorld[k]), ToM(b.board ? b.boardWorld[k] : a.boardWorld[k]), t);
            std::memcpy(pose.world[k], w.r, sizeof(w.r));
        }
        // The same choice as live play: the shop's parts, else the native board.
        const bool shopParts = boardparts::Tick(true, pose);
        if (!shopParts) boardnative::Tick(true, pose);
        else boardnative::Tick(false, SvBoardPose{});
    } else {  // no board in this frame (carried, bailed away): none drawn
        boardnative::Tick(false, SvBoardPose{});
        boardparts::Tick(false, SvBoardPose{});
    }
}

struct Pose {
    float root[3], heading;  // heading: GTA's forward, radians
};
Pose PoseAt(const Recording::Sample& s) {
    const Frame& a = *s.a;
    const Frame& b = *s.b;
    Pose p{};
    float vel[3];
    Lerp3(a.root, b.root, s.t, p.root);
    Lerp3(a.velocity, b.velocity, s.t, vel);
    // GTA's forward: from the velocity, else Skate's heading + 180 (as the ped is turned).
    p.heading = std::hypot(vel[0], vel[1]) > 0.5f ? std::atan2(-vel[0], vel[1]) : (a.heading + 180.0f) * kDeg;
    return p;
}

void LookAt(const float* eye, const float* at, float* rot) {
    const float dx = at[0] - eye[0], dy = at[1] - eye[1], dz = at[2] - eye[2];
    rot[0] = std::atan2(dz, std::hypot(dx, dy)) / kDeg;
    rot[1] = 0.0f;
    rot[2] = std::atan2(-dx, dy) / kDeg;
}

// The first world, vehicle or object hit from `a` to `b` (the skater ignored).
bool Ray(const float* a, const float* b, float* hit, float* normal, int flags) {
    const int test = Call<int>(START_EXPENSIVE_SYNCHRONOUS_SHAPE_TEST_LOS_PROBE, a[0], a[1], a[2], b[0], b[1], b[2], flags, g_ped, 7);
    BOOL got = 0;
    Vector3 end{}, n{};
    Entity entity = 0;
    if (Call<int>(GET_SHAPE_TEST_RESULT, test, &got, &end, &n, &entity) != 2 || !got) return false;
    hit[0] = end.x, hit[1] = end.y, hit[2] = end.z;
    normal[0] = n.x, normal[1] = n.y, normal[2] = n.z;
    return true;
}

// `to` moved back onto the `from` side of anything between them: 0.3 m back
// along the line and 0.15 m off the surface (a grazing wall or the ground
// would otherwise still cut the picture open). Flags: 1 world, 2 vehicles, 16 objects.
bool Block(const float* from, float* to, int flags = 1 | 2 | 16) {
    float hit[3], n[3];
    if (!Ray(from, to, hit, n, flags)) return false;
    const float d[3] = {from[0] - hit[0], from[1] - hit[1], from[2] - hit[2]};
    const float len = std::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
    const float k = len > 1e-4f ? std::min(0.3f / len, 1.0f) : 0.0f;
    const float nl = std::sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]);
    for (int i = 0; i < 3; ++i) to[i] = hit[i] + d[i] * k + (nl > 0.5f ? n[i] / nl * 0.15f : 0.0f);
    return true;
}

// How a camera stays out of geometry: Skater = cameras around the skater (a
// wall or the ground between skater and eye pulls the eye in front of it);
// Path = placed cameras (Free, Tripod, Static, keys: the eye cannot pass
// through anything from where it was); None = what play showed.
enum class Clear { None, Skater, Path };

// Sets the replay camera. `aim`: the camera turns to it again after the eye moved.
void SetCam(float* eye, float* rot, float fov, Clear clear, const float* chest, const float* aim, int camKey) {
    bool moved = false;
    // A new camera or clip, or a jump in time, starts the path again (and from the skater).
    if (camKey != g_lastCamKey || std::fabs(g_head.time - g_lastPathTime) > 0.25) g_lastEyeSet = false, g_skaterLen = -1.0f;
    g_lastCamKey = camKey, g_lastPathTime = g_head.time;
    if (clear == Clear::Skater && chest) {
        // In front of a wall or the ground at once, back out eased (no flicker past
        // posts); cars passing do not push it (world and objects only).
        float d[3] = {eye[0] - chest[0], eye[1] - chest[1], eye[2] - chest[2]};
        const float want = std::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
        float clearEye[3] = {eye[0], eye[1], eye[2]};
        const bool hit = Block(chest, clearEye, 1 | 16);
        const float fx = clearEye[0] - chest[0], fy = clearEye[1] - chest[1], fz = clearEye[2] - chest[2];
        const float room = hit ? std::sqrt(fx * fx + fy * fy + fz * fz) : want;
        if (g_skaterLen < 0.0f || room < g_skaterLen) g_skaterLen = room;
        else g_skaterLen += (std::min(room, want) - g_skaterLen) * std::min(1.0f, g_dt * 3.0f);
        if (want > 1e-3f && g_skaterLen < want - 1e-3f) {
            if (hit && room <= g_skaterLen + 1e-3f) std::memcpy(eye, clearEye, sizeof(clearEye));  // the wall's own clearance
            else for (int i = 0; i < 3; ++i) eye[i] = chest[i] + d[i] / want * g_skaterLen;
            moved = true;
        }
    } else if (clear == Clear::Path) {
        if (g_lastEyeSet) moved = Block(g_lastEye, eye);
        else if (chest) moved = Block(chest, eye);  // a start inside a wall comes out on the skater's side
    }
    if (moved && aim) LookAt(eye, aim, rot);
    std::memcpy(g_lastEye, eye, sizeof(g_lastEye));
    g_lastEyeSet = true;
    // The fisheye's sharp middle: a still, paused camera is rendered once more
    // at the detail FOV; anything that moves drops it.
    fov = std::clamp(fov, 10.0f, 120.0f);
    const float cam[7] = {eye[0], eye[1], eye[2], rot[0], rot[1], rot[2], fov};
    bool still = true;
    for (int i = 0; i < 7; ++i) still = still && std::fabs(cam[i] - g_detailCam[i]) < 1e-3f;
    still = still && std::fabs(g_head.time - g_detailTime) < 1e-6 && !g_head.playing;
    if (!still) {
        std::memcpy(g_detailCam, cam, sizeof(cam));
        g_detailTime = g_head.time;
        g_stableSince = g_clock;
        if (g_detailState) fisheye::DropDetail(), g_detailState = 0;
    }
    // Only a truly still picture: no shake, no world moving, and the fisheye
    // really drawn this frame (filtered shots transfer high-frequency detail only).
    const bool fish = g_appliedFisheye && fisheye::Drawing() && fov >= 119.0f && g_appliedShake == 0 &&
                      g_worldScale <= 0.0f;
    if (!fish && g_detailState) fisheye::DropDetail(), g_detailState = 0;
    if (fish && still && g_detailState == 0 && fisheye::DetailIdle() && g_clock - g_stableSince > 0.3) fisheye::BeginDetail(), g_detailState = 1;
    // GTA says which field of view it rendered: the capture waits for the detail view,
    // the hold ends only once it is back at 120 degrees.
    const float rendered = Call<float>(GET_FINAL_RENDERED_CAM_FOV);
    if (g_detailState == 1 && std::fabs(rendered - fisheye::kDetailFov) < 0.5f) fisheye::ArmDetail();
    if (g_detailState == 1 && fisheye::DetailCaptured()) g_detailState = 3;
    if (g_detailState == 3 && std::fabs(rendered - fov) < 0.5f) fisheye::UseDetail(), g_detailState = 2;
    Call<void>(SET_CAM_COORD, g_cam, eye[0], eye[1], eye[2]);
    Call<void>(SET_CAM_ROT, g_cam, rot[0], rot[1], rot[2], 2);
    Call<void>(SET_CAM_FOV, g_cam, g_detailState == 1 ? fisheye::kDetailFov : fov);
}

float MmToFov(float mm) { return 2.0f * std::atan(12.0f / std::max(mm, 1.0f)) / kDeg; }
float FovToMm(float fov) { return 12.0f / std::tan(std::clamp(fov, 5.0f, 150.0f) * 0.5f * kDeg); }

// The editor's camera for a clip at the presented frame (Skate's camera kinds).
void AimEdited(const Recording::Sample& s, CameraKind kind, const CameraParams& c, int lens, float dt) {
    const Frame& a = *s.a;
    const Frame& b = *s.b;
    Pose p = PoseAt(s);
    // Follow: the heading of the skater's travel, kept while slow (no flip to the
    // body's heading, which is 180 degrees off riding fakie) and eased (no jumps).
    {
        float vel[3];
        Lerp3(a.velocity, b.velocity, s.t, vel);
        const bool moving = std::hypot(vel[0], vel[1]) > 0.5f;
        if (!g_followHeadingSet) g_followHeading = p.heading, g_followHeadingSet = true;
        else if (moving) g_followHeading += std::remainder(p.heading - g_followHeading, 6.2831853f) * std::min(1.0f, dt * 3.0f);
        p.heading = g_followHeading;
    }
    float eye[3], rot[3] = {}, fov = 50.0f;
    const float aim[3] = {p.root[0], p.root[1], p.root[2] + std::clamp(c.target, 0.0f, 1.0f) * 1.8f};
    switch (kind) {
    case CameraKind::Follow:
    case CameraKind::AdvFollow: {
        // Around the skater, yaw relative to where the skater goes (0: behind).
        const float ang = p.heading + c.yaw * kDeg;
        eye[0] = p.root[0] + std::sin(ang) * c.distance;
        eye[1] = p.root[1] - std::cos(ang) * c.distance;
        eye[2] = p.root[2] + c.height;
        LookAt(eye, aim, rot);
        break;
    }
    case CameraKind::Tripod:
    case CameraKind::AdvTripod:
        if (c.positioned) std::memcpy(eye, c.position, sizeof(eye));
        else Lerp3(a.camera, b.camera, s.t, eye);
        LookAt(eye, aim, rot);
        break;
    case CameraKind::Free:
    case CameraKind::AdvFree:
        if (c.positioned) std::memcpy(eye, c.position, sizeof(eye));
        else Lerp3(a.camera, b.camera, s.t, eye);
        rot[0] = c.pitch, rot[2] = c.yaw;
        break;
    default:  // Game Camera: what play showed
        Lerp3(a.camera, b.camera, s.t, eye);
        LerpAngles3(a.camera + 3, b.camera + 3, s.t, rot);
        fov = a.camera[6] + (b.camera[6] - a.camera[6]) * s.t;
        break;
    }
    fov = LensFov(lens, fov);
    if (c.zoomMm > 0.0f && lens != kLensFisheye) fov = MmToFov(c.zoomMm);
    const float body[3] = {p.root[0], p.root[1], p.root[2] + 1.0f};
    const bool around = kind == CameraKind::Follow || kind == CameraKind::AdvFollow;
    const bool tripod = kind == CameraKind::Tripod || kind == CameraKind::AdvTripod;
    const Clear clear = kind == CameraKind::Game ? Clear::None : around ? Clear::Skater : Clear::Path;
    SetCam(eye, rot, fov, clear, body, around || tripod ? aim : nullptr, 200 + static_cast<int>(kind) + 16 * g_edit.ClipAt(g_head.time));
}

// ---- the editor's HUD ----------------------------------------------------------

void KindColour(CameraKind k, float& r, float& g, float& b) {
    static const float c[][3] = {{0.6f, 0.6f, 0.6f}, {0.2f, 0.5f, 1.0f}, {0.3f, 0.8f, 0.3f}, {1.0f, 0.6f, 0.1f},
                                 {0.55f, 0.75f, 1.0f}, {0.65f, 1.0f, 0.65f}, {1.0f, 0.82f, 0.45f}};
    const auto i = std::min<std::size_t>(static_cast<std::size_t>(k), 6);
    r = c[i][0], g = c[i][1], b = c[i][2];
}

// Skate's timeline strip along the top: the whole recording, In/Out, clips by camera, markers, keys, playhead.
void Strip(int highlightClip, bool preview = false) {
    const double start = g_rec.Start(), len = std::max(g_rec.Length(), 1e-3);
    auto X = [&](double t) { return 0.05f + 0.9f * static_cast<float>(std::clamp((t - start) / len, 0.0, 1.0)); };
    Rect(0.045f, 0.028f, 0.955f, 0.082f, 0.0f, 0.0f, 0.0f, 0.55f);
    Rect(0.05f, 0.04f, 0.95f, 0.07f, 0.15f, 0.15f, 0.15f, 0.9f);  // outside In..Out
    if (preview) {
        Rect(0.05f, 0.04f, X(g_head.time), 0.07f, 0.7f, 0.57f, 0.16f, 0.9f);
    } else {
        for (int i = 0; i < static_cast<int>(g_edit.Clips().size()); ++i) {
            float r, g, b;
            KindColour(g_edit.ClipRef(i).camera, r, g, b);
            const float x0 = X(g_edit.ClipStart(i)), x1 = X(g_edit.ClipEnd(i));
            if (i == highlightClip) Rect(x0, 0.034f, x1, 0.076f, 1.0f, 1.0f, 1.0f, 0.9f);
            Rect(x0, 0.04f, x1, 0.07f, r, g, b, 0.95f);
            for (const Key& k : g_edit.ClipRef(i).keys) Rect(X(k.time) - 0.0015f, 0.07f, X(k.time) + 0.0015f, 0.078f, 1.0f, 0.45f, 0.1f, 1.0f);
        }
        for (double m : g_edit.Markers()) Rect(X(m) - 0.0012f, 0.036f, X(m) + 0.0012f, 0.074f, 1.0f, 1.0f, 1.0f, 1.0f);
        Rect(X(g_edit.In()) - 0.002f, 0.032f, X(g_edit.In()) + 0.002f, 0.078f, 1.0f, 0.85f, 0.1f, 1.0f);
        Rect(X(g_edit.Out()) - 0.002f, 0.032f, X(g_edit.Out()) + 0.002f, 0.078f, 1.0f, 0.85f, 0.1f, 1.0f);
    }
    Rect(X(g_head.time) - 0.0015f, 0.026f, X(g_head.time) + 0.0015f, 0.084f, 1.0f, 0.2f, 0.2f, 1.0f);
}

void EditorInfo(const char* title, bool preview) {
    std::string line = title;
    if (preview) {
        line += "   " + Clock(g_head.time - g_rec.Start()) + " / " + Clock(g_rec.Length());
    } else {
        line += "   Clip " + std::to_string(g_edit.ClipAt(g_head.time) + 1);
        line += " / " + std::to_string(g_edit.Clips().size());
        line += "   " + Clock(g_head.time - g_edit.In());
        line += "   Edited length " + Clock(g_edit.EditedLength());
    }
    Txt(line, 0.5f, 0.088f, 0.3f, true);
    if (g_clock < g_noticeUntil) Txt(g_notice, 0.5f, 0.12f, 0.36f, true);
}

void Keys(const char* pad, const char* keyboard, float y = 0.95f) {
    Txt(g_keyboard ? keyboard : pad, 0.5f, y, 0.3f, true);
}

// A GTA-style list panel (A53: GTA-native menus) with Skate's entries.
void Panel(const char* title, const std::vector<std::string>& items, int sel) {
    const float x0 = 0.06f, x1 = 0.40f, top = 0.25f, row = 0.034f;
    Rect(x0, top, x1, top + 0.05f, 0.05f, 0.05f, 0.05f, 0.92f);
    Txt(title, x0 + 0.012f, top + 0.01f, 0.5f);
    for (int i = 0; i < static_cast<int>(items.size()); ++i) {
        const float y = top + 0.05f + row * static_cast<float>(i);
        if (i == sel) Rect(x0, y, x1, y + row, 0.95f, 0.95f, 0.95f, 0.95f);
        else Rect(x0, y, x1, y + row, 0.0f, 0.0f, 0.0f, 0.75f);
        // GTA's selected row is white with dark text; ours keeps white text on a light row via a dark band.
        if (i == sel) Rect(x0 + 0.004f, y + 0.004f, x1 - 0.004f, y + row - 0.004f, 0.18f, 0.35f, 0.65f, 1.0f);
        Txt(items[static_cast<std::size_t>(i)], x0 + 0.012f, y + 0.006f, 0.34f);
    }
}

void PromptBox() {
    Rect(0.28f, 0.38f, 0.72f, 0.62f, 0.0f, 0.0f, 0.0f, 0.88f);
    Rect(0.28f, 0.38f, 0.72f, 0.385f, 1.0f, 0.85f, 0.1f, 1.0f);
    Txt(g_prompt.title, 0.5f, 0.395f, 0.45f, true);
    // Skate's texts: "^" is a line break; wrap long lines at about 70 characters.
    std::vector<std::string> lines;
    std::string cur;
    for (std::size_t i = 0; i <= g_prompt.text.size(); ++i) {
        const char ch = i < g_prompt.text.size() ? g_prompt.text[i] : '^';
        if (ch == '^' || (ch == ' ' && cur.size() > 66)) {
            if (!cur.empty()) lines.push_back(cur);
            cur.clear();
            continue;
        }
        cur += ch;
    }
    for (std::size_t i = 0; i < lines.size() && i < 5; ++i) Txt(lines[i], 0.5f, 0.44f + 0.03f * static_cast<float>(i), 0.32f, true);
    Txt(g_keyboard ? "Enter yes   Backspace no" : "A yes   B no", 0.5f, 0.585f, 0.32f, true);
}

// ---- help, photos, saved replays ------------------------------------------------

// Skate's help pages ("^" line breaks) for the timeline, the Edit Menu and each camera.
void OpenHelp(const std::vector<std::pair<const char*, const char*>>& ids, Mode back) {
    g_help.clear();
    for (const auto& [title, text] : ids) {
        const std::string fallbackTitle = "Help";
        g_help.emplace_back(T(title, fallbackTitle.c_str()), T(text, "Skate's help text is prepared at setup (Reading the replay editor texts)."));
    }
    g_helpPage = 0;
    g_helpBack = back;
    g_hudHidden = false;
    g_head.playing = false;
    g_mode = Mode::Help;
}

void HelpForTimeline(Mode back) {
    OpenHelp({{"ID_REPLAYEDITOR_HELP_TEXT_INSTANTREPLAY_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_INSTANTREPLAY1"},
              {"ID_REPLAYEDITOR_HELP_TEXT_INSTANTREPLAY_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_INSTANTREPLAY2"},
              {"ID_REPLAYEDITOR_HELP_TEXT_MAINTIMELINE_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_MAINTIMELINE1"},
              {"ID_REPLAYEDITOR_HELP_TEXT_MAINTIMELINE_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_MAINTIMELINE2"}},
             back);
}

void HelpForCamera(CameraKind k, Mode back) {
    std::vector<std::pair<const char*, const char*>> ids = {{"ID_REPLAYEDITOR_HELP_TEXT_EDITMENU_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_EDITMENU1"},
                                                            {"ID_REPLAYEDITOR_HELP_TEXT_EDITMENU_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_EDITMENU2"}};
    switch (k) {
    case CameraKind::Game: ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_GAMECAMERA_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_GAMECAMERA"}); break;
    case CameraKind::Follow: ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_FOLLOWCAMERA_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_FOLLOWCAMERA"}); break;
    case CameraKind::Tripod: ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_TRIPODCAMERA_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_TRIPODCAMERA"}); break;
    case CameraKind::Free: ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_FREECAMERA_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_FREECAMERA"}); break;
    case CameraKind::AdvFollow:
        ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_FOLLOWCAMERA_ADVANCED_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_FOLLOWCAMERA_ADVANCED1"});
        ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_FOLLOWCAMERA_ADVANCED_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_FOLLOWCAMERA_ADVANCED2"});
        break;
    case CameraKind::AdvTripod:
        ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_TRIPODCAMERA_ADVANCED_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_TRIPODCAMERA_ADVANCED1"});
        ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_TRIPODCAMERA_ADVANCED_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_TRIPODCAMERA_ADVANCED2"});
        break;
    default:
        ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_FREECAMERA_ADVANCED_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_FREECAMERA_ADVANCED1"});
        ids.push_back({"ID_REPLAYEDITOR_HELP_TEXT_FREECAMERA_ADVANCED_TITLE", "ID_REPLAYEDITOR_HELP_TEXT_FREECAMERA_ADVANCED2"});
        break;
    }
    OpenHelp(ids, back);
}

void HelpBox() {
    Rect(0.12f, 0.14f, 0.88f, 0.86f, 0.0f, 0.0f, 0.0f, 0.9f);
    Rect(0.12f, 0.14f, 0.88f, 0.145f, 1.0f, 0.85f, 0.1f, 1.0f);
    if (g_help.empty()) return;
    const auto& [title, text] = g_help[static_cast<std::size_t>(g_helpPage)];
    Txt(title, 0.5f, 0.155f, 0.45f, true);
    std::vector<std::string> lines;
    std::string cur;
    for (std::size_t i = 0; i <= text.size(); ++i) {
        const char ch = i < text.size() ? text[i] : '^';
        if (ch == '^' || (ch == ' ' && cur.size() > 92)) {
            lines.push_back(cur);
            cur.clear();
            continue;
        }
        cur += ch;
    }
    for (std::size_t i = 0; i < lines.size() && i < 18; ++i) Txt(lines[i], 0.14f, 0.2f + 0.034f * static_cast<float>(i), 0.3f);
    char foot[96];
    std::snprintf(foot, sizeof(foot), "%d / %d     %s", g_helpPage + 1, static_cast<int>(g_help.size()),
                  g_keyboard ? "Enter next page   Backspace close" : "A next page   B close");
    Txt(foot, 0.5f, 0.82f, 0.32f, true);
}

void TakePhoto() {
    const std::wstring dir = Folder(L"Photos");
    if (dir.empty()) return Notice("The photo could not be saved (no Documents folder).");
    g_photoPath = dir + Stamp(L".png");  // requested once the picture is ready (PollPhoto)
    g_photoTaking = true;
    g_photoSince = g_clock;
    g_head.playing = false;
}

void PollPhoto() {
    if (!g_photoTaking) return;
    if (!g_photoPath.empty()) {
        // Under the fisheye the photo waits (up to 2 s) for the sharp middle.
        if (g_appliedFisheye && !fisheye::DetailReady() && g_clock - g_photoSince < 2.0) return;
        fisheye::RequestPhoto(g_photoPath);
        g_photoPath.clear();
        g_photoSince = g_clock;
        return;
    }
    const int state = fisheye::PhotoState();
    if (state == 2) Notice(T("ID_REPLAY_EDITOR_SAVE_PHOTO_COMPLETE", "Your photograph has been saved successfully.") + " (Documents\\GTAxSkate\\Photos)");
    else if (state == 3 || g_clock - g_photoSince > 6.0) fisheye::CancelPhoto(), Notice("The photo could not be saved.");
    else return;
    fisheye::ResetPhoto();
    g_photoTaking = false;
}

// Skate's "Save Replay": In..Out (and half a second around it), the edit and the sound.
void SaveReplay(int ped) {
    if (g_saveState.load() == 1) return Notice("A replay is still being saved.");
    const std::wstring dir = Folder(L"Replays");
    if (dir.empty()) return Notice("The replay could not be saved (no Documents folder).");
    SavedReplay s;
    s.modelHash = static_cast<std::uint32_t>(Call<int>(GET_ENTITY_MODEL, ped));
    s.savedUnix = static_cast<std::uint64_t>(std::time(nullptr));
    const double from = std::max(g_rec.Start(), g_edit.In() - 0.5), to = std::min(g_rec.End(), g_edit.Out() + 0.5);
    double t0 = -1.0;
    for (std::size_t i = 0; i < g_rec.Size(); ++i) {
        const Frame& f = g_rec.Get(i);
        if (f.time < from - 1e-9 || f.time > to + 1e-9) continue;
        if (t0 < 0.0) t0 = f.time;
        s.frames.push_back(f);
        s.frames.back().time = f.time - t0;
    }
    if (s.frames.size() < 2) return Notice("Nothing to save.");
    s.in = g_edit.In() - t0, s.out = g_edit.Out() - t0;
    for (double m : g_edit.Markers()) s.markers.push_back(m - t0);
    s.clips = g_edit.Clips();
    for (Clip& c : s.clips)
        for (Key& k : c.keys) k.time -= t0;
    gtaaudio::ReplaySoundCopy(s.frames.front().gameMs, s.frames.back().gameMs + 50, s.pcm, s.soundStartMs);
    std::vector<std::uint8_t> bytes = Serialize(s);
    const std::wstring path = dir + Stamp(L".svreplay");
    g_saveState = 1;
    std::thread([bytes = std::move(bytes), path] {
        const std::wstring part = path + L".part";
        bool ok = false;
        {
            std::ofstream f(part, std::ios::binary);
            ok = f.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size())).good();
        }
        ok = ok && MoveFileExW(part.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING);
        if (!ok) DeleteFileW(part.c_str());
        g_saveState = ok ? 2 : 3;
    }).detach();
}

void PollSave() {
    const int state = g_saveState.load();
    if (state == 2) Notice("Save successful. Open it from Saved Replays (Documents\\GTAxSkate\\Replays).");
    if (state == 3) Notice("The replay could not be saved.");
    if (state >= 2) g_saveState = 0;
}

void LoadReel() {
    g_reel.clear();
    const std::wstring dir = Folder(L"Replays");
    WIN32_FIND_DATAW fd;
    HANDLE find = dir.empty() ? INVALID_HANDLE_VALUE : FindFirstFileW((dir + L"*.svreplay").c_str(), &fd);
    if (find == INVALID_HANDLE_VALUE) return;
    do {
        SavedInfo info;
        info.path = dir + fd.cFileName;
        std::ifstream f(info.path, std::ios::binary);
        std::vector<std::uint8_t> head(kInfoBytes);
        f.read(reinterpret_cast<char*>(head.data()), static_cast<std::streamsize>(head.size()));
        head.resize(static_cast<std::size_t>(f.gcount()));
        if (ReadInfo(head, info)) g_reel.push_back(info);
    } while (FindNextFileW(find, &fd));
    FindClose(find);
    std::sort(g_reel.begin(), g_reel.end(), [](const SavedInfo& a, const SavedInfo& b) { return a.savedUnix > b.savedUnix; });
    g_reelSel = std::clamp(g_reelSel, 0, std::max(0, static_cast<int>(g_reel.size()) - 1));
}

void OpenReel(Mode back) {
    if (!g_editValid) g_edit.Reset(g_rec.Start(), g_rec.End()), g_editValid = true;  // the editor's modes always have an edit
    LoadReel();
    g_reelBack = back;
    g_hudHidden = false;
    g_head.playing = false;
    g_mode = Mode::Reel;
}

// Fade, stream the world around `at`, then show `after` (the skater may be far away).
void BeginLoading(const float* at, Mode after) {
    std::memcpy(g_loadAt, at, sizeof(g_loadAt));
    g_afterLoad = after;
    g_loadStep = 0;
    g_loadUntil = g_clock + 8.0;
    Call<void>(DO_SCREEN_FADE_OUT, 400);
    g_mode = Mode::Loading;
}

void LoadingTick() {
    if (g_loadStep == 0 && (Call<BOOL>(IS_SCREEN_FADED_OUT) || g_clock > g_loadUntil - 6.0)) {
        Call<BOOL>(NEW_LOAD_SCENE_START_SPHERE, g_loadAt[0], g_loadAt[1], g_loadAt[2], 120.0f, 0);
        Call<void>(SET_FOCUS_POS_AND_VEL, g_loadAt[0], g_loadAt[1], g_loadAt[2], 0.0f, 0.0f, 0.0f);
        g_loadStep = 1;
    } else if (g_loadStep == 1 && (Call<BOOL>(IS_NEW_LOAD_SCENE_LOADED) || g_clock > g_loadUntil)) {
        Call<void>(NEW_LOAD_SCENE_STOP);
        Call<void>(CLEAR_FOCUS);
        Call<void>(DO_SCREEN_FADE_IN, 600);
        g_mode = g_afterLoad;
        g_input.Reset();
        if (std::exchange(g_exitAfterLoad, false)) g_exitPending = true;  // leaves on A/Enter's release (already let go: at once)
    }
}

void LeaveSaved(bool reload);

// Opens a saved replay in the editor (the live recording waits until it is closed).
void OpenSaved(int index) {
    if (index < 0 || index >= static_cast<int>(g_reel.size())) return;
    std::ifstream f(g_reel[static_cast<std::size_t>(index)].path, std::ios::binary);
    const std::vector<std::uint8_t> bytes((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
    SavedReplay s;
    std::string error;
    if (!Deserialize(bytes, s, error)) return Notice("This replay cannot be opened: " + error + ".");
    if (g_bones > 0 && s.frames.front().bones.size() != static_cast<std::size_t>(g_bones) * 16)
        return Notice("This replay was saved with another skater model.");
    if (g_loaded) LeaveSaved(false);
    std::swap(g_rec, g_liveRec);
    g_liveEdit = g_edit, g_liveEditValid = g_editValid, g_liveTime = g_head.time;
    const int bones = static_cast<int>(s.frames.front().bones.size() / 16);
    g_rec = Recording{};
    g_rec.Reserve(s.frames.back().time + 2.0, 60.0, bones);
    for (std::size_t i = 0; i < s.frames.size(); ++i) g_rec.Add(s.frames[i], i ? s.frames[i].time - s.frames[i - 1].time : 0.0);
    g_edit.Restore(g_rec.Start(), g_rec.End(), s.in, s.out, s.markers, s.clips);
    g_editValid = true;
    gtaaudio::ReplaySoundLoad(s.soundStartMs, std::move(s.pcm), std::pow(10.0f, g_soundGainDb / 20.0f));
    if (!g_loaded) {
        g_invincibleWas = Call<BOOL>(GET_PLAYER_INVINCIBLE, Call<int>(PLAYER_ID)) != 0;
        Call<void>(SET_PLAYER_INVINCIBLE, Call<int>(PLAYER_ID), 1);  // far away, a fall or the water never kills
    }
    g_loaded = true;
    g_head.time = g_edit.In();
    g_head.playing = false;
    char text[200];
    std::snprintf(text, sizeof(text), "replay: opened a saved replay (%u frames, %.1f s)", static_cast<unsigned>(s.frames.size()), g_rec.Length());
    Log(text);
    BeginLoading(s.frames.front().root, Mode::Timeline);
}

// Back to the live recording; `reload` streams the world back around it first.
void LeaveSaved(bool reload) {
    if (!g_loaded) return;
    std::swap(g_rec, g_liveRec);
    g_liveRec = Recording{};
    g_edit = g_liveEdit, g_editValid = g_liveEditValid;
    g_head.time = g_liveTime;
    g_head.playing = false;
    g_loaded = false;
    Call<void>(SET_PLAYER_INVINCIBLE, Call<int>(PLAYER_ID), g_invincibleWas ? 1 : 0);
    if (g_sound && g_rec.Size()) {
        const Recording::Sample first = g_rec.At(g_rec.Start());
        if (first.a) gtaaudio::ReplaySoundStart(first.a->gameMs, g_rec.Last().gameMs + 50, std::pow(10.0f, g_soundGainDb / 20.0f));
    }
    if (reload && g_rec.Size()) BeginLoading(g_rec.Last().root, Mode::Instant);
}

// ---- the editor's modes -----------------------------------------------------------

constexpr CameraKind kMenuCameras[] = {CameraKind::Game,      CameraKind::Follow,    CameraKind::Tripod, CameraKind::Free,
                                       CameraKind::AdvFollow, CameraKind::AdvTripod, CameraKind::AdvFree};
enum EditMenuRow { CameraRow, SpeedRow, ShakeRow, FilterRow, StrengthRow, LensRow, DoneRow, CameraHelpRow, EditMenuRows };
enum ActionRow { PlayRow, PhotoRow, SaveRow, ReelRow, HelpRow, ResetRow, CloseSavedRow, ExitRow, ActionRows };

void LeaveReplay() {
    if (g_loaded) {
        g_exitAfterLoad = true;
        LeaveSaved(true);
    } else {
        g_editValid = false;
        g_exitPending = true;
    }
}

void Ask(PromptAction action, const std::string& title, const std::string& text);

void RequestExit() {
    if (g_edit.Edited() && !g_loaded)
        Ask(PromptAction::ExitReplay, "Exit Replay", T("ID_REPLAY_EDITOR_QUIT", "Exit the Replay Editor? Unsaved edits will be lost."));
    else
        LeaveReplay();
}

void Ask(PromptAction action, const std::string& title, const std::string& text) {
    g_hudHidden = false;  // a question is never asked unseen
    g_head.playing = false;
    g_prompt.action = action;
    g_prompt.back = g_mode;
    g_prompt.title = title;
    g_prompt.text = text;
    g_prompt.time = g_head.time;
    g_mode = Mode::Prompt;
}

void EnterTimeline() {
    g_followHeadingSet = false;
    if (!g_editValid || !g_edit.Edited() || g_edit.In() < g_rec.Start() - 1e-6) {
        g_edit.Reset(g_rec.Start(), g_rec.End());
        g_editValid = true;
    }
    g_mode = Mode::Timeline;
    g_head.time = std::clamp(g_head.time, g_edit.In(), g_edit.Out());
    g_head.playing = false;
    g_head.reverse = false;
    g_input.Reset();
}

// The camera mode opens paused at the current time inside the clip.
// `changed`: the camera was just changed (the snapshot is already taken; Cancel asks).
void EnterCameraEdit(int clip, bool changed) {
    g_editClip = clip;
    if (!changed) g_clipBefore = g_edit.ClipRef(clip);
    g_camChanged = changed;
    g_followHeadingSet = false;
    Clip& c = g_edit.ClipRef(clip);
    const Vector3 at = Call<Vector3>(GET_CAM_COORD, g_cam);
    const Vector3 rot = Call<Vector3>(GET_CAM_ROT, g_cam, 2);
    auto place = [&](CameraParams& p) {
        if (p.positioned) return;
        p.position[0] = at.x, p.position[1] = at.y, p.position[2] = at.z;
        p.yaw = rot.z, p.pitch = rot.x;
        if (c.camera == CameraKind::Follow || c.camera == CameraKind::AdvFollow) p.yaw = 0.0f, p.pitch = 0.0f;
        p.positioned = true;
    };
    place(c.base);
    g_head.time = std::clamp(g_head.time, g_edit.ClipStart(clip), g_edit.ClipEnd(clip) - 1e-4);
    g_head.playing = false;
    g_work = IsAdvanced(c.camera) && !c.keys.empty() ? g_edit.Evaluate(g_head.time).p : c.base;
    place(g_work);
    g_workDirty = false;
    g_mode = Mode::CameraEdit;
    g_input.Reset();
}

// The params the camera mode shows now.
const CameraParams& Shown() { return IsAdvanced(g_edit.ClipRef(g_editClip).camera) ? g_work : g_edit.ClipRef(g_editClip).base; }

// Moves the camera being set (Skate: angle, distance, height, zoom, aim point; position for Tripod/Free).
void AdjustCamera(const PadIn& pad, const KeysIn& keys, const Recording::Sample& s, float dt) {
    Clip& clip = g_edit.ClipRef(g_editClip);
    CameraParams& p = IsAdvanced(clip.camera) ? g_work : clip.base;
    const bool follow = clip.camera == CameraKind::Follow || clip.camera == CameraKind::AdvFollow;
    const bool free = clip.camera == CameraKind::Free || clip.camera == CameraKind::AdvFree;
    const bool game = clip.camera == CameraKind::Game;
    const bool tracks = !free && !game;
    const bool read = keys.read;
    const auto k = [&](bool v) { return read && v ? 1.0f : 0.0f; };
    const float side = game ? 0.0f : std::clamp(StickAxis(pad.leftX) + k(keys.right) - k(keys.left), -1.0f, 1.0f);
    const float fwd = game ? 0.0f : std::clamp(StickAxis(pad.leftY) + k(keys.forward) - k(keys.back), -1.0f, 1.0f);
    const float rise = game ? 0.0f : std::clamp((pad.rightTrigger - pad.leftTrigger) / 255.0f + k(keys.up) - k(keys.down) + k(keys.faster) - k(keys.slower), -1.0f, 1.0f);
    const float lookYaw = follow || free ? -StickAxis(pad.rightX) * 120.0f * dt - (read ? keys.mouseX : 0.0f) * 8.0f : 0.0f;
    const float lookPitch = free ? StickAxis(pad.rightY) * 90.0f * dt - (read ? keys.mouseY : 0.0f) * 6.0f : 0.0f;
    const float zoom = clip.lens == kLensFisheye ? 0.0f :
        ((pad.buttons & 0x0008) ? 1.0f : 0.0f) - ((pad.buttons & 0x0004) ? 1.0f : 0.0f) + k(keys.zoomIn) - k(keys.zoomOut);
    const float aim = tracks ? ((pad.buttons & 0x0001) ? 1.0f : 0.0f) - ((pad.buttons & 0x0002) ? 1.0f : 0.0f) + k(keys.keyX) - k(keys.keyZ) : 0.0f;
    if (side == 0.0f && fwd == 0.0f && rise == 0.0f && lookYaw == 0.0f && lookPitch == 0.0f && zoom == 0.0f && aim == 0.0f) return;
    const Pose pose = PoseAt(s);
    switch (clip.camera) {
    case CameraKind::Follow:
    case CameraKind::AdvFollow:
        p.yaw += side * 90.0f * dt - lookYaw;
        p.distance = std::clamp(p.distance - fwd * 3.0f * dt, 1.0f, 15.0f);
        p.height = std::clamp(p.height + rise * 2.0f * dt, -1.0f, 8.0f);
        break;
    case CameraKind::Tripod:
    case CameraKind::AdvTripod:
    case CameraKind::Free:
    case CameraKind::AdvFree: {
        if (free) p.yaw += lookYaw, p.pitch = std::clamp(p.pitch + lookPitch, -85.0f, 85.0f);
        // Tripod moves relative to the line towards the skater; Free along its own look.
        const float face = free ? p.yaw * kDeg : std::atan2(-(pose.root[0] - p.position[0]), pose.root[1] - p.position[1]);
        const float before[3] = {p.position[0], p.position[1], p.position[2]};
        p.position[0] += (-std::sin(face) * fwd + std::cos(face) * side) * 6.0f * dt;
        p.position[1] += (std::cos(face) * fwd + std::sin(face) * side) * 6.0f * dt;
        p.position[2] += rise * 4.0f * dt;
        Block(before, p.position);  // never into a wall, the ground or a car
        // Skate: "the Free Camera can only be a certain distance" from the skater.
        const float dx = p.position[0] - pose.root[0], dy = p.position[1] - pose.root[1], dz = p.position[2] - pose.root[2];
        const float d = std::sqrt(dx * dx + dy * dy + dz * dz);
        if (d > 40.0f) for (int i = 0; i < 3; ++i) p.position[i] = pose.root[i] + (p.position[i] - pose.root[i]) * 40.0f / d;
        break;
    }
    default:
        break;
    }
    if (zoom != 0.0f) {
        if (p.zoomMm <= 0.0f) p.zoomMm = FovToMm(Call<float>(GET_CAM_FOV, g_cam));
        p.zoomMm = std::clamp(p.zoomMm * std::pow(2.0f, zoom * dt), 8.0f, 200.0f);
    }
    p.target = std::clamp(p.target + aim * 0.6f * dt, 0.0f, 1.0f);
    if (IsAdvanced(clip.camera)) g_workDirty = true;
    g_camChanged = true;
    g_edit.Touch();
}

// Edited playback: the playhead runs at each clip's speed; Timeline stops at Out, camera mode loops its clip.
void AdvanceEdited(float dt) {
    if (!g_head.playing) return;
    const double speed = g_edit.Evaluate(g_head.time).speed;
    g_head.time += dt * speed;
    if (g_mode == Mode::CameraEdit) {
        const double a = g_edit.ClipStart(g_editClip), b = g_edit.ClipEnd(g_editClip) - 1e-4;  // ClipAt(b) is still this clip
        if (g_head.time >= b) g_head.time = a;
    } else if (g_head.time >= g_edit.Out()) {
        g_head.time = g_edit.Out();
        g_head.playing = false;
    }
}

// The prompt's yes.
void PromptYes() {
    const Mode back = g_prompt.back;
    g_mode = back;
    switch (g_prompt.action) {
    case PromptAction::PlaceMarker:
        g_edit.PlaceMarker(g_prompt.time);
        break;
    case PromptAction::DeleteMarker:
        g_edit.DeleteMarker(g_prompt.time, 0.5, true);
        break;
    case PromptAction::DeleteKey:
        if (g_edit.DeleteKey(g_prompt.time, 0.25) == EditResult::Ok) g_camChanged = true, g_workDirty = false;
        break;
    case PromptAction::ResetTimeline:
        g_edit.Reset(g_rec.Start(), g_rec.End());
        g_head.time = g_edit.In();
        g_mode = Mode::Timeline;
        break;
    case PromptAction::ChangeCamera:
        g_edit.SetCamera(g_editClip, g_prompt.kind, true);
        EnterCameraEdit(g_editClip, true);
        break;
    case PromptAction::CancelCameraEdit:
        g_edit.ClipRef(g_editClip) = g_clipBefore;
        g_edit.SetEdited(g_editWasDirty);
        g_mode = Mode::EditMenu;
        g_menuSel = CameraRow;
        break;
    case PromptAction::SavePhoto:
        TakePhoto();
        break;
    case PromptAction::SaveReplay:
        g_mode = Mode::Timeline;
        SaveReplay(g_promptPed);
        break;
    case PromptAction::ExitReplay:
        LeaveReplay();
        break;
    case PromptAction::DeleteSaved:
        if (g_reelSel >= 0 && g_reelSel < static_cast<int>(g_reel.size())) DeleteFileW(g_reel[static_cast<std::size_t>(g_reelSel)].path.c_str());
        OpenReel(g_reelBack);
        break;
    default:
        break;
    }
    g_prompt.action = PromptAction::None;
    g_input.Reset();
}

// One replay frame. Exit is deferred until the triggering controls are released.
bool EditorTick(int ped, const PadIn& pad, const KeysIn& keys, float dt) {
    g_promptPed = ped;
    PollPhoto();
    PollSave();
    const Edges e = g_input.Poll(pad, keys);
    const auto P = [&](std::uint16_t bit) { return (e.pad & bit) != 0; };
    const Mode inputMode = g_mode;
    const bool listOrPrompt = g_mode == Mode::CameraMenu || g_mode == Mode::EditMenu || g_mode == Mode::Finished || g_mode == Mode::Prompt || g_mode == Mode::Help || g_mode == Mode::Reel;
    if (listOrPrompt) g_hudHidden = false;  // lists and questions are always shown
    else if (P(0x0080) || e.key.hide) g_hudHidden = !g_hudHidden;
    // The left stick steps through lists like the D-pad (one step per push).
    const int stick = StickAxis(pad.leftY) > 0.5f ? -1 : StickAxis(pad.leftY) < -0.5f ? 1 : 0;
    const int stickStep = stick != g_stickPrev ? stick : 0;
    g_stickPrev = stick;
    const int wheel = static_cast<int>(e.key.wheelDown) - static_cast<int>(e.key.wheelUp);
    const int vertical = (P(0x0001) || e.key.faster ? -1 : 0) +
                         (P(0x0002) || e.key.slower ? 1 : 0) + stickStep + wheel;
    const bool accept = P(0x1000) || e.key.enter;
    const bool back = P(0x2000) || e.key.backspace;
    if (!g_photoTaking) switch (g_mode) {
    case Mode::Instant: {
        if (back || P(0x0020) || e.key.tab) {
            RequestExit();
        } else if (accept || P(0x0040) || e.key.keyT) {
            EnterTimeline();
        } else if (P(0x4000) || e.key.keyX) {
            Ask(PromptAction::SavePhoto, T("ID_REPLAYEDITOR_TAKE_PHOTO", "Take Photo"),
                T("ID_REPLAY_EDITOR_POPUP_PHOTOSAVE", "Save Photo?"));
        } else if (P(0x8000) || e.key.keyU) {
            HelpForTimeline(Mode::Instant);
        } else if (P(0x0200) || e.key.play) {
            if (!g_head.playing && g_head.time >= g_rec.End()) g_head.time = g_rec.Start();
            g_head.playing = !g_head.playing;
        } else if (P(0x0004) || e.key.pageUp) {
            g_head.time = g_rec.Start();
            g_head.playing = false;
        } else if (P(0x0008) || e.key.pageDown) {
            g_head.time = g_rec.End();
            g_head.playing = false;
        } else {
            const float scrub = (static_cast<float>(pad.rightTrigger) - pad.leftTrigger) / 255.0f +
                                StickAxis(pad.leftX) + static_cast<float>(keys.read && keys.right) -
                                static_cast<float>(keys.read && keys.left);
            const int frames = static_cast<int>(e.key.stepForward) - static_cast<int>(e.key.stepBack) - wheel;
            if (scrub != 0.0f || frames) {
                g_head.playing = false;
                g_head.time = std::clamp(g_head.time + std::clamp(scrub, -1.0f, 1.0f) * dt * 2.0 +
                                             frames * kTickSeconds, g_rec.Start(), g_rec.End());
            }
        }
        break;
    }
    case Mode::Prompt:
        if (back) g_mode = g_prompt.back, g_prompt.action = PromptAction::None;
        else if (accept) PromptYes();
        break;
    case Mode::Timeline: {
        if (back) {
            if (g_loaded) LeaveSaved(true);
            else g_mode = Mode::Instant, g_head.playing = false;
            break;
        }
        if (P(0x0020) || e.key.tab) {
            g_menuSel = 0;
            g_head.playing = false;
            g_mode = Mode::Finished;
            break;
        }
        if (P(0x1000) || e.key.play) {
            if (!g_head.playing && (g_head.time < g_edit.In() || g_head.time >= g_edit.Out() - 1e-3)) g_head.time = g_edit.In();
            g_head.playing = !g_head.playing;
        }
        if (P(0x0100) || e.key.stepBack) g_head.time = g_edit.PrevStop(g_head.time), g_head.playing = false;
        if (P(0x0200) || e.key.stepForward) g_head.time = g_edit.NextStop(g_head.time), g_head.playing = false;
        if (P(0x0004) || e.key.pageUp) g_head.time = g_edit.In(), g_head.playing = false;
        if (P(0x0008) || e.key.pageDown) g_head.time = g_edit.Out(), g_head.playing = false;
        if (wheel) {
            g_head.time = std::clamp(g_head.time - wheel * kTickSeconds, g_rec.Start(), g_rec.End());
            g_head.playing = false;
        }
        if (P(0x0002) || e.key.keyN) {
            // As in Skate: checked first, then asked (A54).
            bool tooNear = g_head.time - g_edit.In() < Edit::kMinGap || g_edit.Out() - g_head.time < Edit::kMinGap;
            for (double m : g_edit.Markers()) tooNear = tooNear || std::fabs(m - g_head.time) < Edit::kMinGap;
            if (static_cast<int>(g_edit.Markers().size()) >= Edit::kMaxMarkers)
                Notice(T("ID_REPLAY_EDITOR_NO_MORE_MARKERS", "You have reached the maximum limit on the number of markers allowed."));
            else if (tooNear)
                Notice(T("ID_REPLAY_EDITOR_POPUP_PROXIMITY", "Too close to another Marker or In/Out Point!"));
            else
                Ask(PromptAction::PlaceMarker, "Place Clip Marker",
                    T("ID_REPLAY_EDITOR_POPUP_CLIPMARKER", "If you decide to remove this Clip Marker later, it will also remove all Key Markers in the clips on both sides.^^Are you sure you want to do this?"));
        }
        if (g_mode != inputMode) break;
        if (P(0x0001) || e.key.del) {
            if (g_edit.DeleteMarker(g_head.time, 0.5, false) == EditResult::NeedsConfirm)
                Ask(PromptAction::DeleteMarker, T("ID_REPLAY_EDITOR_DELETE_CLIP", "Delete Clip Marker"),
                    T("ID_REPLAY_EDITOR_DELETE_CLIP_DESC", "You are about to delete a clip marker. All key and camera information will be reset on either side of the marker. Are you sure you want to do this?"));
        }
        if (g_mode != inputMode) break;
        if (e.lt || e.key.keyI) {
            if (g_edit.SetIn(g_head.time) == EditResult::TooClose) Notice(T("ID_REPLAY_EDITOR_POPUP_PROXIMITY", "Too close to another Marker or In/Out Point!"));
        }
        if (e.rt || e.key.keyO) {
            if (g_edit.SetOut(g_head.time) == EditResult::TooClose) Notice(T("ID_REPLAY_EDITOR_POPUP_PROXIMITY", "Too close to another Marker or In/Out Point!"));
        }
        if (P(0x4000) || e.key.enter) {
            g_editClip = g_edit.ClipAt(g_head.time);
            g_menuSel = CameraRow;
            g_head.playing = false;
            g_mode = Mode::EditMenu;
        }
        if (g_mode != inputMode) break;
        if (!g_head.playing) {
            const float scrub = std::clamp(StickAxis(pad.leftX) + (keys.read && keys.right ? 1.0f : 0.0f) - (keys.read && keys.left ? 1.0f : 0.0f), -1.0f, 1.0f);
            g_head.time = std::clamp(g_head.time + scrub * dt * 2.0, g_rec.Start(), g_rec.End());
        }
        break;
    }
    case Mode::EditMenu: {
        if (back) {
            g_mode = Mode::Timeline;
            break;
        }
        Clip& clip = g_edit.ClipRef(g_editClip);
        g_menuSel = Cycle(g_menuSel, vertical, EditMenuRows);
        const int step = (P(0x0008) || e.key.right) ? 1 : (P(0x0004) || e.key.left) ? -1 : 0;
        if (step) {
            const Clip before = clip;
            switch (g_menuSel) {
            case SpeedRow: clip.speed = std::clamp(clip.speed + step, 0, 3); break;
            case ShakeRow: clip.shake = std::clamp(clip.shake + step, 0, 3); break;
            case FilterRow: clip.filter = Cycle(clip.filter, step, kFilterCount); break;
            case StrengthRow: if (clip.filter) clip.filterStrength = std::clamp(StrengthPercent(clip) + step * 10, 0, 100); break;
            case LensRow: clip.lens = Cycle(clip.lens, step, kLensCount); break;
            default: break;
            }
            if (clip.speed != before.speed || clip.shake != before.shake || clip.filter != before.filter ||
                clip.filterStrength != before.filterStrength || clip.lens != before.lens) g_edit.Touch();
        }
        if (accept && g_menuSel == CameraRow) {
            g_menuSel = static_cast<int>(clip.camera);
            g_mode = Mode::CameraMenu;
        } else if (accept && g_menuSel == DoneRow) {
            g_mode = Mode::Timeline;
        } else if (accept && g_menuSel == CameraHelpRow) {
            HelpForCamera(clip.camera, Mode::EditMenu);
        }
        break;
    }
    case Mode::CameraMenu: {
        if (back) {
            g_mode = Mode::EditMenu;
            g_menuSel = CameraRow;
            break;
        }
        g_menuSel = Cycle(g_menuSel, vertical, static_cast<int>(std::size(kMenuCameras)));
        if (!accept) break;
        const CameraKind kind = kMenuCameras[g_menuSel];
        g_clipBefore = g_edit.ClipRef(g_editClip);
        g_editWasDirty = g_edit.Edited();
        if (g_edit.SetCamera(g_editClip, kind, false) == EditResult::NeedsConfirm) {
            Ask(PromptAction::ChangeCamera, T("ID_REPLAY_EDITOR_REPLACE_CAMERA_TITLE", "Change Camera"),
                T("ID_REPLAY_EDITOR_REPLACE_CAMERA_DESC", "Changing camera will reset this clip's camera settings and keys. Continue?"));
            g_prompt.kind = kind;
        } else {
            EnterCameraEdit(g_editClip, g_clipBefore.camera != kind);
        }
        break;
    }
    case Mode::CameraEdit: {
        Clip& clip = g_edit.ClipRef(g_editClip);
        const bool adv = IsAdvanced(clip.camera);
        if (back) {
            if (g_camChanged) Ask(PromptAction::CancelCameraEdit, "Cancel", T("ID_REPLAY_EDITOR_POPUP_CANCEL", "Cancel this editing mode?"));
            else g_edit.ClipRef(g_editClip) = g_clipBefore, g_mode = Mode::EditMenu, g_menuSel = CameraRow;
            break;
        }
        if (P(0x0020) || e.key.tab) {
            g_mode = Mode::Timeline;
            g_head.playing = false;
            break;
        }
        if (P(0x1000) || e.key.play) g_head.playing = !g_head.playing, g_workDirty = false;
        // One frame back/on while paused; held, it repeats.
        const bool frameBack = (pad.buttons & 0x0100) || (keys.read && keys.stepBack);
        const bool on = (pad.buttons & 0x0200) || (keys.read && keys.stepForward);
        if (frameBack || on || wheel) {
            const bool first = P(0x0100) || P(0x0200) || e.key.stepBack || e.key.stepForward || wheel;
            g_stepHeld = first ? 0.0f : g_stepHeld + dt;
            if (first || g_stepHeld > 0.35f) {
                g_head.playing = false;
                g_head.time = std::clamp(g_head.time + (on || wheel < 0 ? kTickSeconds : -kTickSeconds), g_edit.ClipStart(g_editClip), g_edit.ClipEnd(g_editClip) - 1e-4);
                g_workDirty = false;
            }
        }
        if (adv && (P(0x4000) || e.key.keyK)) {
            CameraParams k = g_work;
            if (k.zoomMm <= 0.0f) k.zoomMm = clip.lens == kLensFisheye ? 24.0f : FovToMm(Call<float>(GET_CAM_FOV, g_cam));  // keys blend zooms smoothly
            g_edit.AddKey(g_head.time, k);
            g_workDirty = false, g_camChanged = true;
        }
        if (adv && (P(0x8000) || e.key.del)) {
            bool found = false;
            for (const Key& k : clip.keys) found = found || std::fabs(k.time - g_head.time) <= 0.25;
            if (found) Ask(PromptAction::DeleteKey, T("ID_REPLAY_EDITOR_DELETE_KEY", "Delete Key Marker"), T("ID_REPLAY_EDITOR_DELETE_KEY_DESC", "You are about to remove a key. Are you sure you want to do this?"));
        }
        if (g_mode != inputMode) break;
        if (adv && (P(0x0040) || e.key.keyJ)) {
            const double next = g_edit.NextKey(g_head.time);
            g_head.time = next > g_head.time ? next : (clip.keys.empty() ? g_head.time : clip.keys.front().time);
            g_head.playing = false, g_workDirty = false;
        }
        break;
    }
    case Mode::Finished: {
        if (back || e.key.tab) {
            g_mode = Mode::Timeline;
            break;
        }
        g_menuSel = Cycle(g_menuSel, vertical, g_loaded ? ActionRows : ActionRows - 1);
        if (accept) {
            const int action = !g_loaded && g_menuSel == CloseSavedRow ? ExitRow : g_menuSel;
            switch (action) {
            case PlayRow: g_head.time = g_edit.In(), g_head.playing = true, g_mode = Mode::Timeline; break;
            case PhotoRow: Ask(PromptAction::SavePhoto, T("ID_REPLAYEDITOR_TAKE_PHOTO", "Take Photo"), T("ID_REPLAY_EDITOR_POPUP_PHOTOSAVE", "Save Photo?")); break;
            case SaveRow: Ask(PromptAction::SaveReplay, T("ID_REPLAYEDITOR_BASIC_SAVE_REPLAY", "Save Replay"), T("ID_REPLAY_EDITOR_POPUP_REPLAYSAVE", "Save Replay?")); break;
            case ReelRow: OpenReel(Mode::Finished); break;
            case HelpRow: HelpForTimeline(Mode::Finished); break;
            case ResetRow: Ask(PromptAction::ResetTimeline, T("ID_REPLAY_EDITOR_RESET_TIMELINE", "Reset Timeline"), T("ID_REPLAY_EDITOR_POPUP_RESET", "This will remove all editing from this replay!^Continue?")); break;
            case CloseSavedRow: LeaveSaved(true); break;
            case ExitRow: RequestExit(); break;
            }
        }
        break;
    }
    case Mode::Help: {
        if (P(0x1000) || e.key.enter) {
            if (++g_helpPage >= static_cast<int>(g_help.size())) g_mode = g_helpBack;
        }
        if (P(0x2000) || e.key.backspace) g_mode = g_helpBack;
        break;
    }
    case Mode::Reel: {
        if (!g_reel.empty()) g_reelSel = Cycle(g_reelSel, vertical, static_cast<int>(g_reel.size()));
        if ((P(0x1000) || e.key.enter) && !g_reel.empty()) OpenSaved(g_reelSel);
        else if ((P(0x8000) || e.key.del) && !g_reel.empty())
            Ask(PromptAction::DeleteSaved, "Delete Replay", "This saved replay will be deleted from Documents\\GTAxSkate\\Replays.^Are you sure?");
        else if (P(0x2000) || e.key.backspace) {
            g_mode = g_reelBack;
        }
        break;
    }
    case Mode::Loading:
        LoadingTick();
        break;
    default:
        break;
    }
    const bool preview = g_mode == Mode::Instant ||
                         (g_mode == Mode::Prompt && g_prompt.back == Mode::Instant) ||
                         (g_mode == Mode::Help && g_helpBack == Mode::Instant) ||
                         (g_mode == Mode::Reel && g_reelBack == Mode::Instant);
    if (preview) g_head.Advance(dt, g_rec.Start(), g_rec.End());
    else AdvanceEdited(dt);
    const Recording::Sample s = g_rec.At(g_head.time);
    const int clipIndex = g_mode == Mode::CameraEdit ? g_editClip : g_edit.ClipAt(g_head.time);
    const Clip previewClip;
    const Clip& clip = preview ? previewClip : g_edit.ClipRef(clipIndex);
    ApplyLook(clip.filter, clip.lens == kLensFisheye, clip.shake, StrengthPercent(clip));
    if (s.a) {
        Present(ped, s);
        if (g_mode == Mode::CameraEdit) {
            if (inputMode == Mode::CameraEdit) AdjustCamera(pad, keys, s, dt);
            if (IsAdvanced(clip.camera) && (g_head.playing || !g_workDirty)) {
                if (!clip.keys.empty()) g_work = g_edit.Evaluate(g_head.time).p;
            }
            AimEdited(s, clip.camera, Shown(), clip.lens, dt);
        } else {
            const Edit::Frame f = preview ? Edit::Frame{} : g_edit.Evaluate(g_head.time);
            AimEdited(s, f.camera, f.p, clip.lens, dt);
        }
    }
    const float speed = preview ? 1.0f : g_edit.Evaluate(g_head.time).speed;
    gtaaudio::ReplaySoundAt(GameMsAt(s), g_head.playing ? speed : 0.0f);

    // HUD
    if (g_mode == Mode::Loading) {
        Txt("Loading...", 0.5f, 0.48f, 0.5f, true);
        FlushHud();
        return true;
    }
    if (g_photoTaking) {  // the picture without the editor on it
        g_rects.clear(), g_texts.clear();
        const bool hidden = g_hudHidden;
        g_hudHidden = true;
        FlushHud();
        g_hudHidden = hidden;
        return true;
    }
    Strip(g_mode == Mode::CameraEdit ? g_editClip : -1, preview);
    EditorInfo(preview ? "Instant Replay" : g_mode == Mode::CameraEdit ? CameraKindName(clip.camera) : "Main Timeline", preview);
    if (g_mode == Mode::Instant) {
        Keys("RB Play/Pause   LT/RT Scrub   D-pad Left/Right Start/End   A Edit Timeline",
             "Space Play/Pause   A/D or Wheel Scrub   PgUp/PgDn Start/End   Enter Edit Timeline", 0.91f);
        Keys("X Photo   Y Help   R3 Hide   B Exit", "X Photo   U Help   H Hide   Backspace Exit");
    } else if (g_mode == Mode::Timeline) {
        Keys("A Play/Pause   Left Stick Scrub   LT/RT In/Out   LB/RB Jump to Marker",
             "Space Play/Pause   A/D or Wheel Scrub   I/O In/Out   ,/. Jump to Marker", 0.91f);
        Keys("D-pad Down/Up Add/Delete Marker   X Edit Clip   Back Actions   R3 Hide   B Back",
             "N/Delete Add/Delete Marker   Enter Edit Clip   Tab Actions   H Hide   Backspace Back");
    } else if (g_mode == Mode::CameraEdit) {
        const CameraParams& camera = Shown();
        const bool fish = clip.lens == kLensFisheye;
        if (fish || camera.zoomMm <= 0.0f) Txt(LensName(clip.lens), 0.5f, 0.15f, 0.3f, true);
        else Txt("Zoom " + std::to_string(static_cast<int>(std::lround(camera.zoomMm))) + " mm", 0.5f, 0.15f, 0.3f, true);
        std::string padKeys = "LB/RB Step Frame";
        std::string keyboardKeys = ",/. or Wheel Step Frame";
        if (!fish) {
            padKeys += "   D-pad Left/Right Zoom";
            keyboardKeys += "   [ / ] Zoom";
        }
        if (clip.camera != CameraKind::Game) {
            const bool free = clip.camera == CameraKind::Free || clip.camera == CameraKind::AdvFree;
            padKeys += "   Left Stick Move   LT/RT Height";
            keyboardKeys += "   WASD Move   Q/E Height";
            if (free) {
                padKeys += "   Right Stick Look";
                keyboardKeys += "   Mouse Look";
            } else {
                padKeys += "   D-pad Up/Down Aim";
                keyboardKeys += "   Z/X Aim";
            }
        }
        Keys(padKeys.c_str(), keyboardKeys.c_str(), 0.87f);
        Keys("A Play/Pause   Back Commit   B Cancel   R3 Hide", "Space Play/Pause   Tab Commit   Backspace Cancel   H Hide", 0.91f);
        if (IsAdvanced(clip.camera)) Keys("X Set Key   Y Delete Key   L3 Next Key", "K Set Key   Delete Key   J Next Key");
    } else if (g_mode == Mode::EditMenu) {
        static const char* speeds[] = {"1/4x", "1/2x", "1x", "2x"};
        static const char* shakes[] = {"Off", "Low", "Medium", "High"};
        const std::string strength = clip.filter ? std::to_string(StrengthPercent(clip)) + "%" : "Off";
        const bool zoomOverride = clip.lens != kLensFisheye &&
            (clip.base.zoomMm > 0.0f || std::any_of(clip.keys.begin(), clip.keys.end(), [](const Key& key) { return key.p.zoomMm > 0.0f; }));
        const std::string lens = std::string("Lens   < ") + LensName(clip.lens) + " >" +
                                 (zoomOverride ? "   (camera zoom set)" : "");
        Panel(("Edit Clip " + std::to_string(g_editClip + 1)).c_str(),
              {std::string("Camera   ") + CameraKindName(clip.camera),
               std::string("Speed   < ") + speeds[std::clamp(clip.speed, 0, 3)] + " >",
               std::string("Shake   < ") + shakes[std::clamp(clip.shake, 0, 3)] + " >",
               std::string("Filter   < ") + FilterName(clip.filter) + " >",
               "Filter Strength   < " + strength + " >",
               lens,
               "Done", T("ID_REPLAYEDITOR_HELP", "Help")}, g_menuSel);
        Keys("D-pad Select / Adjust   A Open   B Back", "Arrows Select / Adjust   Enter Open   Backspace Back");
    } else if (g_mode == Mode::CameraMenu) {
        std::vector<std::string> items;
        for (CameraKind kind : kMenuCameras) items.emplace_back(CameraKindName(kind));
        Panel("Camera", items, g_menuSel);
        Keys("A Select   B Back", "Enter Select   Backspace Back");
    } else if (g_mode == Mode::Finished) {
        std::vector<std::string> items = {"Play from In Point", T("ID_REPLAYEDITOR_TAKE_PHOTO", "Take Photo"),
            T("ID_REPLAYEDITOR_BASIC_SAVE_REPLAY", "Save Replay"), "Saved Replays", T("ID_REPLAYEDITOR_HELP", "Help"),
            T("ID_REPLAY_EDITOR_RESET_TIMELINE", "Reset Timeline")};
        if (g_loaded) items.emplace_back("Close Saved Replay");
        items.emplace_back("Exit Replay");
        Panel("Replay Actions", items, g_menuSel);
        Keys("A Select   B Back", "Enter Select   Backspace Back");
    } else if (g_mode == Mode::Help) {
        HelpBox();
    } else if (g_mode == Mode::Reel) {
        std::vector<std::string> items;
        for (const SavedInfo& r : g_reel) {
            char line[96];
            std::tm tm{};
            const std::time_t t = static_cast<std::time_t>(r.savedUnix);
            localtime_s(&tm, &t);
            std::snprintf(line, sizeof(line), "%04d-%02d-%02d %02d:%02d   %s", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min,
                          Clock(r.length).c_str());
            items.push_back(line);
        }
        if (items.empty()) items.push_back("No saved replays yet (Finished > Save Replay)");
        Panel("Saved Replays", items, g_reel.empty() ? -1 : g_reelSel);
        Keys("A open   Y delete   B back", "Enter open   Delete delete   Backspace back");
    } else if (g_mode == Mode::Prompt) {
        PromptBox();
    }
    FlushHud();
    g_applyCalls += posehook::TakeApplyTime().calls;
    return true;
}

}  // namespace

void Configure(bool enabled, double seconds, float worldTimeScale, void (*log)(const char*)) {
    g_log = log;
    g_enabled = enabled;
    g_seconds = std::clamp(seconds, 10.0, 60.0);
    g_worldScale = std::clamp(worldTimeScale, 0.0f, 1.0f);
    g_bones = -1;  // slots made again for the new window
}

bool Enabled() { return g_enabled; }

void ConfigureLook(bool sound, float soundGainDb, bool fisheye, float filterStrength) {
    g_sound = sound;
    g_soundGainDb = std::clamp(soundGainDb, -30.0f, 6.0f);
    g_fisheyeOk = fisheye;
    g_filterStrength = std::isfinite(filterStrength) ? std::clamp(filterStrength, 0.0f, 1.0f) : 0.85f;
}

void Clear() {
    g_rec.Clear();
    g_lastTick = 0;
    g_editValid = false;
}

void Record(const SvOutput& o, const float* placed, const float* bones, int count, const SvBoardPose* board) {
    if (g_graceUntil && !g_active && static_cast<int>(GetTickCount() - g_graceUntil) >= 0) {  // the forced exit's grace is over
        g_graceUntil = 0;
        Call<void>(NEW_LOAD_SCENE_STOP);
        Call<void>(SET_PLAYER_INVINCIBLE, Call<int>(PLAYER_ID), g_graceInvincible ? 1 : 0);
    }
    if (!g_enabled || g_active || o.tick == g_lastTick || count <= 0 || !bones) return;
    if (count != g_bones) {
        g_rec.Reserve(g_seconds, 60.0, count);
        g_bones = count;
        g_lastTick = 0;
        g_editValid = false;
    }
    Frame& f = g_scratch;
    const double dt = g_lastTick && o.tick > g_lastTick ? static_cast<double>(o.tick - g_lastTick) * kTickSeconds : 0.0;
    // Where play put the ped (the skater root plus PedZOffset), else the root.
    if (placed) std::memcpy(f.root, placed, sizeof(f.root));
    else f.root[0] = o.skater_position.x, f.root[1] = o.skater_position.y, f.root[2] = o.skater_position.z;
    f.cut = false;
    if (g_rec.Size()) {
        const Frame& last = g_rec.Last();
        const float dx = f.root[0] - last.root[0], dy = f.root[1] - last.root[1], dz = f.root[2] - last.root[2];
        f.cut = dx * dx + dy * dy + dz * dz > 20.0f * 20.0f;
    }
    f.heading = o.skater_heading_degrees;
    f.velocity[0] = o.velocity.x, f.velocity[1] = o.velocity.y, f.velocity[2] = o.velocity.z;
    const Vector3 cam = Call<Vector3>(GET_FINAL_RENDERED_CAM_COORD);
    const Vector3 rot = Call<Vector3>(GET_FINAL_RENDERED_CAM_ROT, 2);
    f.camera[0] = cam.x, f.camera[1] = cam.y, f.camera[2] = cam.z;
    f.camera[3] = rot.x, f.camera[4] = rot.y, f.camera[5] = rot.z;
    f.camera[6] = Call<float>(GET_FINAL_RENDERED_CAM_FOV);
    f.gameMs = static_cast<std::uint32_t>(Call<int>(GET_GAME_TIMER));
    f.board = board != nullptr;
    if (board) {
        std::memcpy(f.boardEntity, board->entity, sizeof(f.boardEntity));
        std::memcpy(f.boardWorld, board->world, sizeof(f.boardWorld));
    }
    f.bones.assign(bones, bones + static_cast<std::size_t>(count) * 16);
    g_rec.Add(f, dt);
    g_lastTick = o.tick;
}

bool Ready() { return g_enabled && !g_active && g_rec.Length() >= 1.0; }

bool Active() { return g_active; }

bool Enter(int ped) {
    if (!Ready()) return false;
    g_active = true;
    g_mode = Mode::Instant;
    g_clock = 0.0;
    g_exitPending = false;
    g_head = Playhead{};
    g_head.time = std::max(g_rec.Start(), g_rec.End() - 10.0);  // the last ten seconds first
    const Frame& last = g_rec.Last();
    g_hudHidden = false;
    g_notice.clear();
    g_noticeUntil = 0.0;
    if (!g_editValid || g_edit.In() < g_rec.Start() - 1e-6) {
        g_edit.Reset(g_rec.Start(), g_rec.End());
        g_editValid = true;
    }
    g_cam = Call<int>(CREATE_CAM, "DEFAULT_SCRIPTED_CAMERA", 0);
    Call<void>(SET_CAM_NEAR_CLIP, g_cam, 0.05f);  // close walls are not cut open
    g_lastEyeSet = false;
    g_detailState = 0;
    g_ped = ped;
    Call<void>(SET_CAM_ACTIVE, g_cam, 1);
    Call<void>(RENDER_SCRIPT_CAMS, 1, 0, 0, 1, 0, 0);
    Call<void>(SET_TIME_SCALE, g_worldScale);
    // The recorded sound first (it stops Skate's recording), then Skate's live sound pauses.
    bool sound = false;
    if (g_sound) {
        const Recording::Sample first = g_rec.At(g_rec.Start());
        sound = first.a && gtaaudio::ReplaySoundStart(first.a->gameMs, last.gameMs + 50, std::pow(10.0f, g_soundGainDb / 20.0f));
    }
    gtaaudio::SetReplayPaused(true);
    g_appliedFilter = 0, g_appliedFisheye = false, g_appliedShake = 0;
    fisheye::SetFiltered(false);
    g_input.Reset();
    posehook::TakeApplyTime();
    g_applyCalls = 0;
    (void)ped;
    if (std::exchange(g_openReel, false) && GetTickCount() - g_openReelAt < 10000) OpenReel(Mode::Instant);
    char text[200];
    std::snprintf(text, sizeof(text), "replay: entered (%.1f s recorded, world time scale %.2f, sound %s)",
                  g_rec.Length(), g_worldScale, !g_sound ? "off" : sound ? "on" : "none recorded");
    Log(text);
    return true;
}

bool Tick(int ped, const PadIn& pad, const KeysIn& keys, float dt) {
    if (!g_active) return false;
    g_clock += dt;
    g_dt = dt;
    g_ped = ped;
    // GTA's controls are taken by the caller as in skating (DisableSkateControls),
    // never all at once: that would stop GTA's own recorder.
    Call<void>(HIDE_HUD_AND_RADAR_THIS_FRAME);
    // D-pad down is GTA's character wheel; the replay uses it.
    Call<void>(DISABLE_CONTROL_ACTION, 0, 19, 1);
    if (g_exitPending) {
        const bool held = (pad.buttons & (0x1000 | 0x2000 | 0x0020)) || !keys.read || keys.enter || keys.leave;
        if (!held) return false;  // leaves on the release: the press never reaches skating
        FlushHud();
        return true;
    }
    return EditorTick(ped, pad, keys, dt);
}

void Exit() {
    if (!g_active) return;
    g_exitAfterLoad = false;
    if (g_mode == Mode::Loading) {
        Call<void>(NEW_LOAD_SCENE_STOP);
        Call<void>(CLEAR_FOCUS);
    }
    if (g_loaded) {
        // Forced out of a saved replay (pause, death of the session, F6): the skater's
        // area streams in behind a fade while the player stays invincible for a moment.
        const bool was = g_invincibleWas;
        LeaveSaved(false);
        if (g_rec.Size()) {
            const float* at = g_rec.Last().root;
            Call<BOOL>(NEW_LOAD_SCENE_START_SPHERE, at[0], at[1], at[2], 120.0f, 0);
            Call<void>(DO_SCREEN_FADE_OUT, 0);
        }
        Call<void>(SET_PLAYER_INVINCIBLE, Call<int>(PLAYER_ID), 1);
        g_graceInvincible = was;
        g_graceUntil = GetTickCount() + 4000;
    }
    if (Call<BOOL>(IS_SCREEN_FADED_OUT) || g_mode == Mode::Loading || g_graceUntil) Call<void>(DO_SCREEN_FADE_IN, 1500);
    if (g_photoTaking) fisheye::CancelPhoto(), fisheye::ResetPhoto(), g_photoTaking = false, g_photoPath.clear();
    if (g_detailState) fisheye::DropDetail(), g_detailState = 0;
    if (g_appliedShake && g_cam) Call<void>(STOP_CAM_SHAKING, g_cam, 1);
    if (g_appliedFilter) Call<void>(CLEAR_TIMECYCLE_MODIFIER);
    g_appliedFilter = 0, g_appliedShake = 0, g_appliedFisheye = false;
    fisheye::SetFiltered(false);
    g_active = false;
    g_mode = Mode::Instant;
    Call<void>(SET_TIME_SCALE, 1.0f);
    if (g_cam) {
        Call<void>(SET_CAM_ACTIVE, g_cam, 0);
        Call<void>(DESTROY_CAM, g_cam, 0);
        g_cam = 0;
    }
    gtaaudio::ReplaySoundStop();
    gtaaudio::SetReplayPaused(false);
    fisheye::SetEnabled(false);
    fisheye::SetHudItems({}, {}, false);
    char text[160];
    std::snprintf(text, sizeof(text), "replay: left (pose writes during the replay: %u)", g_applyCalls);
    Log(text);
}

void SetKeyboard(bool keyboard) { g_keyboard = keyboard; }

void OpenSavedReplaysOnEnter() { g_openReel = true, g_openReelAt = GetTickCount(); }

void LoadTexts(const std::wstring& path) {
    g_textTable.clear();
    std::ifstream f(path, std::ios::binary);
    std::string line;
    while (std::getline(f, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        const std::size_t tab = line.find('\t');
        if (tab != std::string::npos) g_textTable[line.substr(0, tab)] = line.substr(tab + 1);
    }
    char text[160];
    std::snprintf(text, sizeof(text), "replay: %u of Skate's editor texts loaded", static_cast<unsigned>(g_textTable.size()));
    Log(text);
}

}  // namespace replay
