#include "replay_logic.h"

#include <algorithm>
#include <cmath>
#include <iterator>
#include <utility>

namespace replay {

void Recording::SetWindow(double seconds) {
    window_ = std::clamp(seconds, 1.0, 120.0);
}

void Recording::Reserve(double seconds, double rate, int bones) {
    SetWindow(seconds);
    const std::size_t n = static_cast<std::size_t>(std::ceil(window_ * std::max(rate, 1.0))) + 1;
    slots_.assign(n, Frame{});
    for (Frame& f : slots_) f.bones.reserve(static_cast<std::size_t>(std::max(bones, 0)) * 16);
    Clear();
}

void Recording::Clear() {
    head_ = count_ = 0;
    clock_ = 0.0;
}

double Recording::Start() const { return count_ ? At(std::size_t{0}).time : 0.0; }
double Recording::End() const { return count_ ? At(count_ - 1).time : 0.0; }

void Recording::Add(const Frame& f, double dt) {
    if (count_ && At(std::size_t{0}).bones.size() != f.bones.size()) Clear();
    clock_ += count_ && dt > 0.0 && dt < 1.0 ? dt : 0.0;
    // Drop frames older than the window, keeping one before it for scrubs.
    while (count_ > 1 && At(std::size_t{1}).time <= clock_ - window_) {
        head_ = (head_ + 1) % slots_.size();
        --count_;
    }
    if (slots_.empty()) return;  // Reserve first
    if (count_ == slots_.size()) {  // full: the oldest frame goes
        head_ = (head_ + 1) % slots_.size();
        --count_;
    }
    Frame& slot = slots_[(head_ + count_) % slots_.size()];
    static_cast<FrameData&>(slot) = f;  // the bones into the slot's own room
    slot.bones.assign(f.bones.begin(), f.bones.end());
    slot.time = clock_;
    ++count_;
}

Recording::Sample Recording::At(double time) const {
    Sample s;
    if (!count_) return s;
    // Binary search over the ring by time.
    std::size_t lo = 0, hi = count_ - 1;
    if (time <= At(lo).time) {
        s.a = s.b = &At(lo);
        return s;
    }
    if (time >= At(hi).time) {
        s.a = s.b = &At(hi);
        return s;
    }
    while (hi - lo > 1) {
        const std::size_t mid = (lo + hi) / 2;
        (At(mid).time <= time ? lo : hi) = mid;
    }
    s.a = &At(lo);
    s.b = At(hi).cut ? s.a : &At(hi);
    const double span = At(hi).time - At(lo).time;
    s.t = s.b == s.a || span <= 0.0 ? 0.0f : static_cast<float>((time - At(lo).time) / span);
    return s;
}

void Playhead::Advance(double dt, double start, double end) {
    if (playing && dt > 0.0 && dt < 1.0) time += (reverse ? -dt : dt) * kSpeeds[speed];
    if (time <= start) {
        time = start;
        if (reverse) playing = false;
    } else if (time >= end) {
        time = end;
        if (!reverse) playing = false;
    }
}

bool Tap::Update(bool down, bool otherDown, unsigned nowMs) {
    if (down && !down_) {
        down_ = true;
        spoiled_ = otherDown;
        since_ = nowMs;
        return false;
    }
    if (down) {
        if (otherDown || nowMs - since_ > tapMs_) spoiled_ = true;
        return false;
    }
    if (down_) {
        down_ = false;
        return !spoiled_ && nowMs - since_ <= tapMs_;
    }
    return false;
}

float StickAxis(std::int16_t v) {
    const float x = std::max(-1.0f, v / 32767.0f);
    return std::fabs(x) < 0.2f ? 0.0f : x;
}

void ExitMask::Apply(PadIn& pad) {
    buttons_ &= pad.buttons;
    lt_ = lt_ && pad.leftTrigger > 30;
    rt_ = rt_ && pad.rightTrigger > 30;
    pad.buttons &= static_cast<std::uint16_t>(~buttons_);
    if (lt_) pad.leftTrigger = 0;
    if (rt_) pad.rightTrigger = 0;
}

Command Input::Update(const PadIn& pad, const KeysIn& in, float dt, bool arrowsMove, bool paused) {
    // Keys not read this frame (focus lost, a menu has them) keep their last
    // state: a Backspace held into an alt-tab is no release.
    KeysIn keys = in.read ? in : prevKeys_;
    if (!in.read) keys.wheelUp = keys.wheelDown = false, keys.mouseX = keys.mouseY = 0.0f;
    if (std::exchange(fresh_, false)) prevPad_ = pad.buttons, prevKeys_ = keys;
    const std::uint16_t pressed = pad.buttons & ~prevPad_;
    const std::uint16_t released = prevPad_ & ~pad.buttons;
    const auto down = [&](bool KeysIn::*k) { return keys.*k && !(prevKeys_.*k); };
    const auto up = [&](bool KeysIn::*k) { return !(keys.*k) && prevKeys_.*k; };
    const auto clamp1 = [](float v) { return std::clamp(v, -1.0f, 1.0f); };
    Command c;
    // B or Back (Backspace) leave on the release, so the press never
    // reaches skating afterwards.
    c.leave = (released & (0x2000 | 0x0020)) || up(&KeysIn::leave);
    c.playPause = (pressed & 0x1000) || down(&KeysIn::play);
    // Speed: LB/RB, the wheel, + and -; the Up/Down arrows too, except in the
    // Free and Orbit cameras, where they move the camera up and down.
    // Paused, LB/RB step one frame instead (as Skate's editor does).
    c.slower = (!paused && (pressed & 0x0100)) || (!arrowsMove && down(&KeysIn::slower)) || down(&KeysIn::minus) || keys.wheelDown;
    c.faster = (!paused && (pressed & 0x0200)) || (!arrowsMove && down(&KeysIn::faster)) || down(&KeysIn::plus) || keys.wheelUp;
    if ((paused && (pressed & 0x0100)) || down(&KeysIn::stepBack)) c.frameStep = -1;
    if ((paused && (pressed & 0x0200)) || down(&KeysIn::stepForward)) c.frameStep = 1;
    c.hideHud = (pressed & 0x0080) || down(&KeysIn::hide);
    c.zoom = static_cast<float>(keys.zoomIn) - static_cast<float>(keys.zoomOut);
    c.openEditor = (pressed & 0x0040) || down(&KeysIn::keyT);
    // Filters: F (Shift+F back), X, D-pad left/right. Lenses: L (Shift+L back), D-pad up/down.
    if (down(&KeysIn::filter)) c.filterStep = keys.shift ? -1 : 1;
    if (pressed & 0x4000) c.filterStep = 1;
    if (pressed & 0x0008) c.filterStep = 1;
    if (pressed & 0x0004) c.filterStep = -1;
    if (down(&KeysIn::lens)) c.lensStep = keys.shift ? -1 : 1;
    if (pressed & 0x0001) c.lensStep = 1;
    if (pressed & 0x0002) c.lensStep = -1;
    c.nextCamera = (pressed & 0x8000) || down(&KeysIn::camera);
    c.reverse = pad.leftTrigger > 128 || keys.reverse;
    const float side = static_cast<float>(keys.right) - static_cast<float>(keys.left);
    c.scrub = clamp1(StickAxis(pad.leftX) + side);
    c.move[0] = clamp1(StickAxis(pad.leftX) + side);
    c.move[1] = clamp1(StickAxis(pad.leftY) + static_cast<float>(keys.forward) - static_cast<float>(keys.back));
    const float arrows = arrowsMove ? static_cast<float>(keys.faster) - static_cast<float>(keys.slower) : 0.0f;
    c.move[2] = clamp1((pad.rightTrigger - pad.leftTrigger) / 255.0f + static_cast<float>(keys.up) - static_cast<float>(keys.down) + arrows);
    // The right stick turns at 120 / 90 degrees a second; the mouse as in the
    // object dropper (8 / 6 degrees per look normal).
    c.lookYaw = -StickAxis(pad.rightX) * 120.0f * dt - keys.mouseX * 8.0f;
    c.lookPitch = StickAxis(pad.rightY) * 90.0f * dt - keys.mouseY * 6.0f;
    prevPad_ = pad.buttons;
    prevKeys_ = keys;
    prevLt_ = pad.leftTrigger > 128, prevRt_ = pad.rightTrigger > 128;
    return c;
}

Edges Input::Poll(const PadIn& pad, const KeysIn& in) {
    KeysIn keys = in.read ? in : prevKeys_;
    if (!in.read) keys.wheelUp = keys.wheelDown = false, keys.mouseX = keys.mouseY = 0.0f;
    const bool lt = pad.leftTrigger > 128, rt = pad.rightTrigger > 128;
    if (std::exchange(fresh_, false)) prevPad_ = pad.buttons, prevKeys_ = keys, prevLt_ = lt, prevRt_ = rt;
    Edges e;
    e.pad = pad.buttons & ~prevPad_;
    e.lt = lt && !prevLt_, e.rt = rt && !prevRt_;
    const auto edge = [&](bool KeysIn::*k) { e.key.*k = keys.*k && !(prevKeys_.*k); };
    for (bool KeysIn::*k : {&KeysIn::play, &KeysIn::faster, &KeysIn::slower, &KeysIn::camera, &KeysIn::leave, &KeysIn::reverse,
                            &KeysIn::left, &KeysIn::right, &KeysIn::arrowLeft, &KeysIn::arrowRight, &KeysIn::photo, &KeysIn::forward, &KeysIn::back, &KeysIn::up, &KeysIn::down,
                            &KeysIn::plus, &KeysIn::minus, &KeysIn::filter, &KeysIn::lens, &KeysIn::hide, &KeysIn::stepBack,
                            &KeysIn::stepForward, &KeysIn::zoomIn, &KeysIn::zoomOut, &KeysIn::tab, &KeysIn::backspace,
                            &KeysIn::enter, &KeysIn::del, &KeysIn::pageUp, &KeysIn::pageDown, &KeysIn::keyT, &KeysIn::keyN,
                            &KeysIn::keyI, &KeysIn::keyO, &KeysIn::keyU, &KeysIn::keyK, &KeysIn::keyJ, &KeysIn::keyZ,
                            &KeysIn::keyX, &KeysIn::keyV, &KeysIn::keyS, &KeysIn::keyE, &KeysIn::key1, &KeysIn::key2,
                            &KeysIn::key3, &KeysIn::key4, &KeysIn::keyF1, &KeysIn::keyF2, &KeysIn::keyF3, &KeysIn::keyF4,
                            &KeysIn::home, &KeysIn::endKey, &KeysIn::escape, &KeysIn::mouseLeft, &KeysIn::mouseRight})
        edge(k);
    e.key.shift = keys.shift;
    e.key.ctrl = keys.ctrl;
    e.key.alt = keys.alt;
    e.key.wheelUp = keys.wheelUp, e.key.wheelDown = keys.wheelDown;
    prevPad_ = pad.buttons;
    prevKeys_ = keys;
    prevLt_ = lt, prevRt_ = rt;
    return e;
}

const char* LensName(int lens) {
    static const char* const names[kLensCount] = {"Auto lens", "Fisheye", "14 mm", "24 mm", "35 mm", "50 mm", "85 mm", "135 mm"};
    return lens >= 0 && lens < kLensCount ? names[lens] : names[0];
}

float LensFov(int lens, float autoFov) {
    // 35 mm film: the frame is 24 mm high; GTA's field of view is vertical.
    static const float focal[kLensCount] = {0.0f, 0.0f, 14.0f, 24.0f, 35.0f, 50.0f, 85.0f, 135.0f};
    if (lens == kLensFisheye) return 120.0f;
    if (lens <= kLensAuto || lens >= kLensCount) return autoFov;
    return 2.0f * std::atan(12.0f / focal[lens]) * 57.29578f;
}

const char* FilterName(int filter) {
    static const char* const names[kFilterCount] = {
        "Off", "Black and white", "Film 1", "Film 2", "Film 3", "Film 4", "Film 5", "Film 6", "Film 7",
        "Film 8", "Film 9", "Film 10", "Film 11", "Film 12", "Film 13", "Film 14", "Film 15", "Film 16",
        "Film 17", "Film 18", "Film 19", "Film 20", "Film 21", "Film 22", "Film 23", "Film 24", "Film 25"};
    return filter >= 0 && filter < kFilterCount ? names[filter] : names[0];
}

const char* FilterModifier(int filter) {
    static const char* const mods[kFilterCount] = {
        nullptr, "blackNwhite", "NG_filmic01", "NG_filmic02", "NG_filmic03", "NG_filmic04", "NG_filmic05",
        "NG_filmic06", "NG_filmic07", "NG_filmic08", "NG_filmic09", "NG_filmic10", "NG_filmic11", "NG_filmic12",
        "NG_filmic13", "NG_filmic14", "NG_filmic15", "NG_filmic16", "NG_filmic17", "NG_filmic18", "NG_filmic19",
        "NG_filmic20", "NG_filmic21", "NG_filmic22", "NG_filmic23", "NG_filmic24", "NG_filmic25"};
    return filter > 0 && filter < kFilterCount ? mods[filter] : nullptr;
}

int Cycle(int index, int step, int count) {
    if (count <= 0) return 0;
    return ((index + step) % count + count) % count;
}

double GameMsAt(const Recording::Sample& s) {
    if (!s.a) return -1.0;
    if (s.b == s.a || !s.b) return s.a->gameMs;
    // Game time ran on while nothing was recorded (a menu, a pause): the sound
    // there is not this stretch's, so it is silence.
    const double step = static_cast<double>(s.b->gameMs) - s.a->gameMs;
    if (step < 0.0 || step > (s.b->time - s.a->time) * 1000.0 + 100.0) return -1.0;
    return s.a->gameMs + step * s.t;
}

double SoundRate(const Playhead& head, bool scrubbing) {
    if (!head.playing || head.reverse || scrubbing) return 0.0;
    return kSpeeds[head.speed];
}

std::string FormatSkateBadges(const std::string& in) {
    static const std::pair<const char*, const char*> kReplacements[] = {
        {"LB/RB", "[LB/RB]"},
        {"LT/RT", "[LT/RT]"},
        {"D-pad Left/Right", "[D-PAD \xE2\x86\x90\xE2\x86\x92]"},
        {"D-pad Down/Up", "[D-PAD \xE2\x86\x93\xE2\x86\x91]"},
        {"D-pad Up/Down", "[D-PAD \xE2\x86\x91\xE2\x86\x93]"},
        {"D-pad Adjust", "[D-PAD] Adjust"},
        {"D-pad", "[D-PAD]"},
        {"Left Stick", "[LS]"},
        {"Right Stick", "[RS]"},
        {"RB", "[RB]"},
        {"LB", "[LB]"},
        {"RT", "[RT]"},
        {"LT", "[LT]"},
        {"R3", "[RS]"},
        {"L3", "[LS]"},
        {"Back Actions", "[BACK] Actions"},
        {"Back Commit", "[BACK] Commit"},
        {"A", "[A]"},
        {"B", "[B]"},
        {"X", "[X]"},
        {"Y", "[Y]"},
        {"a", "[A]"},
        {"b", "[B]"},
        {"y", "[Y]"},
    };
    std::string s = in;
    for (const auto& rep : kReplacements) {
        const std::string from = rep.first;
        const std::string to = rep.second;
        std::size_t pos = 0;
        while ((pos = s.find(from, pos)) != std::string::npos) {
            const bool beforeOk = (pos == 0 || s[pos - 1] == ' ');
            const std::size_t end = pos + from.size();
            const bool afterOk = (end == s.size() || s[end] == ' ');
            if (beforeOk && afterOk) {
                s.replace(pos, from.size(), to);
                pos += to.size();
            } else {
                pos += from.size();
            }
        }
    }
    return s;
}

}  // namespace replay
