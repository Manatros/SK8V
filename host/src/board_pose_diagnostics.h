#pragma once
// Developer observations only: no pose correction, allocation or game access.
#include "pose_math.h"
#include <algorithm>
#include <cmath>
#include <cstdint>

namespace boardnative::diagnostics {
using posemath::M;
struct State {
    bool active = false, previous = false, priorWrite = false;
    std::uint64_t lastMs = 0, lastTick = 0;
    unsigned samples = 0;
    M written[7]{}, sampled[7]{};
    float motion[6]{}, beforeWritten[6]{};
    float localError = 0, objectError = 0;
};
inline bool ShouldSample(State& s, bool enabled, std::uint64_t tick, std::uint64_t now) {
    if (!enabled) { s.active = false; return false; }
    if (!s.active) { s = State{}; s.active = true; }
    if (s.previous && (tick == s.lastTick || now - s.lastMs < 250)) return false;
    return true;
}
inline float RotationDegrees(const M& a, const M& b) {
    float trace = 0;
    for (int r = 0; r < 3; ++r) {
        float aa = 0, bb = 0, ab = 0;
        for (int c = 0; c < 3; ++c) { aa += a.r[r][c] * a.r[r][c]; bb += b.r[r][c] * b.r[r][c]; ab += a.r[r][c] * b.r[r][c]; }
        if (!(aa > 1e-12f && bb > 1e-12f) || !std::isfinite(ab)) return 180.0f;
        trace += ab / std::sqrt(aa * bb);
    }
    return std::acos(std::clamp((trace - 1) * 0.5f, -1.0f, 1.0f)) * 57.2957795f;
}
inline float RotationElementError(const M& a, const M& b) {
    float error = 0;
    for (int r = 0; r < 3; ++r) for (int c = 0; c < 3; ++c) {
        const float delta = std::fabs(a.r[r][c] - b.r[r][c]);
        if (!std::isfinite(delta)) return 1e9f;
        error = std::max(error, delta);
    }
    return error;
}
// intended and observed locals are parent-relative, so board/world motion is
// not counted as wheel or truck motion. beforeWritten is an observation, not
// evidence identifying which game system changed the matrices between calls.
inline void Sample(State& s, std::uint64_t tick, std::uint64_t now,
                   const M (&intended)[7], const M (&before)[7], const M (&after)[7],
                   const M (&objects)[7], const M (&objectsAfter)[7]) {
    for (int i = 1; i < 7; ++i) {
        if (s.previous) {
            s.motion[i - 1] = std::max(s.motion[i - 1], RotationDegrees(intended[i], s.sampled[i]));
        }
        if (s.priorWrite) s.beforeWritten[i - 1] = std::max(s.beforeWritten[i - 1], RotationDegrees(before[i], s.written[i]));
        s.localError = std::max(s.localError, RotationElementError(intended[i], after[i]));
        s.objectError = std::max(s.objectError, RotationElementError(objects[i], objectsAfter[i]));
    }
    for (int i = 0; i < 7; ++i) s.sampled[i] = intended[i];
    s.previous = true; s.lastTick = tick; s.lastMs = now;
    if (s.samples != UINT32_MAX) ++s.samples;
}
// Keep the actual last write separate from the slower motion sampling: using
// the last sampled pose here would mislabel ordinary animation as a change.
inline void Written(State& s, const M (&intended)[7]) {
    for (int i = 0; i < 7; ++i) s.written[i] = intended[i];
    s.priorWrite = true;
}
inline void ClearWindow(State& s) {
    for (int i = 0; i < 6; ++i) s.motion[i] = s.beforeWritten[i] = 0;
    s.samples = 0; s.localError = s.objectError = 0;
}
} // namespace boardnative::diagnostics
