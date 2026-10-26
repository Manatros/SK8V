#pragma once
// The object dropper's state without GTA (contributed by GTAxSkate, B36):
// the ghost the player moves, turns and lifts, and the objects dropped.
// object_dropper.cpp turns this into GTA objects; tests/object_dropper_tests.cpp
// checks it without the game. Positions are GTA world metres, headings GTA
// degrees (0 = +Y, counter-clockwise).
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <vector>

namespace dropper {

constexpr std::size_t kMaxObjects = 24;   // Skate 3 keeps a small park per session too
constexpr float kMoveSpeed = 4.0f;        // m/s, the ghost on the ground
constexpr float kFastFactor = 2.5f;       // Shift / stick clicked
constexpr float kTurnSpeed = 120.0f;      // degrees/s
constexpr float kLiftSpeed = 1.0f;        // m/s
constexpr float kMaxLift = 4.0f;          // m above the ground under the ghost
constexpr float kMinLift = -4.0f;         // manual correction below GTA's ground query
constexpr float kMaxReach = 25.0f;        // m from the player
constexpr float kStartDistance = 5.0f;    // m ahead of the camera on opening
constexpr float kPickReach = 3.0f;        // m from the ghost to an object picked up

// One frame of the player's dropper input (edges already taken).
struct Input {
    float moveX = 0, moveY = 0;  // -1..1, right / forward of the camera
    float turn = 0;              // -1..1, positive turns clockwise
    float lift = 0;              // -1..1, positive raises
    bool fast = false;
    bool previous = false, next = false;  // object kind
    bool drop = false, pickUp = false, clearAll = false, close = false;
};

struct Placed {
    int kind = 0;
    float x = 0, y = 0, z = 0;  // z: the object's own origin, as created
    float heading = 0;
    std::uint32_t id = 0;       // stable across picks, for the GTA side
};

enum class Event { None, Dropped, PickedUp, Cleared, Full, NothingNear, Closed };

struct State {
    bool open = false;
    int kinds = 0, kind = 0;
    float x = 0, y = 0, heading = 0, lift = 0;
    std::vector<Placed> placed;
    std::uint32_t nextId = 1;

    // Opens ahead of the camera at (px, py) looking along camHeading.
    void Open(int kindCount, float px, float py, float camHeading) {
        kinds = kindCount;
        if (kinds <= 0) return;
        kind = std::clamp(kind, 0, kinds - 1);
        const float r = camHeading * 3.14159265f / 180.0f;
        x = px - std::sin(r) * kStartDistance;
        y = py + std::cos(r) * kStartDistance;
        heading = Wrap(camHeading + 180.0f);  // the ramp's run-up faces the player
        lift = 0;
        open = true;
    }

    static float Wrap(float deg) {
        deg = std::fmod(deg, 360.0f);
        return deg < 0 ? deg + 360.0f : deg;
    }

    // Constant-height convenience for callers without a ground query.
    Event Update(const Input& in, float camHeading, float px, float py, float dt, float groundZ) {
        return UpdateResolved(in, camHeading, px, py, dt, [groundZ](const State&) { return groundZ; });
    }

    // Resolve the object's origin only after this frame's kind, motion and lift
    // have been applied, so dropping uses the same pose as the ghost.
    template <typename Ground>
    Event UpdateResolved(const Input& in, float camHeading, float px, float py, float dt, Ground groundZ) {
        if (!open) return Event::None;
        if (in.close) {
            open = false;
            return Event::Closed;
        }
        if (in.next) kind = (kind + 1) % kinds;
        if (in.previous) kind = (kind + kinds - 1) % kinds;
        const float speed = kMoveSpeed * (in.fast ? kFastFactor : 1.0f) * dt;
        const float r = camHeading * 3.14159265f / 180.0f;
        // Camera forward (-sin, cos), right (cos, sin) in GTA's heading convention.
        x += (-std::sin(r) * in.moveY + std::cos(r) * in.moveX) * speed;
        y += (std::cos(r) * in.moveY + std::sin(r) * in.moveX) * speed;
        const float dx = x - px, dy = y - py, d = std::sqrt(dx * dx + dy * dy);
        if (d > kMaxReach) x = px + dx / d * kMaxReach, y = py + dy / d * kMaxReach;
        heading = Wrap(heading - in.turn * kTurnSpeed * (in.fast ? kFastFactor : 1.0f) * dt);
        lift = std::clamp(lift + in.lift * kLiftSpeed * dt, kMinLift, kMaxLift);
        if (in.clearAll) {
            if (placed.empty()) return Event::NothingNear;
            placed.clear();
            return Event::Cleared;
        }
        if (in.pickUp) {
            auto best = placed.end();
            float bestD = kPickReach * kPickReach;
            for (auto it = placed.begin(); it != placed.end(); ++it) {
                const float ex = it->x - x, ey = it->y - y, e = ex * ex + ey * ey;
                if (e <= bestD) bestD = e, best = it;
            }
            if (best == placed.end()) return Event::NothingNear;
            placed.erase(best);
            return Event::PickedUp;
        }
        if (in.drop) {
            if (placed.size() >= kMaxObjects) return Event::Full;
            const float z = groundZ(static_cast<const State&>(*this));
            placed.push_back({kind, x, y, z, heading, nextId++});
            return Event::Dropped;
        }
        return Event::None;
    }
};

}  // namespace dropper
