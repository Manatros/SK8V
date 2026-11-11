#include "../src/replay_logic.cpp"
#include "../src/pose_math.h"
#include <cassert>
#include <cmath>
#include <iostream>
#include <string>

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

    // Design 2026-11-09-replay-look-sound: lenses (vertical FOV of a 24 mm high frame).
    assert(LensFov(kLensAuto, 50.0f) == 50.0f);
    assert(LensFov(kLensFisheye, 50.0f) == 120.0f);
    assert(std::fabs(LensFov(kLens14, 0) - 81.2f) < 0.1f);
    assert(std::fabs(LensFov(kLens24, 0) - 53.1f) < 0.1f);
    assert(std::fabs(LensFov(kLens35, 0) - 37.8f) < 0.1f);
    assert(std::fabs(LensFov(kLens50, 0) - 27.0f) < 0.1f);
    assert(std::fabs(LensFov(kLens85, 0) - 16.1f) < 0.1f);
    assert(std::fabs(LensFov(kLens135, 0) - 10.2f) < 0.1f);
    assert(Cycle(0, -1, kLensCount) == kLensCount - 1 && Cycle(kLensCount - 1, 1, kLensCount) == 0);
    // Filters: Off, black and white, Film 1..25 = NG_filmic01..25.
    assert(FilterModifier(0) == nullptr && std::string(FilterName(0)) == "Off");
    assert(std::string(FilterModifier(1)) == "blackNwhite");
    assert(std::string(FilterModifier(2)) == "NG_filmic01" && std::string(FilterName(2)) == "Film 1");
    assert(std::string(FilterModifier(kFilterCount - 1)) == "NG_filmic25" && std::string(FilterName(kFilterCount - 1)) == "Film 25");
    assert(Cycle(kFilterCount - 1, 1, kFilterCount) == 0);
    {
        // Free and Orbit: the Up/Down arrows move the camera, not the speed;
        // elsewhere they stay the speed keys. + and - are the speed everywhere.
        Input in;
        KeysIn k;
        in.Update(PadIn{}, k, 0.016f, true);  // first frame: held state only
        k.faster = true;                       // Up arrow
        Command c = in.Update(PadIn{}, k, 0.016f, true);
        assert(!c.faster && c.move[2] == 1.0f);
        k = KeysIn{};
        k.slower = true;                       // Down arrow
        c = in.Update(PadIn{}, k, 0.016f, true);
        assert(!c.slower && c.move[2] == -1.0f);
        k = KeysIn{};
        in.Update(PadIn{}, k, 0.016f, false);
        k.faster = true;
        c = in.Update(PadIn{}, k, 0.016f, false);
        assert(c.faster && c.move[2] == 0.0f);
        k = KeysIn{};
        in.Update(PadIn{}, k, 0.016f, true);
        k.plus = true;
        c = in.Update(PadIn{}, k, 0.016f, true);
        assert(c.faster);
        k = KeysIn{};
        k.minus = true;
        c = in.Update(PadIn{}, k, 0.016f, true);
        assert(c.slower);
        // F next, Shift+F back; L next, Shift+L back (edges only).
        k = KeysIn{};
        in.Update(PadIn{}, k, 0.016f);
        k.filter = true;
        c = in.Update(PadIn{}, k, 0.016f);
        assert(c.filterStep == 1);
        c = in.Update(PadIn{}, k, 0.016f);
        assert(c.filterStep == 0);  // held: once
        k = KeysIn{};
        in.Update(PadIn{}, k, 0.016f);
        k.filter = k.shift = true;
        c = in.Update(PadIn{}, k, 0.016f);
        assert(c.filterStep == -1);
        k = KeysIn{};
        in.Update(PadIn{}, k, 0.016f);
        k.lens = true;
        c = in.Update(PadIn{}, k, 0.016f);
        assert(c.lensStep == 1);
        // Controller: X and D-pad right/left filter, D-pad up/down lens.
        k = KeysIn{};
        PadIn p;
        in.Update(p, k, 0.016f);
        p.buttons = 0x4000;
        assert(in.Update(p, k, 0.016f).filterStep == 1);
        p.buttons = 0;
        in.Update(p, k, 0.016f);
        p.buttons = 0x0004;
        assert(in.Update(p, k, 0.016f).filterStep == -1);
        p.buttons = 0;
        in.Update(p, k, 0.016f);
        p.buttons = 0x0001;
        assert(in.Update(p, k, 0.016f).lensStep == 1);
        p.buttons = 0;
        in.Update(p, k, 0.016f);
        p.buttons = 0x0002;
        assert(in.Update(p, k, 0.016f).lensStep == -1);
    }
    {
        // Paused, LB/RB step one frame (no speed change); playing they are the speed.
        Input in;
        PadIn p;
        KeysIn k;
        in.Update(p, k, 0.016f, false, true);
        p.buttons = 0x0200;  // RB
        Command c = in.Update(p, k, 0.016f, false, true);
        assert(c.frameStep == 1 && !c.faster);
        p.buttons = 0;
        in.Update(p, k, 0.016f, false, true);
        p.buttons = 0x0100;  // LB
        c = in.Update(p, k, 0.016f, false, true);
        assert(c.frameStep == -1 && !c.slower);
        p.buttons = 0;
        in.Update(p, k, 0.016f, false, false);
        p.buttons = 0x0200;
        c = in.Update(p, k, 0.016f, false, false);
        assert(c.faster && c.frameStep == 0);
        // , and . step on the keyboard; H and the right stick's click hide the HUD; [ ] zoom.
        p = PadIn{};
        in.Update(p, k, 0.016f);
        k.stepForward = true;
        assert(in.Update(p, k, 0.016f).frameStep == 1);
        k = KeysIn{};
        in.Update(p, k, 0.016f);
        k.hide = true;
        assert(in.Update(p, k, 0.016f).hideHud);
        assert(!in.Update(p, k, 0.016f).hideHud);  // held: once
        k = KeysIn{};
        in.Update(p, k, 0.016f);
        p.buttons = 0x0080;
        assert(in.Update(p, k, 0.016f).hideHud);
        p = PadIn{};
        k.zoomIn = true;
        assert(in.Update(p, k, 0.016f).zoom == 1.0f);
        k = KeysIn{};
        k.zoomOut = true;
        assert(in.Update(p, k, 0.016f).zoom == -1.0f);

        // Poll edge detection for direct keyboard/mouse shortcuts:
        Input pollIn;
        pollIn.Reset();
        KeysIn pk{};
        pk.read = true;
        PadIn pp{};
        pollIn.Poll(pp, pk);  // baseline frame establishes unpressed state
        pk.mouseLeft = true;
        pk.ctrl = true;
        pk.key1 = true;
        pk.keyE = true;
        Edges pe = pollIn.Poll(pp, pk);
        assert(pe.key.mouseLeft);
        assert(pe.key.ctrl);
        assert(pe.key.key1);
        assert(pe.key.keyE);
        assert(!pe.key.mouseRight);
        // Next frame, held keys do not trigger new edges, but level modifiers remain:
        pe = pollIn.Poll(pp, pk);
        assert(!pe.key.mouseLeft);
        assert(pe.key.ctrl);
        assert(!pe.key.key1);
        assert(!pe.key.keyE);
        // Release and press mouseRight & Escape:
        pk.mouseLeft = false;
        pk.mouseRight = true;
        pk.escape = true;
        pe = pollIn.Poll(pp, pk);
        assert(!pe.key.mouseLeft);
        assert(pe.key.mouseRight);
        assert(pe.key.escape);
    }
    {
        // Sound: game time between two frames; a gap (a menu was open) is silence.
        Recording s(30.0);
        s.Reserve(30.0, 60.0, 2);
        Frame f = At(0.0f);
        f.gameMs = 1000;
        s.Add(f, 0.0);
        f.gameMs = 1100;
        s.Add(f, 0.1);
        f.gameMs = 6000;  // 4.9 s of game time in 0.1 s of recording
        s.Add(f, 0.1);
        assert(std::fabs(GameMsAt(s.At(s.Start() + 0.05)) - 1050.0) < 1.0);
        assert(GameMsAt(s.At(s.Start() + 0.15)) < 0.0);
        assert(GameMsAt(s.At(s.Start() - 1.0)) == 1000.0);  // before the first frame: its time
        assert(GameMsAt(Recording::Sample{}) < 0.0);
        Playhead h;
        assert(SoundRate(h, false) == 1.0);
        h.speed = 0;
        assert(SoundRate(h, false) == 0.25);
        assert(SoundRate(h, true) == 0.0);  // scrubbing
        h.reverse = true;
        assert(SoundRate(h, false) == 0.0);
        h.reverse = false;
        h.playing = false;
        assert(SoundRate(h, false) == 0.0);
    }
    {
        // ActorSample and Frame world actors:
        Frame fa = At(1.0f);
        fa.vehicleCount = 2;
        fa.vehicles[0].handle = 101;
        fa.vehicles[0].pos[0] = 10.0f; fa.vehicles[0].pos[1] = 20.0f; fa.vehicles[0].pos[2] = 5.0f;
        fa.vehicles[0].quat[0] = 0.0f; fa.vehicles[0].quat[1] = 0.0f; fa.vehicles[0].quat[2] = 0.0f; fa.vehicles[0].quat[3] = 1.0f;
        fa.vehicles[1].handle = 102;
        fa.vehicles[1].pos[0] = 50.0f; fa.vehicles[1].pos[1] = 0.0f; fa.vehicles[1].pos[2] = 0.0f;
        fa.pedCount = 1;
        fa.peds[0].handle = 201;
        fa.peds[0].pos[0] = 2.0f; fa.peds[0].pos[1] = 3.0f; fa.peds[0].pos[2] = 1.0f;

        Frame fb = At(2.0f);
        fb.vehicleCount = 2;
        fb.vehicles[0].handle = 101;
        fb.vehicles[0].pos[0] = 20.0f; fb.vehicles[0].pos[1] = 20.0f; fb.vehicles[0].pos[2] = 5.0f;
        fb.vehicles[0].quat[0] = 0.0f; fb.vehicles[0].quat[1] = 0.0f; fb.vehicles[0].quat[2] = 0.7071068f; fb.vehicles[0].quat[3] = 0.7071068f;
        fb.vehicles[1].handle = 102;
        fb.vehicles[1].pos[0] = 60.0f; fb.vehicles[1].pos[1] = 0.0f; fb.vehicles[1].pos[2] = 0.0f;
        fb.pedCount = 1;
        fb.peds[0].handle = 201;
        fb.peds[0].pos[0] = 4.0f; fb.peds[0].pos[1] = 3.0f; fb.peds[0].pos[2] = 1.0f;

        Recording rec(10.0);
        rec.Reserve(10.0, 60.0, 2);
        rec.Add(fa, 0.0);
        rec.Add(fb, 0.5);
        auto samp = rec.At(0.25);
        assert(samp.a && samp.b);
        assert(samp.a->vehicleCount == 2);
        assert(samp.b->vehicleCount == 2);
        assert(samp.a->vehicles[0].handle == 101);
        assert(samp.b->vehicles[0].handle == 101);
        assert(std::fabs(samp.t - 0.5f) < 1e-6f);

        // Quaternion slerp test:
        posemath::Q qa{fa.vehicles[0].quat[0], fa.vehicles[0].quat[1], fa.vehicles[0].quat[2], fa.vehicles[0].quat[3]};
        posemath::Q qb{fb.vehicles[0].quat[0], fb.vehicles[0].quat[1], fb.vehicles[0].quat[2], fb.vehicles[0].quat[3]};
        posemath::Q qhalf = posemath::Slerp(qa, qb, 0.5f);
        // Angle should be half of 90 degrees = 45 degrees: sin(22.5) ~ 0.38268, cos(22.5) ~ 0.92388
        assert(std::fabs(qhalf.z - 0.3826834f) < 1e-4f);
        assert(std::fabs(qhalf.w - 0.9238795f) < 1e-4f);

        // HudStyle enums:
        assert(static_cast<int>(HudStyle::GTA) == 0);
        assert(static_cast<int>(HudStyle::Skate) == 1);

        // keyF4 edge detection:
        Input inF4;
        PadIn pad{};
        KeysIn keys{};
        keys.read = true;
        inF4.Poll(pad, keys);  // baseline establishes unpressed state
        keys.keyF4 = true;
        Edges ed = inF4.Poll(pad, keys);
        assert(ed.key.keyF4);
        // Next frame, still held: no new edge:
        ed = inF4.Poll(pad, keys);
        assert(!ed.key.keyF4);
        // Release:
        keys.keyF4 = false;
        ed = inF4.Poll(pad, keys);
        assert(!ed.key.keyF4);
        // Press again:
        keys.keyF4 = true;
        ed = inF4.Poll(pad, keys);
        assert(ed.key.keyF4);

        // FormatSkateBadges:
        assert(FormatSkateBadges("RB Play/Pause   LT/RT Scrub   D-pad Left/Right Start/End   A Edit Timeline") ==
               "[RB] Play/Pause   [LT/RT] Scrub   [D-PAD \xE2\x86\x90\xE2\x86\x92] Start/End   [A] Edit Timeline");
        assert(FormatSkateBadges("A Select   B Back") == "[A] Select   [B] Back");
        assert(FormatSkateBadges("Back Actions   B Cancel") == "[BACK] Actions   [B] Cancel");
        assert(FormatSkateBadges("LB/RB Step Frame") == "[LB/RB] Step Frame");
        assert(FormatSkateBadges("X Set Key   Y Delete Key   L3 Next Key") == "[X] Set Key   [Y] Delete Key   [LS] Next Key");
        assert(FormatSkateBadges("a open   y delete   b back") == "[A] open   [Y] delete   [B] back");

        // 48 vehicles + 32 peds maximum capacity test:
        Frame fFull = At(10.0f);
        fFull.vehicleCount = kMaxTrackedVehicles;
        assert(kMaxTrackedVehicles == 48);
        for (int v = 0; v < kMaxTrackedVehicles; ++v) {
            fFull.vehicles[v].handle = 1000 + v;
            fFull.vehicles[v].pos[0] = static_cast<float>(v);
            fFull.vehicles[v].quat[3] = 1.0f;
        }
        fFull.pedCount = kMaxTrackedPeds;
        assert(kMaxTrackedPeds == 32);
        for (int p = 0; p < kMaxTrackedPeds; ++p) {
            fFull.peds[p].handle = 2000 + p;
            fFull.peds[p].pos[1] = static_cast<float>(p);
            fFull.peds[p].quat[3] = 1.0f;
        }
        Recording recFull(5.0);
        recFull.Reserve(5.0, 60.0, 2);
        recFull.Add(fFull, 0.0);
        assert(recFull.Last().vehicleCount == 48);
        assert(recFull.Last().pedCount == 32);
        assert(recFull.Last().vehicles[47].handle == 1047);
        assert(recFull.Last().peds[31].handle == 2031);

        // ActorSpan computation & GetActorReplayState test:
        Recording recSpan(10.0);
        recSpan.Reserve(10.0, 60.0, 2);

        Frame f1 = At(1.0f); // t = 0.0
        f1.vehicleCount = 2;
        f1.vehicles[0].handle = 101; f1.vehicles[0].pos[0] = 10.0f;
        f1.vehicles[1].handle = 102; f1.vehicles[1].pos[0] = 50.0f;
        f1.pedCount = 0;

        Frame f2 = At(2.0f); // t = 0.5
        f2.vehicleCount = 2;
        f2.vehicles[0].handle = 101; f2.vehicles[0].pos[0] = 15.0f;
        f2.vehicles[1].handle = 102; f2.vehicles[1].pos[0] = 55.0f;
        f2.pedCount = 1;
        f2.peds[0].handle = 201; f2.peds[0].pos[0] = 5.0f;

        Frame f3 = At(3.0f); // t = 1.0
        f3.vehicleCount = 1;
        f3.vehicles[0].handle = 101; f3.vehicles[0].pos[0] = 20.0f;
        f3.pedCount = 1;
        f3.peds[0].handle = 201; f3.peds[0].pos[0] = 8.0f;

        Frame f4 = At(4.0f); // t = 1.5
        f4.vehicleCount = 0;
        f4.pedCount = 1;
        f4.peds[0].handle = 201; f4.peds[0].pos[0] = 12.0f;

        recSpan.Add(f1, 0.0);
        recSpan.Add(f2, 0.5);
        recSpan.Add(f3, 0.5);
        recSpan.Add(f4, 0.5);

        std::map<int, ActorSpan> spans = ComputeActorSpans(recSpan);
        assert(spans.size() == 3);

        // Vehicle 101: 0.0s .. 1.0s
        assert(spans[101].handle == 101 && !spans[101].isPed);
        assert(std::fabs(spans[101].firstTime - 0.0) < 1e-5);
        assert(std::fabs(spans[101].lastTime - 1.0) < 1e-5);
        assert(spans[101].firstSample.pos[0] == 10.0f);
        assert(spans[101].lastSample.pos[0] == 20.0f);

        // Vehicle 102: 0.0s .. 0.5s
        assert(spans[102].handle == 102 && !spans[102].isPed);
        assert(std::fabs(spans[102].firstTime - 0.0) < 1e-5);
        assert(std::fabs(spans[102].lastTime - 0.5) < 1e-5);
        assert(spans[102].firstSample.pos[0] == 50.0f);
        assert(spans[102].lastSample.pos[0] == 55.0f);

        // Ped 201: 0.5s .. 1.5s
        assert(spans[201].handle == 201 && spans[201].isPed);
        assert(std::fabs(spans[201].firstTime - 0.5) < 1e-5);
        assert(std::fabs(spans[201].lastTime - 1.5) < 1e-5);
        assert(spans[201].firstSample.pos[0] == 5.0f);
        assert(spans[201].lastSample.pos[0] == 12.0f);

        // Visibility resolution queries:
        // Vehicle 102 (span 0.0 .. 0.5):
        assert(GetActorReplayState(spans[102], -0.2) == ActorReplayState::BeforeSpan);
        assert(GetActorReplayState(spans[102], 0.0) == ActorReplayState::Active);
        assert(GetActorReplayState(spans[102], 0.25) == ActorReplayState::Active);
        assert(GetActorReplayState(spans[102], 0.5) == ActorReplayState::Active);
        assert(GetActorReplayState(spans[102], 0.8) == ActorReplayState::AfterSpan);

        // Ped 201 (span 0.5 .. 1.5):
        assert(GetActorReplayState(spans[201], 0.2) == ActorReplayState::BeforeSpan);
        assert(GetActorReplayState(spans[201], 0.5) == ActorReplayState::Active);
        assert(GetActorReplayState(spans[201], 1.0) == ActorReplayState::Active);
        assert(GetActorReplayState(spans[201], 1.5) == ActorReplayState::Active);
        assert(GetActorReplayState(spans[201], 1.8) == ActorReplayState::AfterSpan);
    }

    std::cout << "replay tests passed\n";
    return 0;
}
