#include "../src/replay_logic.cpp"
#include <cassert>
#include <cmath>
#include <iostream>

using namespace replay;

static Frame At(float x, int bones = 2, bool cut = false) {
    Frame f;
    f.root[0] = x;
    f.cut = cut;
    f.bones.assign(static_cast<std::size_t>(bones) * 16, x);
    return f;
}

int main() {
    // The window keeps the last 2 s (plus one frame before it for scrubs),
    // in slots made once (Skate ticks: 60 a second).
    Recording r(2.0);
    r.Reserve(2.0, 10.0, 2);
    for (int i = 0; i < 300; ++i) r.Add(At(static_cast<float>(i)), 0.1);
    assert(std::fabs(r.End() - 29.9) < 1e-6);
    assert(r.Length() <= 2.1 + 1e-6 && r.Length() >= 2.0 - 1e-6);
    assert(r.Size() <= 22);
    assert(r.Last().root[0] == 299.0f);
    assert(r.Capacity() == 21);  // 2 s at 10 frames a second, plus one

    // Interpolation between neighbours, clamped at both ends.
    Recording q(30.0);
    q.Reserve(30.0, 60.0, 3);
    q.Add(At(0), 0.0);
    q.Add(At(10), 0.5);
    q.Add(At(20), 0.5);
    auto s = q.At(0.25);
    assert(s.a->root[0] == 0.0f && s.b->root[0] == 10.0f && std::fabs(s.t - 0.5f) < 1e-6f);
    s = q.At(-1.0);
    assert(s.a == s.b && s.a->root[0] == 0.0f);
    s = q.At(99.0);
    assert(s.a == s.b && s.a->root[0] == 20.0f);
    // No blend across a cut (a teleport): the earlier frame holds.
    q.Add(At(500, 2, true), 0.5);
    s = q.At(1.25);
    assert(s.a->root[0] == 20.0f && s.b == s.a && s.t == 0.0f);
    // A new skeleton (other bone count) starts a new recording.
    q.Add(At(7, 3), 0.1);
    assert(q.Size() == 1 && q.Last().bones.size() == 48);
    // A long gap (pause, loading) adds no time.
    q.Add(At(8, 3), 5.0);
    assert(q.Length() == 0.0);

    // Playhead: speeds, ends and reverse.
    Playhead p;
    p.time = 1.0;
    p.Advance(0.5, 0.0, 2.0);
    assert(std::fabs(p.time - 1.5) < 1e-9);
    p.Slower();
    p.Advance(0.5, 0.0, 2.0);
    assert(std::fabs(p.time - 1.75) < 1e-9 && p.playing);
    p.Advance(0.6, 0.0, 2.0);
    assert(std::fabs(p.time - 2.0) < 1e-9 && !p.playing);  // stopped at the end
    p.Slower(); p.Slower(); p.Slower();
    assert(p.speed == 0);
    p.Faster(); p.Faster(); p.Faster(); p.Faster();
    assert(kSpeeds[p.speed] == 2.0f);
    p.playing = true;
    p.reverse = true;
    p.Advance(0.4, 0.0, 2.0);
    assert(std::fabs(p.time - 1.2) < 1e-9);
    p.Advance(10.0, 0.0, 2.0);  // a frame longer than 1 s (a pause) does not jump
    assert(std::fabs(p.time - 1.2) < 1e-9);
    p.Advance(0.9, 0.0, 2.0);
    assert(p.time == 0.0 && !p.playing);

    // Back: a quick tap opens the replay; a hold (the gun) or a chord does not.
    Tap t(250);
    assert(!t.Update(true, false, 1000));
    assert(t.Update(false, false, 1100));
    assert(!t.Update(true, false, 2000));
    assert(!t.Update(true, false, 2400));
    assert(!t.Update(false, false, 2500));  // held 500 ms: the gun
    assert(!t.Update(true, false, 3000));
    assert(!t.Update(true, true, 3050));    // Back+LB: the menu
    assert(!t.Update(false, false, 3100));
    assert(!t.Update(true, true, 4000));    // pressed during another button
    assert(!t.Update(false, false, 4050));
    // Leaving the replay with Back (or Back held after B): that press is no tap.
    t.Block();
    assert(!t.Update(true, false, 5000));
    assert(!t.Update(false, false, 5050));
    t.Block();
    assert(!t.Update(false, false, 6000));  // already released: just re-armed
    assert(!t.Update(true, false, 6100));
    assert(t.Update(false, false, 6200));   // the next tap opens it again

    // Input: the controller and the keyboard and mouse give the same commands.
    {
        Input in;
        PadIn pad;
        KeysIn keys;
        pad.buttons = 0x1000;  // A held on entry: no play/pause
        Command c = in.Update(pad, keys, 0.016f);
        assert(!c.playPause && !c.leave);
        pad.buttons = 0;
        c = in.Update(pad, keys, 0.016f);
        pad.buttons = 0x1000;
        assert(in.Update(pad, keys, 0.016f).playPause);
        assert(!in.Update(pad, keys, 0.016f).playPause);  // held: one edge
        pad.buttons = 0;
        in.Update(pad, keys, 0.016f);
        keys.play = true;
        assert(in.Update(pad, keys, 0.016f).playPause);   // Space
        keys.play = false;
        keys.camera = true;
        assert(in.Update(pad, keys, 0.016f).nextCamera);  // C
        keys.camera = false;
        keys.faster = true;
        assert(in.Update(pad, keys, 0.016f).faster);      // Up arrow or wheel
        keys.faster = false;
        // Backspace leaves on the release, like B and Back.
        keys.leave = true;
        assert(!in.Update(pad, keys, 0.016f).leave);
        keys.leave = false;
        assert(in.Update(pad, keys, 0.016f).leave);
        pad.buttons = 0x2000;
        assert(!in.Update(pad, keys, 0.016f).leave);
        pad.buttons = 0;
        assert(in.Update(pad, keys, 0.016f).leave);
        // Keys held when the replay opens do nothing until let go.
        in.Reset();
        keys.leave = true;
        in.Update(pad, keys, 0.016f);
        keys.leave = false;
        assert(in.Update(pad, keys, 0.016f).leave);  // its release still leaves (as before: B held on entry)
        // Axes: keys reach full throw, the stick's dead zone holds, both clamp.
        keys = KeysIn{};
        keys.right = true;
        keys.forward = true;
        keys.up = true;
        c = in.Update(pad, keys, 0.016f);
        assert(c.scrub == 1.0f && c.move[0] == 1.0f && c.move[1] == 1.0f && c.move[2] == 1.0f);
        pad.leftX = 32767;
        assert(in.Update(pad, keys, 0.016f).move[0] == 1.0f);
        keys = KeysIn{};
        pad = PadIn{};
        pad.leftX = 5000;  // inside the dead zone
        assert(in.Update(pad, keys, 0.016f).scrub == 0.0f);
        pad.leftX = -32768;
        assert(in.Update(pad, keys, 0.016f).scrub == -1.0f);
        pad = PadIn{};
        pad.leftTrigger = 200;
        c = in.Update(pad, keys, 0.016f);
        assert(c.reverse && c.move[2] < -0.7f);
        pad = PadIn{};
        keys.reverse = true;
        assert(in.Update(pad, keys, 0.016f).reverse);    // hold R
        // Look: mouse right turns right (yaw down), mouse up looks up, as the stick.
        keys = KeysIn{};
        keys.mouseX = 0.5f;
        keys.mouseY = -0.5f;
        c = in.Update(pad, keys, 0.016f);
        assert(c.lookYaw < 0.0f && c.lookPitch > 0.0f);
        keys = KeysIn{};
        pad.rightX = 32767;
        pad.rightY = 32767;
        c = in.Update(pad, keys, 0.5f);
        assert(std::fabs(c.lookYaw + 60.0f) < 1e-3f && std::fabs(c.lookPitch - 45.0f) < 1e-3f);
    }

    // Review 2026-11-09: wheel notches on consecutive frames are two steps;
    // keys not read (focus lost) are no release.
    {
        Input in;
        PadIn pad;
        KeysIn keys;
        in.Update(pad, keys, 0.016f);
        keys.wheelUp = true;
        assert(in.Update(pad, keys, 0.016f).faster);
        assert(in.Update(pad, keys, 0.016f).faster);
        keys = KeysIn{};
        keys.leave = true;
        in.Update(pad, keys, 0.016f);
        KeysIn unread;
        unread.read = false;
        assert(!in.Update(pad, unread, 0.016f).leave);  // alt-tab with Backspace held
        assert(!in.Update(pad, keys, 0.016f).leave);    // back, still held
        keys.leave = false;
        assert(in.Update(pad, keys, 0.016f).leave);     // the real release
    }
    // Held on leaving: masked until let go, then back to normal.
    {
        ExitMask m;
        PadIn held;
        held.buttons = 0x8000 | 0x1000;  // Y (Q) and A (Space)
        held.rightTrigger = 255;         // RT (C)
        m.Start(held);
        PadIn p = held;
        m.Apply(p);
        assert(p.buttons == 0 && p.rightTrigger == 0 && m.Active());
        p = PadIn{};
        p.buttons = 0x1000;              // Q and C let go, Space still held
        m.Apply(p);
        assert(p.buttons == 0);
        p = PadIn{};
        m.Apply(p);
        assert(!m.Active());
        p.buttons = 0x8000;
        p.rightTrigger = 200;
        m.Apply(p);
        assert(p.buttons == 0x8000 && p.rightTrigger == 200);  // a new press goes through
    }

    std::cout << "replay tests passed\n";
    return 0;
}
