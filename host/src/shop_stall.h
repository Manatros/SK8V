#pragma once
// The Vespucci Skate Shop stall (design 2026-11-07-skate-shop-rework): its
// outline, the seller's spot and presence rule, and the inspect flight of a
// rack board. Pure helpers, tested in host/tests/shop_stall_tests.cpp.
#include <cmath>
#include <cstdio>
#include <string>
#include <vector>

namespace shopstall {

// The stall's outline (GTA's vb_29_stall_02/marketwalls, measured 2026-11-07).
inline constexpr float kOutline[4][2] = {
    {-1230.3f, -1437.0f}, {-1227.0f, -1443.0f}, {-1222.6f, -1440.3f}, {-1225.8f, -1435.0f}};
// The seller's default spot: behind the register, in the glass counter's U
// (collision of vb_29_0.ybn, material 55, panes floor to 6.41 m, so the player
// cannot walk in; measured 2026-11-08 with the bake probe). The register's arm
// runs (-1225.75,-1438.35)..(-1224.55,-1437.55); 0.55 m in from its middle,
// facing out over it.
inline constexpr float kSeller[4] = {-1225.45f, -1437.49f, 3.34f, 213.5f};
inline constexpr float kCounter[4][2] = {
    {-1227.80f, -1437.00f}, {-1226.95f, -1438.20f}, {-1225.75f, -1438.35f}, {-1224.55f, -1437.55f}};
inline constexpr float kBackWall[2][2] = {{-1230.0f, -1439.65f}, {-1222.5f, -1434.37f}};
inline constexpr float kSellerNear = 60.0f;  // wanted within this of the anchor
inline constexpr float kSellerFar = 80.0f;   // removed beyond this
inline constexpr float kFlightSeconds = 0.35f;

// Inside the outline grown by `grow` metres (convex quad, either winding).
inline bool InStall(float x, float y, float grow = 0.5f) {
    float area = 0.0f;
    for (int i = 0; i < 4; ++i) {
        const float* a = kOutline[i];
        const float* b = kOutline[(i + 1) % 4];
        area += a[0] * b[1] - b[0] * a[1];
    }
    const float sign = area > 0.0f ? 1.0f : -1.0f;
    for (int i = 0; i < 4; ++i) {
        const float* a = kOutline[i];
        const float* b = kOutline[(i + 1) % 4];
        const float ex = b[0] - a[0], ey = b[1] - a[1];
        const float len = std::sqrt(ex * ex + ey * ey);
        // Signed distance to the edge's line, positive inside.
        const float d = sign * (ex * (y - a[1]) - ey * (x - a[0])) / len;
        if (d < -grow) return false;
    }
    return true;
}

// "x,y,z,heading" -> spot; false (spot unchanged) on bad text.
inline bool ParseSpot(const std::string& text, float (&spot)[4]) {
    float v[4];
    char tail = 0;
    if (std::sscanf(text.c_str(), " %f , %f , %f , %f %c", &v[0], &v[1], &v[2], &v[3], &tail) != 4) return false;
    for (float f : v)
        if (!std::isfinite(f)) return false;
    for (int i = 0; i < 4; ++i) spot[i] = v[i];
    return true;
}

// Whether a seller should be made: open hours and near. Being in the stall
// never makes one (none appears at night), it only keeps one (SellerGoes).
inline bool SellerWanted(bool openNow, float anchorDistance, bool /*inStall*/) {
    return openNow && anchorDistance <= kSellerNear;
}
// Whether an existing seller goes: not wanted, and either far away or closed
// with the player outside the stall.
inline bool SellerGoes(bool openNow, float anchorDistance, bool inStall) {
    if (inStall || SellerWanted(openNow, anchorDistance, inStall)) return false;
    return anchorDistance > kSellerFar || (!openNow && !inStall);
}

inline float Smoothstep(float t) {
    t = t < 0.0f ? 0.0f : t > 1.0f ? 1.0f : t;
    return t * t * (3.0f - 2.0f * t);
}

// Quaternion (x, y, z, w) of a rotation in `m` (column-major 4x4: columns
// 0..2 are the axes, as the runtime's board matrices).
inline void ToQuat(const float (&m)[16], float (&q)[4]) {
    const float m00 = m[0], m11 = m[5], m22 = m[10];
    const float tr = m00 + m11 + m22;
    if (tr > 0.0f) {
        const float s = std::sqrt(tr + 1.0f) * 2.0f;
        q[3] = 0.25f * s, q[0] = (m[6] - m[9]) / s, q[1] = (m[8] - m[2]) / s, q[2] = (m[1] - m[4]) / s;
    } else if (m00 > m11 && m00 > m22) {
        const float s = std::sqrt(1.0f + m00 - m11 - m22) * 2.0f;
        q[3] = (m[6] - m[9]) / s, q[0] = 0.25f * s, q[1] = (m[4] + m[1]) / s, q[2] = (m[8] + m[2]) / s;
    } else if (m11 > m22) {
        const float s = std::sqrt(1.0f + m11 - m00 - m22) * 2.0f;
        q[3] = (m[8] - m[2]) / s, q[0] = (m[4] + m[1]) / s, q[1] = 0.25f * s, q[2] = (m[9] + m[6]) / s;
    } else {
        const float s = std::sqrt(1.0f + m22 - m00 - m11) * 2.0f;
        q[3] = (m[1] - m[4]) / s, q[0] = (m[8] + m[2]) / s, q[1] = (m[9] + m[6]) / s, q[2] = 0.25f * s;
    }
    const float n = std::sqrt(q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]);
    for (float& c : q) c /= n;
}

inline void FromQuat(const float (&q)[4], float (&m)[16]) {
    const float x = q[0], y = q[1], z = q[2], w = q[3];
    m[0] = 1 - 2 * (y * y + z * z), m[1] = 2 * (x * y + z * w), m[2] = 2 * (x * z - y * w), m[3] = 0;
    m[4] = 2 * (x * y - z * w), m[5] = 1 - 2 * (x * x + z * z), m[6] = 2 * (y * z + x * w), m[7] = 0;
    m[8] = 2 * (x * z + y * w), m[9] = 2 * (y * z - x * w), m[10] = 1 - 2 * (x * x + y * y), m[11] = 0;
}

// The pose a rack board has `t` (0..1, smoothstepped) of the way from its
// rack `a` to the inspect pose `b`: position lerped, rotation slerped.
inline void Flight(const float (&a)[16], const float (&b)[16], float t, float (&out)[16]) {
    t = Smoothstep(t);
    float qa[4], qb[4];
    ToQuat(a, qa), ToQuat(b, qb);
    float dot = qa[0] * qb[0] + qa[1] * qb[1] + qa[2] * qb[2] + qa[3] * qb[3];
    if (dot < 0.0f) {
        for (float& c : qb) c = -c;
        dot = -dot;
    }
    float q[4];
    if (dot > 0.9995f) {
        for (int i = 0; i < 4; ++i) q[i] = qa[i] + (qb[i] - qa[i]) * t;
    } else {
        const float th = std::acos(dot), s = std::sin(th);
        const float wa = std::sin((1 - t) * th) / s, wb = std::sin(t * th) / s;
        for (int i = 0; i < 4; ++i) q[i] = qa[i] * wa + qb[i] * wb;
    }
    const float n = std::sqrt(q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]);
    for (float& c : q) c /= n;
    FromQuat(q, out);
    for (int i = 12; i < 15; ++i) out[i] = a[i] + (b[i] - a[i]) * t;
    out[15] = 1.0f;
}

// Index of the rack board the highlighted offer is, -1 for none: first one
// whose models all appear in the offer with the same deck first, else the
// first with the same deck (the offer wears the player's own griptape and
// truck paint, peer review 2026-11-07).
inline bool SameBoard(const std::vector<std::string>& rack, const std::vector<std::string>& offer);
inline int RackFor(const std::vector<std::vector<std::string>>& racks, const std::vector<std::string>& offer) {
    for (std::size_t i = 0; i < racks.size(); ++i)
        if (SameBoard(racks[i], offer)) return static_cast<int>(i);
    for (std::size_t i = 0; i < racks.size(); ++i)
        if (!racks[i].empty() && !offer.empty() && racks[i].front() == offer.front()) return static_cast<int>(i);
    return -1;
}

// Whether the highlighted offer (its models, as the menu lists them) is the
// rack board with `rack` models: the same deck first, and every rack model in
// the offer (the rack list drops models the game lacks; the offer may not).
inline bool SameBoard(const std::vector<std::string>& rack, const std::vector<std::string>& offer) {
    if (rack.empty() || offer.empty() || rack.front() != offer.front()) return false;
    for (const std::string& m : rack) {
        bool found = false;
        for (const std::string& o : offer) found = found || o == m;
        if (!found) return false;
    }
    // An offer model the rack lacks is one the game lacks (the rack list was
    // filtered by availability): a different part would differ in the rack's.
    return true;
}

}  // namespace shopstall
