#include "vehicle_survey.h"
#include "natives.h"
#include "runtime_bridge.h"
#include <windows.h>
#include <cstdint>
#include <cstdio>

namespace vehiclesurvey {
namespace {
using namespace gta;

// One probe against `vehicle`, in its model space: from and to become world
// points, and only a hit on that vehicle counts.
std::uint32_t __cdecl Probe(void* ctx, const float* from, const float* to, float* hit) {
    const int vehicle = static_cast<int>(reinterpret_cast<std::intptr_t>(ctx));
    const Vector3 a = Call<Vector3>(GET_OFFSET_FROM_ENTITY_IN_WORLD_COORDS, vehicle, from[0], from[1], from[2]);
    const Vector3 b = Call<Vector3>(GET_OFFSET_FROM_ENTITY_IN_WORLD_COORDS, vehicle, to[0], to[1], to[2]);
    const int test = Call<int>(START_EXPENSIVE_SYNCHRONOUS_SHAPE_TEST_LOS_PROBE, a.x, a.y, a.z, b.x, b.y, b.z, 2,
                               Call<int>(PLAYER_PED_ID), 7);
    BOOL any = 0;
    Vector3 end{}, normal{};
    Entity entity = 0;
    if (Call<int>(GET_SHAPE_TEST_RESULT, test, &any, &end, &normal, &entity) != 2 || !any || entity != vehicle) return 0;
    const Vector3 local = Call<Vector3>(GET_OFFSET_FROM_ENTITY_GIVEN_WORLD_COORDS, vehicle, end.x, end.y, end.z);
    hit[0] = local.x, hit[1] = local.y, hit[2] = local.z;
    return 1;
}

} // namespace

void Tick(RuntimeBridge& runtime, float radius, void (*log)(const char*)) {
    if (!runtime.HasVehicleSurvey()) return;
    // The pool scan twice a second is plenty: a model is surveyed only once.
    static unsigned long s_next = 0;
    const unsigned long now = GetTickCount();
    if (static_cast<long>(now - s_next) < 0) return;
    s_next = now + 500;
    static int vehicles[256];
    const int n = worldGetAllVehicles(vehicles, 256);
    const int ped = Call<int>(PLAYER_PED_ID);
    const Vector3 me = Call<Vector3>(GET_ENTITY_COORDS, ped, 0);
    for (int i = 0; i < n; ++i) {
        const int v = vehicles[i];
        const Vector3 p = Call<Vector3>(GET_ENTITY_COORDS, v, 0);
        const float dx = p.x - me.x, dy = p.y - me.y, dz = p.z - me.z;
        if (dx * dx + dy * dy + dz * dz > radius * radius) continue;
        const std::uint32_t model = Call<std::uint32_t>(GET_ENTITY_MODEL, v);
        if (!runtime.VehicleNeedsShape(model)) continue;
        Vector3 lo{}, hi{};
        Call<void>(GET_MODEL_DIMENSIONS, model, &lo, &hi);
        const float min[3] = {lo.x, lo.y, lo.z}, max[3] = {hi.x, hi.y, hi.z};
        const int tris = runtime.VehicleSurvey(model, min, max, Probe, reinterpret_cast<void*>(static_cast<std::intptr_t>(v)));
        if (log) {
            char line[160];
            std::snprintf(line, sizeof(line), "vehicle shape: model %08x surveyed: %d triangles (%.1f x %.1f x %.1f m)", model,
                          tris, hi.x - lo.x, hi.y - lo.y, hi.z - lo.z);
            log(line);
        }
        return;  // one model per frame: a survey is up to 500 probes
    }
}

} // namespace vehiclesurvey
