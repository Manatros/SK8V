#include "skate_buddies.h"
#include "host_util.h"
#include "natives.h"
#include <windows.h>
#include <atomic>
#include <cstdint>
#include <cstdio>

namespace skatebuddies {
namespace {
using namespace gta;

constexpr int kMax = 3;
constexpr const char* kModels[] = {"a_m_y_skater_01", "a_m_y_skater_02", "a_f_y_skater_01"};

std::atomic<int> g_calls{0};
std::atomic<bool> g_dismiss{false};
int g_buddies[kMax] = {};
std::uint32_t g_loading = 0;
ULONGLONG g_loadingSince = 0;
int g_next = 0;

int Count() {
    int n = 0;
    for (int& b : g_buddies) {
        if (b && !Call<BOOL>(DOES_ENTITY_EXIST, b)) b = 0;
        if (b) ++n;
    }
    return n;
}

} // namespace

void Summon() { g_calls.fetch_add(1); }
void Dismiss() { g_dismiss.store(true); }

void Tick(void (*log)(const char*)) {
    if (g_dismiss.exchange(false)) {
        int n = 0;
        for (int& b : g_buddies) {
            if (!b || !Call<BOOL>(DOES_ENTITY_EXIST, b)) { b = 0; continue; }
            Call<void>(REMOVE_PED_FROM_GROUP, b);
            Call<void>(SET_PED_NEVER_LEAVES_GROUP, b, 0);
            Call<void>(SET_PED_AS_NO_LONGER_NEEDED, &b);
            b = 0, ++n;
        }
        g_calls.store(0);
        if (log) {
            char line[96];
            std::snprintf(line, sizeof(line), "skate buddies: %d sent home", n);
            log(line);
        }
    }
    if (g_calls.load() <= 0) return;
    if (Count() >= kMax) {
        g_calls.store(0);
        if (log) log("skate buddies: the crew is full");
        return;
    }
    if (!g_loading) {
        g_loading = util::Joaat(kModels[g_next++ % (sizeof(kModels) / sizeof(*kModels))]);
        g_loadingSince = GetTickCount64();
        Call<void>(REQUEST_MODEL, g_loading);
    }
    if (!Call<BOOL>(HAS_MODEL_LOADED, g_loading)) {
        if (GetTickCount64() - g_loadingSince > 10000) {
            if (log) log("skate buddies: the skater model did not load");
            g_loading = 0, g_calls.store(0);
        }
        return;
    }
    // Two metres behind the player and a little to the side, facing the same way.
    const int player = Call<int>(PLAYER_PED_ID);
    const Vector3 at = Call<Vector3>(GET_OFFSET_FROM_ENTITY_IN_WORLD_COORDS, player, (Count() % 2 ? 1.2f : -1.2f), -2.0f, 0.0f);
    const float heading = Call<float>(GET_ENTITY_HEADING, player);
    const int ped = Call<int>(CREATE_PED, 4, g_loading, at.x, at.y, at.z, heading, 0, 1);
    Call<void>(SET_MODEL_AS_NO_LONGER_NEEDED, g_loading);
    g_loading = 0;
    g_calls.fetch_sub(1);
    if (!ped) {
        if (log) log("skate buddies: GTA did not place the friend");
        return;
    }
    Call<void>(SET_PED_RANDOM_COMPONENT_VARIATION, ped, 0);
    const int group = Call<int>(GET_PED_GROUP_INDEX, player);
    Call<void>(SET_PED_AS_GROUP_MEMBER, ped, group);
    Call<void>(SET_PED_NEVER_LEAVES_GROUP, ped, 1);
    Call<void>(SET_GROUP_SEPARATION_RANGE, group, 300.0f);
    Call<void>(SET_BLOCKING_OF_NON_TEMPORARY_EVENTS, ped, 1);
    for (int& b : g_buddies)
        if (!b) { b = ped; break; }
    if (log) {
        char line[96];
        std::snprintf(line, sizeof(line), "skate buddies: a friend joined (ped %d, group %d), %d in the crew", ped, group, Count());
        log(line);
    }
}

} // namespace skatebuddies
