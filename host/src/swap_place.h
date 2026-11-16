#pragma once
// Pure logic for the Skate <-> GTA mode swap ("teleporting between places when swapping
// skate and GTA mode", SK8V-BUGS #3). No game calls, so host/tests can run it.
#include <cmath>

namespace swapplace {

enum class Settle { Hold, Place, Abort };

// Skate's first frames after an entry can carry the last session's skater. Until the skater is
// within `nearM` of the entry point the ped is not placed (Hold). Once the settle window is over
// and the skater is still far away (stale or never arrived), Abort: the ped stays where it stood
// instead of being placed at a stale position.
inline Settle DecideSettle(float dx, float dy, float dz, long remainingMs, float nearM = 10.0f) {
    const float d2 = dx * dx + dy * dy + dz * dz;
    if (std::isfinite(d2) && d2 < nearM * nearM) return Settle::Place;
    return remainingMs > 0 ? Settle::Hold : Settle::Abort;
}

// Ground height for an entry. GTA's probe fails or answers sea level where it has no collision
// (offshore islands, unstreamed areas, 2026-11-15: entry at z 0.05 on Tutorial Island). Accept it
// only when it lies between 10 m below and 1 m above the ped; else the ped's own height - 1 m.
inline float EntryGround(bool probeOk, float probed, float pedZ) {
    if (probeOk && std::isfinite(probed) && probed <= pedZ + 1.0f && probed >= pedZ - 10.0f) return probed;
    return pedZ - 1.0f;
}

} // namespace swapplace
