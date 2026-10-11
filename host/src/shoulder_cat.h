#pragma once
#include <main.h>
#include <windows.h>
#include <algorithm>
#include <cmath>
#include "natives.h"
#include "pose_math.h"
#include "host_util.h"

namespace shouldercat {
struct Settings {
    bool enabled = true, left = false;
    int pose = 0;
    float sideways = 0.12f, forward = -0.14f, height = 0.04f;
    float yaw = 0.0f;
};
struct State {
    int cat = 0, player = 0, group = 0;
    unsigned model = 0;
    bool collision = true, ragdoll = true;
    DWORD scanAt = 0, animAt = 0;
    float torsoRightX = 1.0f, torsoRightY = 0.0f;
};
inline State state;
inline bool IsCat(unsigned model) {
    constexpr auto cat = util::Joaat("a_c_cat_01");
    constexpr auto cat2 = util::Joaat("a_c_cat_02");
    return model == cat || model == cat2;
}
inline bool Exists() {
    return state.cat && gta::Call<BOOL>(gta::DOES_ENTITY_EXIST, state.cat)
        && gta::Call<unsigned>(gta::GET_ENTITY_MODEL, state.cat) == state.model;
}
inline void Release(void (*log)(const char*)) {
    if (Exists()) {
        gta::Call<void>(0x961AC54BF0613F5DULL, state.cat, false, false);
        gta::Call<void>(gta::CLEAR_PED_TASKS_IMMEDIATELY, state.cat);
        if (state.player && gta::Call<BOOL>(gta::DOES_ENTITY_EXIST, state.player)) {
            auto p = gta::Call<Vector3>(0x1899F328B0E12848ULL, state.player, 1.0f, 0.0f, 0.0f);
            float z = p.z;
            if (gta::Call<BOOL>(gta::GET_GROUND_Z_FOR_3D_COORD, p.x, p.y, p.z + 2.0f, &z, false, false)) p.z = z + 0.05f;
            gta::Call<void>(gta::SET_ENTITY_COORDS_NO_OFFSET, state.cat, p.x, p.y, p.z, false, false, false);
            // Restore the follower role to the same player's current group.
            const int group = gta::Call<int>(0xF162E133B4E7A675ULL, state.player);
            if (group == state.group) {
                gta::Call<void>(0x9F3480FE65DB31B5ULL, state.cat, group);
                gta::Call<void>(0x3DBFC55D5C9BB447ULL, state.cat, true);
                gta::Call<void>(0x2E2F4240B3F24647ULL, state.cat, group, true);
            }
        }
        gta::Call<void>(gta::SET_ENTITY_COLLISION, state.cat, state.collision, false);
        gta::Call<void>(gta::SET_PED_CAN_RAGDOLL, state.cat, state.ragdoll);
        if (log) log("shoulder cat released; follower role restored");
    }
    state = {};
}

// Acquire exactly one living cat from the player's group. Ambient cats and
// non-following Spooner cats cannot qualify, irrespective of their distance.
inline void Tick(int player, bool skating, const Settings& cfg,
                 const float* bones, unsigned count, void (*log)(const char*)) {
    if (!skating || !cfg.enabled || !player || (state.player && state.player != player)) {
        if (state.cat) Release(log);
        return;
    }
    if (state.cat && (!Exists() || gta::Call<BOOL>(gta::IS_ENTITY_DEAD, state.cat, false))) Release(log);
    const DWORD now = GetTickCount();
    if (!state.cat && static_cast<int>(now - state.scanAt) >= 0) {
        state.scanAt = now + 500;
        const int group = gta::Call<int>(0xF162E133B4E7A675ULL, player);
        const auto at = gta::Call<Vector3>(gta::GET_ENTITY_COORDS, player, true);
        int pool[1024]{};
        const int n = worldGetAllPeds(pool, 1024);
        int selected = 0;
        float nearest = 40.0f * 40.0f;
        for (int i = 0; i < n; ++i) {
            const int ped = pool[i];
            if (ped == player || !gta::Call<BOOL>(gta::DOES_ENTITY_EXIST, ped)
                || !IsCat(gta::Call<unsigned>(gta::GET_ENTITY_MODEL, ped))
                || !gta::Call<BOOL>(0x9BB01E3834671191ULL, ped, group)
                || gta::Call<BOOL>(gta::IS_ENTITY_DEAD, ped, false)
                || gta::Call<BOOL>(gta::IS_ENTITY_ATTACHED, ped)) continue;
            const auto p = gta::Call<Vector3>(gta::GET_ENTITY_COORDS, ped, true);
            const float dx = p.x-at.x, dy = p.y-at.y, dz = p.z-at.z;
            const float d = dx*dx + dy*dy + dz*dz;
            if (d < nearest) { selected = ped; nearest = d; }
        }
        if (selected) {
            state.cat = selected; state.player = player; state.group = group;
            state.model = gta::Call<unsigned>(gta::GET_ENTITY_MODEL, selected);
            state.collision = !gta::Call<BOOL>(gta::GET_ENTITY_COLLISION_DISABLED, selected);
            state.ragdoll = gta::Call<BOOL>(gta::CAN_PED_RAGDOLL, selected) != 0;
            gta::Call<void>(0x2E2F4240B3F24647ULL, selected, group, false);
            gta::Call<void>(0xED74007FFB146BC2ULL, selected);
            gta::Call<void>(gta::CLEAR_PED_TASKS_IMMEDIATELY, selected);
            gta::Call<void>(gta::SET_ENTITY_COLLISION, selected, false, false);
            gta::Call<void>(gta::SET_PED_CAN_RAGDOLL, selected, false);
            if (log) log("shoulder cat acquired from player's follower group; group catch-up suspended");
        }
    }
    if (!state.cat) return;

    // The runtime pose is already world space, so use the actual animated
    // shoulder. Root attachment keeps the cat upright as the skater leans.
    const auto clavicle = [&](int tag) {
        const int bone = gta::Call<int>(0x3F428D08BE5AAE31ULL, player, tag);
        Vector3 point{};
        if (bones && bone >= 0 && static_cast<unsigned>(bone) < count) {
            const float* matrix = bones + 16 * bone;
            point.x=matrix[12]; point.y=matrix[13]; point.z=matrix[14];
        } else {
            point = gta::Call<Vector3>(0x17C07FC640E86B4EULL, player, tag, 0.0f, 0.0f, 0.0f);
        }
        return point;
    };
    const auto leftShoulder = clavicle(64729), rightShoulder = clavicle(10706);
    Vector3 shoulder = cfg.left ? leftShoulder : rightShoulder;
    // Anatomical shoulder selection and offsets follow the animated torso,
    // independent of the entity heading that reverses with board travel.
    const float rx = rightShoulder.x-leftShoulder.x, ry = rightShoulder.y-leftShoulder.y;
    const float width = std::hypot(rx, ry);
    if (width > 0.02f) {
        state.torsoRightX = rx/width; state.torsoRightY = ry/width;
    }
    const float side = cfg.left ? -cfg.sideways : cfg.sideways;
    shoulder.x += side*state.torsoRightX - cfg.forward*state.torsoRightY;
    shoulder.y += side*state.torsoRightY + cfg.forward*state.torsoRightX;
    shoulder.z += cfg.height;
    Vector3 forward{}, right{}, up{}, position{};
    gta::Call<void>(gta::GET_ENTITY_MATRIX, player, &forward, &right, &up, &position);
    posemath::M m = posemath::Identity(), inv{};
    const Vector3 axes[] = {right, forward, up, position};
    for (int i=0; i<4; ++i) { m.r[i][0]=axes[i].x; m.r[i][1]=axes[i].y; m.r[i][2]=axes[i].z; }
    if (!posemath::Inverse(m, inv)) return;
    float local[3]{};
    for (int i=0; i<3; ++i) local[i]=shoulder.x*inv.r[0][i]+shoulder.y*inv.r[1][i]+shoulder.z*inv.r[2][i]+inv.r[3][i];
    const float torsoYaw = std::atan2(state.torsoRightY, state.torsoRightX);
    const float rootYaw = std::atan2(right.y, right.x);
    const float relativeYaw = std::remainder((torsoYaw-rootYaw)*57.2957795f + cfg.yaw, 360.0f);
    gta::Call<void>(0x6B9BBD38AB0796DFULL, state.cat, player, -1,
        local[0], local[1], local[2],
        0.0f, 0.0f, relativeYaw, false, false, false, true, 2, true);

    const char* dicts[] = {"creatures@cat@amb@world_cat_sleeping_ledge@base",
        "creatures@cat@amb@world_cat_sleeping_ground@base", "creatures@cat@move"};
    const int pose = (std::clamp)(cfg.pose, 0, 2);
    const char* dict = dicts[pose];
    const char* clip = pose == 2 ? "idle" : "base";
    gta::Call<void>(gta::REQUEST_ANIM_DICT, dict);
    if (static_cast<int>(now - state.animAt) >= 0 && gta::Call<BOOL>(gta::HAS_ANIM_DICT_LOADED, dict)) {
        state.animAt = now + 500;
        if (!gta::Call<BOOL>(gta::IS_ENTITY_PLAYING_ANIM, state.cat, dict, clip, 3)) {
            gta::Call<void>(gta::TASK_PLAY_ANIM, state.cat, dict, clip, 4.0f, -4.0f, -1, 1, 1.0f, true, true, true);
            if (log) log(dict);
        }
    }
}
} // namespace shouldercat
