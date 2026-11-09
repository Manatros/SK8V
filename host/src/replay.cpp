#include "replay.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <utility>

#include "board_native.h"
#include "board_parts.h"
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

enum Camera { Recorded, Follow, Tripod, Free, Filmer, kCameras };
constexpr const char* kCameraNames[kCameras] = {"Recorded", "Follow", "Tripod", "Free", "Filmer"};

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
        const float want[3] = {root[0] + std::sin(heading) * 4.0f, root[1] - std::cos(heading) * 4.0f, root[2] + 1.8f};
        const float k = std::min(1.0f, dt * 4.0f);
        for (int i = 0; i < 3; ++i) g_eye[i] += (want[i] - g_eye[i]) * k;
        std::memcpy(eye, g_eye, sizeof(eye));
        break;
    }
    case Tripod:
        std::memcpy(eye, g_eye, sizeof(eye));
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
        eye[2] = root[2] - 0.5f;
        fov = kFilmerFov;
        break;
    }
    }
    if (g_camera != Recorded && g_camera != Free) {
        // Turn to the skater's chest.
        const float dx = root[0] - eye[0], dy = root[1] - eye[1], dz = root[2] + 0.4f - eye[2];
        rot[0] = std::atan2(dz, std::hypot(dx, dy)) * 57.29578f;
        rot[2] = std::atan2(-dx, dy) * 57.29578f;
        fov = g_camera == Filmer ? kFilmerFov : 50.0f;
    }
    Call<void>(SET_CAM_COORD, g_cam, eye[0], eye[1], eye[2]);
    Call<void>(SET_CAM_ROT, g_cam, rot[0], rot[1], rot[2], 2);
    Call<void>(SET_CAM_FOV, g_cam, std::clamp(fov, 10.0f, 120.0f));
}

void DrawTimeline() {
    const double start = g_rec.Start(), len = std::max(g_rec.Length(), 1e-3);
    const float x0 = 0.2f, w = 0.6f, y = 0.92f;
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
    g_cam = Call<int>(CREATE_CAM, "DEFAULT_SCRIPTED_CAMERA", 0);
    Call<void>(SET_CAM_ACTIVE, g_cam, 1);
    Call<void>(RENDER_SCRIPT_CAMS, 1, 0, 0, 1, 0, 0);
    Call<void>(SET_TIME_SCALE, g_worldScale);
    gtaaudio::SetReplayPaused(true);
    g_input.Reset();
    posehook::TakeApplyTime();
    g_applyCalls = 0;
    (void)ped;
    char text[160];
    std::snprintf(text, sizeof(text), "replay: entered (%.1f s recorded, world time scale %.2f)", g_rec.Length(), g_worldScale);
    Log(text);
    return true;
}

bool Tick(int ped, const PadIn& pad, const KeysIn& keys, float dt) {
    if (!g_active) return false;
    // GTA's controls are taken by the caller as in skating (DisableSkateControls),
    // never all at once: that would stop GTA's own recorder.
    Call<void>(HIDE_HUD_AND_RADAR_THIS_FRAME);
    const Command in = g_input.Update(pad, keys, dt);
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
        if (g_camera == Tripod || g_camera == Free) {
            const Vector3 c = Call<Vector3>(GET_CAM_COORD, g_cam);
            g_eye[0] = c.x, g_eye[1] = c.y, g_eye[2] = c.z;
        }
    }
    if (g_camera != Free) {
        g_head.reverse = in.reverse;
        if (!g_head.playing) g_head.time += in.scrub * dt * 2.0;  // scrub
    }
    g_head.Advance(dt, g_rec.Start(), g_rec.End());
    const Recording::Sample s = g_rec.At(g_head.time);
    if (s.a) {
        Present(ped, s);
        Aim(s, dt, in);
    }
    DrawTimeline();
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
    gtaaudio::SetReplayPaused(false);
    char text[160];
    std::snprintf(text, sizeof(text), "replay: left (pose writes during the replay: %u)", g_applyCalls);
    Log(text);
}

std::string Status(bool keyboard) {
    char text[256];
    static const char* speeds[] = {"1/4x", "1/2x", "1x", "2x"};
    const char* help = !keyboard ? (g_camera == Free ? "A play/pause  LB/RB speed  Y camera  left stick move  RT/LT up/down  right stick look  B back"
                                                     : "A play/pause  left stick scrub  LB/RB speed  hold LT reverse  Y camera  B back")
                       : g_camera == Free ? "Space play/pause  Up/Down or wheel speed  C camera  W A S D move  E/Q up/down  mouse look  Tab back"
                                          : "Space play/pause  A/D scrub  Up/Down or wheel speed  hold R reverse  C camera  Tab back";
    std::snprintf(text, sizeof(text), "REPLAY  %s%s  %s camera  %.1f / %.1f s   %s", g_head.reverse ? "-" : "", speeds[g_head.speed],
                  kCameraNames[g_camera], g_head.time - g_rec.Start(), g_rec.Length(), help);
    return text;
}

}  // namespace replay
