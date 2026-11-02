#include "replay_logic.h"

#include <algorithm>
#include <cmath>
#include <iterator>

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

}  // namespace replay
