#include "collision_view.h"

#include <cmath>
#include <array>
#include <unordered_set>
#include <vector>
#include "natives.h"
#include "skatev_runtime.h"

namespace collisionview {
namespace {
SvSetCollisionViewFn g_set = nullptr;
SvCollisionViewFn g_view = nullptr;
using WorldStatus = std::uint32_t(__cdecl*)(char*, std::uint32_t);
WorldStatus g_status = nullptr;
std::string g_worldState = "World collision: best", g_tileStatus;
ULONGLONG g_statusAt = 0;
bool g_statusAsked = false;
bool g_on = false;
float g_range = 10.0f;
// Asked every frame (moving bodies change each frame; the runtime keeps the
// static part while the player stays within a metre). DRAW_* lasts one frame.
constexpr int kRefresh = 1;
constexpr std::uint32_t kTris = 1500, kLines = 600;
std::vector<SvViewTri> g_tris(kTris);
std::vector<SvViewLine> g_lines(kLines);
std::uint32_t g_triCount = 0, g_lineCount = 0;
int g_frame = 0;

struct Colour { int r, g, b; const char* name; };
constexpr Colour kKinds[] = {{60, 220, 90, "floor"}, {40, 200, 230, "bank"}, {230, 60, 50, "wall"},
                             {170, 80, 220, "underside"}, {150, 150, 150, "no grind (curb, steps)"}, {255, 150, 30, "moving"},
                             {255, 230, 140, "fallback floor (gap fill)"}, {255, 60, 200, "live GTA collision (not baked)"},
                             {160, 255, 240, "high-res park (visible model)"}, {255, 170, 60, "high-res park: cut GTA piece"}};
constexpr Colour kRail{255, 240, 0, "grind rail"};
constexpr Colour kSkitch{80, 140, 255, "skitch grab line"};
// Lifted off the surface so GTA's own geometry does not hide it.
constexpr float kLift = 0.015f;

void Line(const SvVec3& a, const SvVec3& b, const Colour& c, int alpha, float dz = kLift) {
    gta::Call<void>(gta::DRAW_LINE, a.x, a.y, a.z + dz, b.x, b.y, b.z + dz, c.r, c.g, c.b, alpha);
}

using Edge = std::array<long long, 7>;
struct EdgeHash {
    std::size_t operator()(const Edge& edge) const {
        std::size_t hash = 0;
        for (const auto value : edge) hash ^= std::hash<long long>{}(value) + 0x9e3779b9u + (hash << 6) + (hash >> 2);
        return hash;
    }
};
}  // namespace

void Start(HMODULE runtimeModule) {
    if (!runtimeModule) return;
    g_set = reinterpret_cast<SvSetCollisionViewFn>(GetProcAddress(runtimeModule, "sv_set_collision_view"));
    g_view = reinterpret_cast<SvCollisionViewFn>(GetProcAddress(runtimeModule, "sv_collision_view"));
    g_status = reinterpret_cast<WorldStatus>(GetProcAddress(runtimeModule, "sv_world_status"));
    if (g_set) g_set(g_on ? 1u : 0u);
}

void SetWorldState(const std::string& text) { g_worldState = text; }

void Set(bool on, float range) {
    g_range = range > 1.0f ? range : 1.0f;
    if (on == g_on) return;
    g_on = on;
    g_triCount = g_lineCount = 0;
    if (g_set) g_set(on ? 1u : 0u);
}

void Tick(int ped) {
    if (!g_on || !g_view || !ped) return;
    if (g_frame++ % kRefresh == 0) {
        const Vector3 p = gta::Call<Vector3>(gta::GET_ENTITY_COORDS, ped, true);
        g_triCount = g_view({p.x, p.y, p.z}, g_range, g_tris.data(), kTris, g_lines.data(), kLines, &g_lineCount);
        if (g_lineCount > kLines) g_lineCount = kLines;
        if (g_triCount > kTris) g_triCount = kTris;
    }
    // Host bodies come in their entity's model space: placed on the entity as
    // GTA has it this frame, so the drawing stays on the model while it moves.
    std::uint32_t lastEntity = 0;
    Vector3 fwd{}, right{}, up{}, pos{};
    bool placed = false;
    auto onEntity = [&](const SvVec3& p) {
        return SvVec3{pos.x + right.x * p.x + fwd.x * p.y + up.x * p.z, pos.y + right.y * p.x + fwd.y * p.y + up.y * p.z,
                      pos.z + right.z * p.x + fwd.z * p.y + up.z * p.z};
    };
    // Adjacent triangles share edges. Draw each coloured world-space edge
    // once instead of issuing the same DRAW_LINE native for both triangles.
    static std::unordered_set<Edge, EdgeHash> edges;
    edges.clear();
    auto edge = [&](const SvVec3& a, const SvVec3& b, const Colour& c, std::uint32_t kind) {
        const auto point = [](const SvVec3& p) {
            return std::array<long long, 3>{std::llround(p.x * 10000.0), std::llround(p.y * 10000.0), std::llround(p.z * 10000.0)};
        };
        auto from = point(a), to = point(b);
        if (to < from) std::swap(from, to);
        if (edges.insert({from[0], from[1], from[2], to[0], to[1], to[2], kind}).second) Line(a, b, c, 200);
    };
    for (std::uint32_t i = 0; i < g_triCount; ++i) {
        SvViewTri t = g_tris[i];
        if (t.entity) {
            if (t.entity != lastEntity) {
                lastEntity = t.entity;
                const int e = static_cast<int>(t.entity);
                placed = gta::Call<BOOL>(gta::DOES_ENTITY_EXIST, e) != 0;
                if (placed) gta::Call<void>(gta::GET_ENTITY_MATRIX, e, &fwd, &right, &up, &pos);
            }
            if (!placed) continue;
            t.a = onEntity(t.a), t.b = onEntity(t.b), t.c = onEntity(t.c);
        }
        const Colour& c = kKinds[t.kind < 10 ? t.kind : 2];
        // See-through faces (both sides) and solid edges: the edges show the boxes.
        gta::Call<void>(gta::DRAW_POLY, t.a.x, t.a.y, t.a.z + kLift, t.b.x, t.b.y, t.b.z + kLift, t.c.x, t.c.y, t.c.z + kLift, c.r, c.g, c.b, 60);
        gta::Call<void>(gta::DRAW_POLY, t.c.x, t.c.y, t.c.z + kLift, t.b.x, t.b.y, t.b.z + kLift, t.a.x, t.a.y, t.a.z + kLift, c.r, c.g, c.b, 60);
        edge(t.a, t.b, c, t.kind);
        edge(t.b, t.c, c, t.kind);
        edge(t.c, t.a, c, t.kind);
    }
    for (std::uint32_t i = 0; i < g_lineCount; ++i) {
        // Three stacked lines read as a thick rail.
        const Colour& c = g_lines[i].kind == 1 ? kSkitch : kRail;
        for (float dz : {0.02f, 0.035f, 0.05f}) Line(g_lines[i].a, g_lines[i].b, c, 255, dz);
    }
    const auto now = GetTickCount64();
    if (g_status && (!g_statusAsked || now - g_statusAt >= 1000)) {
        char text[1024]{};
        g_tileStatus = g_status(text, sizeof(text)) ? text : "";
        g_statusAt = now;
        g_statusAsked = true;
    }
    float y = 0.25f;
    gta::Text(g_worldState.c_str(), 0.45f, y, 0.30f, 255, 255, 255);
    gta::Text(g_tileStatus.c_str(), 0.45f, y += 0.022f, 0.30f, 255, 255, 255);
    y += 0.022f;
    gta::Text("Collision view", 0.80f, y, 0.32f, 255, 255, 255);
    for (unsigned kind = 0; kind < 10; ++kind) {
        if (kind == 6) continue; // Legacy faces can still draw, but are no longer produced.
        const auto& c = kKinds[kind];
        gta::Text(c.name, 0.80f, y += 0.022f, 0.30f, c.r, c.g, c.b);
    }
    gta::Text(kRail.name, 0.80f, y += 0.022f, 0.30f, kRail.r, kRail.g, kRail.b);
    gta::Text(kSkitch.name, 0.80f, y += 0.022f, 0.30f, kSkitch.r, kSkitch.g, kSkitch.b);
}

}  // namespace collisionview
