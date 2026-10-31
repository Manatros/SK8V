// cl /std:c++20 /EHsc /W4 object_dropper_tests.cpp: the dropper's state without GTA.
#include "../src/object_dropper_logic.h"
#include <cassert>
#include <cmath>
#include <iostream>

namespace {
bool Near(float a, float b, float e = 1e-3f) { return std::fabs(a - b) <= e; }
}

int main() {
    using namespace dropper;
    State s;
    // Opens 5 m ahead of a camera looking north (+Y), facing the player.
    s.Open(3, 10.0f, 20.0f, 0.0f);
    assert(s.open && Near(s.x, 10.0f) && Near(s.y, 25.0f) && Near(s.heading, 180.0f));
    // Camera looking west (heading 90): ahead is -X.
    State w;
    w.Open(3, 0.0f, 0.0f, 90.0f);
    assert(Near(w.x, -5.0f) && Near(w.y, 0.0f));

    // Forward on the stick moves along the camera; right moves to its right (+X when looking north).
    Input in;
    in.moveY = 1.0f;
    s.Update(in, 0.0f, 10.0f, 20.0f, 0.5f, 0.0f);
    assert(Near(s.y, 27.0f) && Near(s.x, 10.0f));
    in = {};
    in.moveX = 1.0f;
    s.Update(in, 0.0f, 10.0f, 20.0f, 0.5f, 0.0f);
    assert(Near(s.x, 12.0f));

    // Never further than kMaxReach from the player.
    in = {};
    in.moveY = 1.0f;
    in.fast = true;
    for (int i = 0; i < 100; ++i) s.Update(in, 0.0f, 10.0f, 20.0f, 0.1f, 0.0f);
    const float dx = s.x - 10.0f, dy = s.y - 20.0f;
    assert(std::sqrt(dx * dx + dy * dy) <= kMaxReach + 1e-3f);

    // Turning right lowers the heading, wrapped to 0..360; lift is clamped.
    in = {};
    in.turn = 1.0f;
    s.heading = 10.0f;
    s.Update(in, 0.0f, 10.0f, 20.0f, 0.5f, 0.0f);
    assert(Near(s.heading, 310.0f));
    in = {};
    in.lift = -1.0f;
    s.Update(in, 0.0f, 10.0f, 20.0f, 1.0f, 0.0f);
    assert(Near(s.lift, -1.0f));
    for (int i = 0; i < 20; ++i) s.Update(in, 0.0f, 10.0f, 20.0f, 1.0f, 0.0f);
    assert(Near(s.lift, kMinLift));
    in.lift = 1.0f;
    for (int i = 0; i < 20; ++i) s.Update(in, 0.0f, 10.0f, 20.0f, 1.0f, 0.0f);
    assert(Near(s.lift, kMaxLift));

    // Object kinds wrap both ways.
    in = {};
    in.previous = true;
    s.Update(in, 0.0f, 10.0f, 20.0f, 0.0f, 0.0f);
    assert(s.kind == 2);
    in = {};
    in.next = true;
    s.Update(in, 0.0f, 10.0f, 20.0f, 0.0f, 0.0f);
    assert(s.kind == 0);

    // A moving/lowering/kind-switching drop resolves the final pose, not the
    // previous frame's ground or model. This also covers sloping ground.
    State moving;
    moving.Open(3, 0.0f, 0.0f, 0.0f);
    Input drop;
    drop.next = true;
    drop.moveX = 1.0f;
    drop.lift = -1.0f;
    drop.drop = true;
    int queries = 0;
    assert(moving.UpdateResolved(drop, 0.0f, 0.0f, 0.0f, 0.5f, [&](const State& pose) {
        ++queries;
        assert(pose.kind == 1 && Near(pose.x, 2.0f) && Near(pose.y, 5.0f) && Near(pose.lift, -0.5f));
        return pose.x + 10.0f * pose.kind + pose.lift;
    }) == Event::Dropped);
    assert(queries == 1 && moving.placed.back().kind == 1 && Near(moving.placed.back().z, 11.5f));
    assert(Near(moving.placed.back().x, moving.x) && Near(moving.placed.back().y, moving.y));

    // Dropping records kind, place, height and heading with fresh ids, up to the limit.
    in = {};
    in.drop = true;
    assert(s.Update(in, 0.0f, 10.0f, 20.0f, 0.0f, 3.5f) == Event::Dropped);
    assert(s.placed.size() == 1 && Near(s.placed[0].z, 3.5f) && s.placed[0].kind == 0 && s.placed[0].id == 1);
    for (std::size_t i = 1; i < kMaxObjects; ++i) assert(s.Update(in, 0.0f, 10.0f, 20.0f, 0.0f, 0.0f) == Event::Dropped);
    assert(s.Update(in, 0.0f, 10.0f, 20.0f, 0.0f, 0.0f) == Event::Full && s.placed.size() == kMaxObjects);

    // Picking up takes the nearest within reach only.
    State p;
    p.Open(1, 0.0f, 0.0f, 0.0f);
    p.placed = {{0, 0.0f, 5.5f, 0, 0, 7}, {0, 0.0f, 7.0f, 0, 0, 8}, {0, 40.0f, 0.0f, 0, 0, 9}};
    in = {};
    in.pickUp = true;
    assert(p.Update(in, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f) == Event::PickedUp);
    assert(p.placed.size() == 2 && p.placed[0].id == 8);
    p.x = 20.0f;
    assert(p.Update(in, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f) == Event::NothingNear && p.placed.size() == 2);
    in = {};
    in.clearAll = true;
    assert(p.Update(in, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f) == Event::Cleared && p.placed.empty());
    assert(p.Update(in, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f) == Event::NothingNear);

    // Closing keeps what was dropped.
    in = {};
    in.close = true;
    assert(s.Update(in, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f) == Event::Closed && !s.open && s.placed.size() == kMaxObjects);
    assert(s.Update(Input{}, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f) == Event::None);
    std::cout << "object dropper: all checks pass\n";
    return 0;
}
