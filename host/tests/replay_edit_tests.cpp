// Skate 3's replay editor model (design 2026-11-09-skate-replay-editor).
#include "../src/replay_logic.cpp"
#include "../src/replay_edit.cpp"
#include "../src/replay_store.cpp"
#include "../src/modkeys.cpp"
#include "../src/replay_hud_state.h"
#include <cassert>
#include <cmath>
#include <iostream>
#include <limits>
#include <string>

using namespace replay;

int main() {
    Edit e;
    e.Reset(0.0, 30.0);
    assert(e.In() == 0.0 && e.Out() == 30.0 && e.Clips().size() == 1 && !e.Edited());
    assert(std::string(CameraKindName(CameraKind::AdvFree)) == "Advanced Free Camera");

    // Markers: sorted, kept 0.25 s from each other and from In/Out, at most 20.
    assert(e.PlaceMarker(10.0) == EditResult::Ok);
    assert(e.PlaceMarker(5.0) == EditResult::Ok);
    assert(e.PlaceMarker(5.1) == EditResult::TooClose);
    assert(e.PlaceMarker(0.1) == EditResult::TooClose);   // near In
    assert(e.PlaceMarker(29.9) == EditResult::TooClose);  // near Out
    assert(e.Markers().size() == 2 && e.Markers()[0] == 5.0 && e.Markers()[1] == 10.0);
    assert(e.Clips().size() == 3 && e.Edited());
    assert(e.ClipAt(2.0) == 0 && e.ClipAt(5.0) == 1 && e.ClipAt(7.0) == 1 && e.ClipAt(12.0) == 2);
    assert(e.ClipStart(1) == 5.0 && e.ClipEnd(1) == 10.0 && e.ClipEnd(2) == 30.0);
    assert(e.PrevStop(7.0) == 5.0 && e.NextStop(7.0) == 10.0 && e.PrevStop(3.0) == 0.0 && e.NextStop(12.0) == 30.0);
    {
        Edit m;
        m.Reset(0.0, 30.0);
        for (int i = 0; i < Edit::kMaxMarkers; ++i) assert(m.PlaceMarker(1.0 + i) == EditResult::Ok);
        assert(m.PlaceMarker(25.0) == EditResult::TooMany);
    }

    // Cameras: a different camera asks first once the clip has one; keys only on Advanced cameras.
    assert(e.SetCamera(1, CameraKind::Follow, false) == EditResult::Ok);  // from Game: no question
    assert(e.SetCamera(1, CameraKind::AdvTripod, false) == EditResult::NeedsConfirm);
    assert(e.SetCamera(1, CameraKind::AdvTripod, true) == EditResult::Ok);
    assert(e.AddKey(2.0, CameraParams{}) == EditResult::NotFound);  // clip 0 is a Game camera
    CameraParams a, b;
    a.yaw = 170.0f, a.distance = 2.0f;
    b.yaw = -170.0f, b.distance = 6.0f;
    assert(e.AddKey(8.0, b) == EditResult::Ok);
    assert(e.AddKey(6.0, a) == EditResult::Ok);
    assert(e.ClipRef(1).keys.size() == 2 && e.ClipRef(1).keys[0].time == 6.0);  // sorted
    assert(e.AddKey(6.02, a) == EditResult::Ok && e.ClipRef(1).keys.size() == 2);  // replaces the near key
    // Interpolation: held before/after, eased between, the yaw the short way round (180, not 0).
    assert(e.Evaluate(5.5).p.distance == 2.0f && e.Evaluate(9.0).p.distance == 6.0f);
    const Edit::Frame mid = e.Evaluate(7.0);
    assert(mid.clip == 1 && mid.camera == CameraKind::AdvTripod && std::fabs(mid.p.distance - 4.0f) < 1e-4f);
    assert(std::fabs(std::fabs(std::remainder(mid.p.yaw, 360.0f)) - 180.0f) < 1e-3f);
    assert(e.PrevKey(7.0) == 6.0 && e.NextKey(7.0) == 8.0 && e.NextKey(9.0) == 9.0);
    assert(e.DeleteKey(7.0, 0.5) == EditResult::NotFound && e.DeleteKey(7.9, 0.5) == EditResult::Ok);
    assert(e.SetCamera(1, CameraKind::Free, false) == EditResult::NeedsConfirm);  // has a key

    // A marker splits a clip: both halves keep its camera and effects, keys on their side.
    e.ClipRef(1).speed = 0;  // 1/4x
    e.ClipRef(1).filterStrength = 40;
    assert(e.PlaceMarker(7.0) == EditResult::Ok);  // splits clip 1 (5..10)
    assert(e.Clips().size() == 4 && e.ClipRef(1).camera == CameraKind::AdvTripod && e.ClipRef(2).camera == CameraKind::AdvTripod);
    assert(e.ClipRef(1).keys.size() == 1 && e.ClipRef(2).keys.empty() && e.ClipRef(2).speed == 0);
    assert(e.ClipRef(2).filterStrength == 40);

    // Speeds weight the edited length: 0-5 at 1x, 5-10 at 1/4x, 10-30 at 1x = 5 + 20 + 20.
    assert(std::fabs(e.EditedLength() - 45.0) < 1e-9);
    assert(e.Evaluate(6.0).speed == 0.25f && e.Evaluate(20.0).speed == 1.0f);

    // Deleting a marker joins two clips; camera and keys reset, the left clip's effects stay.
    assert(e.DeleteMarker(7.1, 0.5, false) == EditResult::NeedsConfirm);
    assert(e.DeleteMarker(7.1, 0.5, true) == EditResult::Ok);
    assert(e.Clips().size() == 3 && e.ClipRef(1).camera == CameraKind::Game && e.ClipRef(1).keys.empty());
    assert(e.ClipRef(1).speed == 0 && e.ClipRef(1).filterStrength == 40);
    assert(e.DeleteMarker(20.0, 0.5, true) == EditResult::NotFound);

    // In/Out: markers outside go with their clips; too close to a marker is refused.
    assert(e.SetIn(4.9) == EditResult::TooClose);  // 0.1 s before the marker at 5
    assert(e.SetIn(6.0) == EditResult::Ok);  // the marker at 5 goes
    assert(e.In() == 6.0 && e.Markers().size() == 1 && e.Clips().size() == 2 && e.ClipRef(0).speed == 0);
    assert(e.SetOut(10.1) == EditResult::TooClose);  // 0.1 s after the marker at 10
    assert(e.SetOut(9.0) == EditResult::Ok);  // the marker at 10 goes
    assert(e.Out() == 9.0 && e.Markers().empty() && e.Clips().size() == 1);
    assert(e.ClipAt(1.0) == 0 && e.ClipAt(50.0) == 0);
    assert(std::fabs(e.EditedLength() - 12.0) < 1e-9);  // 3 s at 1/4x
    assert(e.SetOut(1.0) == EditResult::Ok && e.Out() == e.In() + Edit::kMinGap);  // clamped after In

    {
        // Keys outside a new In/Out go with the cut part.
        Edit k;
        k.Reset(0.0, 10.0);
        k.SetCamera(0, CameraKind::AdvFree, true);
        k.AddKey(1.0, CameraParams{});
        k.AddKey(5.0, CameraParams{});
        k.AddKey(9.0, CameraParams{});
        assert(k.SetIn(2.0) == EditResult::Ok && k.SetOut(8.0) == EditResult::Ok);
        assert(k.ClipRef(0).keys.size() == 1 && k.ClipRef(0).keys[0].time == 5.0);
    }

    // Reset clears the editing.
    e.Reset(0.0, 10.0);
    assert(e.Clips().size() == 1 && e.Markers().empty() && !e.Edited());
    assert(ShakeAmplitude(3) == 1.2f && ShakeAmplitude(0) == 0.0f && ShakeAmplitude(9) == 0.0f);

    {
        // Saved replays: everything written comes back; another file is refused.
        SavedReplay s;
        s.modelHash = 0x705E61F2u;
        s.savedUnix = 1762700000ull;
        for (int i = 0; i < 3; ++i) {
            Frame f;
            f.time = i / 60.0;
            f.root[0] = 1.0f + i, f.camera[6] = 50.0f, f.gameMs = 1000u + 16u * i, f.board = i == 1, f.cut = i == 2;
            f.boardWorld[6][15] = 7.0f;
            f.bones.assign(2 * 16, 0.5f + i);
            s.frames.push_back(f);
        }
        s.in = 0.0, s.out = 2 / 60.0;
        s.markers = {1 / 60.0};
        Clip ca, c;
        ca.camera = CameraKind::AdvFree, ca.speed = 0, ca.shake = 3, ca.filter = 2, ca.lens = kLensFisheye;
        ca.filterStrength = 0;
        CameraParams kp;
        kp.yaw = 33.0f, kp.positioned = true, kp.position[2] = 9.0f;
        ca.keys.push_back(Key{0.005, kp});
        c.camera = CameraKind::Follow, c.base.distance = 6.5f;
        c.lens = kLens50;
        s.clips = {ca, c};
        s.soundStartMs = 990;
        s.pcm = {1, -2, 3};
        const std::vector<std::uint8_t> bytes = Serialize(s);
        SavedInfo info;
        assert(ReadInfo(std::vector<std::uint8_t>(bytes.begin(), bytes.begin() + kInfoBytes), info) && info.frames == 3);
        SavedReplay back;
        std::string err;
        assert(Deserialize(bytes, back, err));
        assert(back.modelHash == s.modelHash && back.savedUnix == s.savedUnix && back.frames.size() == 3);
        assert(back.frames[1].board && back.frames[2].cut && back.frames[2].gameMs == 1032u && back.frames[2].bones[31] == 2.5f);
        assert(back.frames[0].boardWorld[6][15] == 7.0f && back.frames[2].root[0] == 3.0f);
        assert(back.markers.size() == 1 && back.clips.size() == 2);
        assert(back.clips[0].camera == CameraKind::AdvFree && back.clips[0].shake == 3 && back.clips[0].speed == 0);
        assert(back.clips[0].filter == 2 && back.clips[0].lens == kLensFisheye);  // by name
        assert(back.clips[0].filterStrength == 0);
        assert(back.clips[1].lens == kLens50 && back.clips[1].filterStrength == -1);
        assert(back.clips[0].keys.size() == 1 && back.clips[0].keys[0].p.yaw == 33.0f && back.clips[0].keys[0].p.positioned);
        assert(back.clips[1].base.distance == 6.5f && back.soundStartMs == 990 && back.pcm.size() == 3 && back.pcm[1] == -2);
        // Convert the two clip tails to the published v1 layout. Each lens
        // name follows a 16-bit length and v1 ends the clip after the name.
        std::vector<std::uint8_t> v1 = bytes;
        v1[4] = 1;
        const auto legacyLens = [&](const std::string& name, bool keep) {
            const auto it = std::search(v1.begin(), v1.end(), name.begin(), name.end());
            assert(it != v1.end());
            v1.erase(it + name.size(), it + name.size() + sizeof(std::int32_t));
            if (!keep) {
                *(it - sizeof(std::uint16_t)) = 0;
                v1.erase(it, it + name.size());
            }
        };
        legacyLens("Fisheye", true);
        legacyLens("50 mm", false);
        assert(ReadInfo(std::vector<std::uint8_t>(v1.begin(), v1.begin() + kInfoBytes), info));
        assert(Deserialize(v1, back, err) && back.pcm == s.pcm);
        assert(back.clips[0].lens == kLensFisheye && back.clips[0].filterStrength == -1);
        assert(back.clips[1].lens == kLensAuto && back.clips[1].filterStrength == -1);
        std::vector<std::uint8_t> cut(bytes.begin(), bytes.end() - 3);
        assert(!Deserialize(cut, back, err));  // damaged
        std::vector<std::uint8_t> other = bytes;
        other[4] = 9;  // another version
        assert(!Deserialize(other, back, err) && err == "saved by another version");
        // Restore keeps a valid edit and refuses a broken one.
        Edit r;
        r.Restore(0.0, 10.0, 1.0, 9.0, {4.0}, {Clip{}, Clip{}});
        assert(r.In() == 1.0 && r.Markers().size() == 1);
        r.Restore(0.0, 10.0, 1.0, 9.0, {4.0}, {Clip{}});  // clips != markers + 1
        assert(r.In() == 0.0 && r.Markers().empty());
    }

    // Other mods' keys: hidden only while blocking, mouse buttons never.
    assert(!modkeys::Hidden('A', false) && modkeys::Hidden('A', true) && modkeys::Hidden(VK_F10, true));
    assert(!modkeys::Hidden(VK_LBUTTON, true) && !modkeys::Hidden(VK_RBUTTON, true) && !modkeys::Hidden(0, true));

    // Skate 3 Clip Editor HUD snapshot mapping + GTA/overlay gating.
    {
        Edit hudEdit;
        hudEdit.Reset(10.0, 20.0);
        assert(hudEdit.SetIn(11.0) == EditResult::Ok);
        assert(hudEdit.SetOut(19.0) == EditResult::Ok);
        assert(hudEdit.PlaceMarker(15.0) == EditResult::Ok);
        assert(hudEdit.SetCamera(1, CameraKind::AdvFollow, true) == EditResult::Ok);
        hudEdit.ClipRef(1).speed = 3;
        hudEdit.ClipRef(1).shake = 2;
        hudEdit.ClipRef(1).lens = kLens35;
        CameraParams kp;
        kp.zoomMm = 50.0f;
        assert(hudEdit.AddKey(16.0, kp) == EditResult::Ok);

        const SvReplayHudState gtaOff = BuildReplayHudState({false, ReplayHudScreen::Timeline, true, false, 10.0, 10.0, 15.0, 1, 0.0f, "Edit timeline", &hudEdit});
        assert(gtaOff.active == 0 && gtaOff.size == sizeof(SvReplayHudState));

        const SvReplayHudState helpOff = BuildReplayHudState({true, ReplayHudScreen::HiddenOverlay, false, false, 10.0, 10.0, 15.0, 1, 0.0f, "Help", &hudEdit});
        assert(helpOff.active == 0);

        const SvReplayHudState instant = BuildReplayHudState({true, ReplayHudScreen::Instant, true, false, 10.0, 10.0, 13.5, 0, 0.0f, "Instant replay", &hudEdit});
        assert(instant.active == 1 && instant.mode == 0 && (instant.flags & 1u) != 0 && (instant.flags & 16u) != 0);
        assert(instant.clip_count == 0 && std::fabs(instant.trim_in) < 1e-5f && std::fabs(instant.trim_out - 1.0f) < 1e-5f);
        assert(std::fabs(instant.playhead - 0.35f) < 1e-5f);
        assert(std::string(instant.title_utf8) == "Instant replay");
        assert(std::string(instant.encoded_time_utf8) == "0:03.5 / 0:10.0");

        const SvReplayHudState timeline = BuildReplayHudState({true, ReplayHudScreen::Timeline, true, true, 10.0, 10.0, 15.0, 1, 0.0f, "Edit timeline", &hudEdit});
        assert(timeline.active == 1 && timeline.mode == 1);
        assert((timeline.flags & (1u | 2u | 8u | 16u)) == (1u | 2u | 8u | 16u));
        assert(timeline.clip_count == 2 && timeline.marker_count == 1 && timeline.selected_clip == 1);
        assert(std::fabs(timeline.trim_in - 0.1f) < 1e-5f && std::fabs(timeline.trim_out - 0.9f) < 1e-5f);
        assert(timeline.speed_pct == 200 && timeline.shake_type == 2);

        const SvReplayHudState cam = BuildReplayHudState({true, ReplayHudScreen::CameraEdit, false, false, 10.0, 10.0, 16.05, 1, 50.0f, "Follow (Key)", &hudEdit});
        assert(cam.active == 1 && cam.mode == 2 && (cam.flags & 4u) != 0 && cam.selected_key == 0 && cam.key_count == 1);
        assert(std::string(cam.zoom_utf8) == "50mm");

        hudEdit.ClipRef(0).camera = CameraKind::Tripod;
        hudEdit.ClipRef(0).lens = kLensFisheye;
        const SvReplayHudState fishCam = BuildReplayHudState({true, ReplayHudScreen::CameraEdit, false, false, 10.0, 10.0, 12.0, 0, 50.0f, "Tripod Camera", &hudEdit});
        assert(fishCam.mode == 3 && std::string(fishCam.zoom_utf8) == "Fish Eye");

        const SvReplayHudState panel = BuildReplayHudState({true, ReplayHudScreen::Panel, false, false, 10.0, 10.0, 12.0, 0, 0.0f, "Finished", &hudEdit});
        assert(panel.active == 1 && panel.mode == 6 && (panel.flags & 16u) == 0);

        const double nanVal = std::numeric_limits<double>::quiet_NaN();
        const std::string longUtf8(62, 'A');
        const SvReplayHudState edge = BuildReplayHudState({true, ReplayHudScreen::Timeline, false, false, nanVal, 10.0, nanVal, 0, 0.0f, longUtf8 + "\xC3\xA4", &hudEdit});
        assert(edge.playhead == 0.0f && std::string(edge.title_utf8) == longUtf8);
    }

    std::cout << "replay edit tests passed\n";
    return 0;
}
