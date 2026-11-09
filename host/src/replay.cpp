#include "replay.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <utility>

#include "board_native.h"
#include "board_parts.h"
#include "fisheye.h"
#include "gta_audio.h"
#include "natives.h"
#include "pose_hook.h"
#include "pose_math.h"
#include "replay_logic.h"

namespace replay {
namespace {
using namespace gta;

constexpr double kTickSeconds = 1.0 / 60.0;  // Skate's fixed step
constexpr float kFilmerFov = 100.0f;         // the filmer's fisheye

enum Camera { Recorded, Follow, Tripod, Free, Filmer, Orbit, Static, kCameras };
constexpr const char* kCameraNames[kCameras] = {"Recorded", "Follow", "Tripod", "Free", "Filmer", "Orbit", "Static"};

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
int g_camera = Recorded;
int g_cam = 0;               // the replay's own camera
float g_eye[3] = {}, g_look[3] = {};  // follow/free/tripod state
float g_yaw = 0.0f, g_pitch = 0.0f;
Input g_input;               // both devices; the first frame takes what is held as it is
unsigned g_applyCalls = 0;
// Look and sound (design 2026-11-09-replay-look-sound). Filter and lens are
// kept for the session.
bool g_sound = true, g_fisheyeOk = true;
float g_soundGainDb = -4.0f, g_filterStrength = 0.85f;
int g_filter = 0, g_lens = kLensAuto;
bool g_filterSet = false;    // the replay set a timecycle modifier (and clears it)
float g_height = 0.0f;       // Follow, Tripod, Filmer: metres above their own height
float g_orbitYaw = 0.0f, g_orbitDist = 4.0f, g_orbitHeight = 1.2f;
float g_staticRot[3] = {};
bool g_hudHidden = false;    // H / right stick click
float g_zoom = 1.0f;         // [ and ]: a factor on the field of view

bool ArrowsMove() { return g_camera == Free || g_camera == Orbit; }

void ApplyFilter() {
    if (g_filterSet) Call<void>(CLEAR_TIMECYCLE_MODIFIER), g_filterSet = false;
    if (const char* mod = FilterModifier(g_filter)) {
        Call<void>(SET_TIMECYCLE_MODIFIER, mod);
        Call<void>(SET_TIMECYCLE_MODIFIER_STRENGTH, std::clamp(g_filterStrength, 0.0f, 1.0f));
        g_filterSet = true;
    }
}

void ApplyLens() { fisheye::SetEnabled(g_active && g_lens == kLensFisheye && g_fisheyeOk); }

void Log(const char* text) {
    if (g_log) g_log(text);
}

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

// Camera position/rotation/fov for the presented frame.
void Aim(const Recording::Sample& s, float dt, const Command& in) {
    const Frame& a = *s.a;
    const Frame& b = *s.b;
    float root[3], vel[3];
    Lerp3(a.root, b.root, s.t, root);
    Lerp3(a.velocity, b.velocity, s.t, vel);
    float eye[3], rot[3] = {}, fov = 60.0f;
    const float speed = std::hypot(vel[0], vel[1]);
    // GTA's forward: from the velocity, else Skate's heading + 180 (as the ped is turned).
    const float heading = speed > 0.5f ? std::atan2(-vel[0], vel[1]) : (a.heading + 180.0f) * 0.01745329f;
    switch (g_camera) {
    case Recorded:
        Lerp3(a.camera, b.camera, s.t, eye);
        LerpAngles3(a.camera + 3, b.camera + 3, s.t, rot);
        fov = a.camera[6] + (b.camera[6] - a.camera[6]) * s.t;
        break;
    case Follow: {
        const float want[3] = {root[0] + std::sin(heading) * 4.0f, root[1] - std::cos(heading) * 4.0f, root[2] + 1.8f + g_height};
        const float k = std::min(1.0f, dt * 4.0f);
        for (int i = 0; i < 3; ++i) g_eye[i] += (want[i] - g_eye[i]) * k;
        std::memcpy(eye, g_eye, sizeof(eye));
        break;
    }
    case Tripod:
        std::memcpy(eye, g_eye, sizeof(eye));
        eye[2] += g_height;
        break;
    case Orbit: {
        // Around the skater: look turns it, forward/back the distance, up/down the height.
        g_orbitYaw += in.lookYaw;
        g_orbitDist = std::clamp(g_orbitDist - in.move[1] * 4.0f * dt, 1.5f, 12.0f);
        g_orbitHeight = std::clamp(g_orbitHeight + in.move[2] * 3.0f * dt, -1.0f, 8.0f);
        const float yaw = g_orbitYaw * 0.01745329f;
        eye[0] = root[0] + std::sin(yaw) * g_orbitDist;
        eye[1] = root[1] - std::cos(yaw) * g_orbitDist;
        eye[2] = root[2] + g_orbitHeight;
        break;
    }
    case Static:
        std::memcpy(eye, g_eye, sizeof(eye));
        std::memcpy(rot, g_staticRot, sizeof(rot));
        break;
    case Free: {
        g_yaw += in.lookYaw;
        g_pitch = std::clamp(g_pitch + in.lookPitch, -85.0f, 85.0f);
        const float yaw = g_yaw * 0.01745329f;
        const float fwd = in.move[1] * 8.0f * dt, side = in.move[0] * 8.0f * dt;
        const float up = in.move[2] * 6.0f * dt;
        g_eye[0] += -std::sin(yaw) * fwd + std::cos(yaw) * side;
        g_eye[1] += std::cos(yaw) * fwd + std::sin(yaw) * side;
        g_eye[2] += up;
        std::memcpy(eye, g_eye, sizeof(eye));
        rot[0] = g_pitch, rot[2] = g_yaw;
        fov = 60.0f;
        break;
    }
    default: {  // Filmer: low behind the board, wide lens
        eye[0] = root[0] + std::sin(heading) * 1.6f;
        eye[1] = root[1] - std::cos(heading) * 1.6f;
        eye[2] = root[2] - 0.5f + g_height;
        fov = kFilmerFov;
        break;
    }
    }
    if (g_camera != Recorded && g_camera != Free && g_camera != Static) {
        // Turn to the skater's chest.
        const float dx = root[0] - eye[0], dy = root[1] - eye[1], dz = root[2] + 0.4f - eye[2];
        rot[0] = std::atan2(dz, std::hypot(dx, dy)) * 57.29578f;
        rot[2] = std::atan2(-dx, dy) * 57.29578f;
        fov = g_camera == Filmer ? kFilmerFov : 50.0f;
    }
    if (g_camera == Static) fov = 50.0f;
    fov = LensFov(g_lens, fov);
    if (g_lens != kLensFisheye) fov *= g_zoom;  // the fisheye keeps its 120-degree source
    Call<void>(SET_CAM_COORD, g_cam, eye[0], eye[1], eye[2]);
    Call<void>(SET_CAM_ROT, g_cam, rot[0], rot[1], rot[2], 2);
    Call<void>(SET_CAM_FOV, g_cam, std::clamp(fov, 10.0f, 120.0f));
}

void DrawTimeline() {
    const double start = g_rec.Start(), len = std::max(g_rec.Length(), 1e-3);
    const float x0 = 0.2f, w = 0.6f, y = StatusY() - 0.02f;
    Call<void>(DRAW_RECT, x0 + w / 2, y, w, 0.008f, 20, 20, 20, 180, 0);
    const float f = static_cast<float>((g_head.time - start) / len);
    Call<void>(DRAW_RECT, x0 + w * f / 2, y, w * f, 0.008f, 255, 200, 40, 230, 0);
    Call<void>(DRAW_RECT, x0 + w * f, y, 0.003f, 0.02f, 255, 255, 255, 255, 0);
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
    g_filterStrength = filterStrength;
}

void Clear() {
    g_rec.Clear();
    g_lastTick = 0;
}

void Record(const SvOutput& o, const float* placed, const float* bones, int count, const SvBoardPose* board) {
    if (!g_enabled || g_active || o.tick == g_lastTick || count <= 0 || !bones) return;
    if (count != g_bones) {
        g_rec.Reserve(g_seconds, 60.0, count);
        g_bones = count;
        g_lastTick = 0;
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
    g_head = Playhead{};
    g_head.time = std::max(g_rec.Start(), g_rec.End() - 10.0);  // the last ten seconds first
    g_camera = Recorded;
    const Frame& last = g_rec.Last();
    // Tripod and free cameras start at the camera play left.
    std::memcpy(g_eye, last.camera, sizeof(g_eye));
    g_pitch = last.camera[3], g_yaw = last.camera[5];
    g_height = 0.0f;
    g_hudHidden = false;  // every replay starts with its HUD and help
    g_zoom = 1.0f;
    g_orbitYaw = last.camera[5], g_orbitDist = 4.0f, g_orbitHeight = 1.2f;
    std::memcpy(g_staticRot, last.camera + 3, sizeof(g_staticRot));
    g_cam = Call<int>(CREATE_CAM, "DEFAULT_SCRIPTED_CAMERA", 0);
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
    g_filterSet = false;
    ApplyFilter();
    ApplyLens();
    g_input.Reset();
    posehook::TakeApplyTime();
    g_applyCalls = 0;
    (void)ped;
    char text[160];
    std::snprintf(text, sizeof(text), "replay: entered (%.1f s recorded, world time scale %.2f, sound %s, %s, %s)", g_rec.Length(),
                  g_worldScale, !g_sound ? "off" : sound ? "on" : "none recorded", FilterName(g_filter), LensName(g_lens));
    Log(text);
    return true;
}

bool Tick(int ped, const PadIn& pad, const KeysIn& keys, float dt) {
    if (!g_active) return false;
    // GTA's controls are taken by the caller as in skating (DisableSkateControls),
    // never all at once: that would stop GTA's own recorder.
    Call<void>(HIDE_HUD_AND_RADAR_THIS_FRAME);
    // D-pad down is GTA's character wheel; the replay uses it for the lens.
    Call<void>(DISABLE_CONTROL_ACTION, 0, 19, 1);
    const Command in = g_input.Update(pad, keys, dt, ArrowsMove(), !g_head.playing);
    // B, Back, Tab or Backspace leaves on the release, so the press never reaches
    // skating afterwards (a Back release there toggles the gun or opens the replay).
    if (in.leave) return false;
    if (in.playPause) {
        if (!g_head.playing && g_head.time >= g_rec.End()) g_head.time = g_rec.Start();
        g_head.playing = !g_head.playing;
    }
    if (in.slower) g_head.Slower();
    if (in.faster) g_head.Faster();
    if (in.nextCamera) {
        g_camera = (g_camera + 1) % kCameras;
        g_height = 0.0f;  // each camera starts at its own height (the copied eye already has the old one)
        if (g_camera == Free || g_camera == Orbit) g_head.reverse = false;  // no reverse there: never latched on
        // Tripod, Free and Static start where the camera is; Static also keeps its direction.
        if (g_camera == Tripod || g_camera == Free || g_camera == Static) {
            const Vector3 c = Call<Vector3>(GET_CAM_COORD, g_cam);
            g_eye[0] = c.x, g_eye[1] = c.y, g_eye[2] = c.z;
            if (g_camera == Static) {
                const Vector3 r = Call<Vector3>(GET_CAM_ROT, g_cam, 2);
                g_staticRot[0] = r.x, g_staticRot[1] = r.y, g_staticRot[2] = r.z;
            }
        }
    }
    if (in.filterStep) g_filter = Cycle(g_filter, in.filterStep, kFilterCount), ApplyFilter();
    if (in.lensStep) g_lens = Cycle(g_lens, in.lensStep, kLensCount), g_zoom = 1.0f, ApplyLens();
    if (in.hideHud) g_hudHidden = !g_hudHidden;
    if (in.zoom != 0.0f) g_zoom = std::clamp(g_zoom * std::pow(2.0f, -in.zoom * dt), 0.25f, 1.6f);  // ] halves the FOV in a second
    if (in.frameStep) {
        g_head.playing = false;
        g_head.time = std::clamp(g_head.time + in.frameStep * kTickSeconds, g_rec.Start(), g_rec.End());
    }
    // Follow, Tripod and Filmer: E/Q and the right stick's up/down set the
    // camera height (LT stays "hold to play backwards").
    if (g_camera == Follow || g_camera == Tripod || g_camera == Filmer) {
        const float rise = static_cast<float>(keys.up) - static_cast<float>(keys.down) + StickAxis(pad.rightY);
        g_height = std::clamp(g_height + std::clamp(rise, -1.0f, 1.0f) * 3.0f * dt, -1.0f, 4.0f);
    }
    bool scrubbing = false;
    if (g_camera != Free && g_camera != Orbit) {
        g_head.reverse = in.reverse;
        if (!g_head.playing) {
            g_head.time += in.scrub * dt * 2.0;  // scrub
            scrubbing = in.scrub != 0.0f;
        }
    }
    g_head.Advance(dt, g_rec.Start(), g_rec.End());
    const Recording::Sample s = g_rec.At(g_head.time);
    if (s.a) {
        Present(ped, s);
        Aim(s, dt, in);
    }
    gtaaudio::ReplaySoundAt(GameMsAt(s), SoundRate(g_head, scrubbing));
    if (HudOnGta()) DrawTimeline();  // else the fisheye overlay draws it
    g_applyCalls += posehook::TakeApplyTime().calls;
    return true;
}

void Exit() {
    if (!g_active) return;
    g_active = false;
    Call<void>(SET_TIME_SCALE, 1.0f);
    if (g_cam) {
        Call<void>(SET_CAM_ACTIVE, g_cam, 0);
        Call<void>(DESTROY_CAM, g_cam, 0);
        g_cam = 0;
    }
    gtaaudio::ReplaySoundStop();
    gtaaudio::SetReplayPaused(false);
    if (g_filterSet) Call<void>(CLEAR_TIMECYCLE_MODIFIER), g_filterSet = false;
    fisheye::SetEnabled(false);
    fisheye::SetHud(std::string(), 0.0f, false);
    char text[160];
    std::snprintf(text, sizeof(text), "replay: left (pose writes during the replay: %u)", g_applyCalls);
    Log(text);
}

std::string Status(bool keyboard) {
    char text[512];
    static const char* speeds[] = {"1/4x", "1/2x", "1x", "2x"};
    const bool height = g_camera == Follow || g_camera == Tripod || g_camera == Filmer;
    const char* help;
    if (!keyboard) {
        // Paused, LB/RB step one frame; the right stick's click hides this line.
        help = g_camera == Free    ? "A play/pause  LB/RB speed  Y camera  left stick move  RT/LT up/down  right stick look  X filter  D-pad lens  B back"
               : g_camera == Orbit ? "A play/pause  LB/RB speed  Y camera  right stick turn  left stick distance  RT/LT up/down  X filter  D-pad lens  B back"
               : height ? "A play/pause  left stick scrub  LB/RB speed  hold LT reverse  right stick up/down  Y camera  X filter  D-pad lens  B back"
                        : "A play/pause  left stick scrub  LB/RB speed  hold LT reverse  Y camera  X filter  D-pad lens  B back";
    } else {
        help = g_camera == Free    ? "Space play/pause  wheel or +/- speed  C camera  W A S D move  E/Q or Up/Down up/down  mouse look  F filter  L lens  Tab back"
               : g_camera == Orbit ? "Space play/pause  wheel or +/- speed  C camera  mouse turn  W/S distance  E/Q or Up/Down up/down  F filter  L lens  Tab back"
               : height ? "Space play/pause  A/D scrub  Up/Down, wheel or +/- speed  hold R reverse  E/Q up/down  C camera  F filter  L lens  Tab back"
                        : "Space play/pause  A/D scrub  Up/Down, wheel or +/- speed  hold R reverse  C camera  F filter  L lens  Tab back";
    }
    // Two lines (GTA's ~n~): the state, then the keys. Paused, LB/RB step frames.
    std::string keys = help;
    if (!keyboard && !g_head.playing) {
        const std::size_t at = keys.find("LB/RB speed");
        if (at != std::string::npos) keys.replace(at, 11, "LB/RB frame");
    }
    const char* more = keyboard ? "  , . frame  [ ] zoom  H hide" : "  R3 hide";
    std::snprintf(text, sizeof(text), "REPLAY  %s%s  %s  %s  %s  %.1f / %.1f s~n~%s%s", g_head.reverse ? "-" : "", speeds[g_head.speed],
                  kCameraNames[g_camera], LensName(g_lens), FilterName(g_filter), g_head.time - g_rec.Start(), g_rec.Length(), keys.c_str(), more);
    return text;
}

float StatusY() { return 0.925f; }  // two lines

bool HudShown() { return g_active && !g_hudHidden; }
// GTA draws the HUD unless the fisheye pass is really drawing (it may have
// failed, or the present callback may be missing): never no HUD at all.
bool HudOnGta() { return HudShown() && !fisheye::Drawing(); }

void DrawHudOverlay(const std::string& status) {
    const double len = std::max(g_rec.Length(), 1e-3);
    const float progress = static_cast<float>((g_head.time - g_rec.Start()) / len);
    fisheye::SetHud(status, progress, HudShown() && fisheye::Enabled());  // drawn only while the pass runs
}

}  // namespace replay
