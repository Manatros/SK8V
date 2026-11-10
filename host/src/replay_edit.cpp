#include "replay_edit.h"

#include <algorithm>
#include <cmath>
#include <iterator>

#include "replay_logic.h"

namespace replay {

const char* CameraKindName(CameraKind k) {
    static const char* const names[] = {"Game Camera",           "Follow Camera",          "Tripod Camera",
                                        "Free Camera",           "Advanced Follow Camera", "Advanced Tripod Camera",
                                        "Advanced Free Camera"};
    const auto i = static_cast<std::size_t>(k);
    return i < std::size(names) ? names[i] : names[0];
}

float ShakeAmplitude(int shake) {
    static const float amp[] = {0.0f, 0.3f, 0.7f, 1.2f};
    return shake >= 0 && shake < 4 ? amp[shake] : 0.0f;
}

CameraParams Lerp(const CameraParams& a, const CameraParams& b, float t) {
    CameraParams r = a;
    auto mix = [t](float x, float y) { return x + (y - x) * t; };
    r.yaw = a.yaw + std::remainder(b.yaw - a.yaw, 360.0f) * t;
    r.pitch = mix(a.pitch, b.pitch);
    r.distance = mix(a.distance, b.distance);
    r.height = mix(a.height, b.height);
    r.target = mix(a.target, b.target);
    r.zoomMm = (a.zoomMm > 0.0f && b.zoomMm > 0.0f) ? mix(a.zoomMm, b.zoomMm) : (t < 0.5f ? a.zoomMm : b.zoomMm);
    for (int i = 0; i < 3; ++i) r.position[i] = mix(a.position[i], b.position[i]);
    r.positioned = a.positioned || b.positioned;
    return r;
}

// Catmull-Rom through four keys (the camera passes each key without
// stopping); angles unwrapped the short way round first.
CameraParams Spline(const CameraParams& p0, const CameraParams& p1, const CameraParams& p2, const CameraParams& p3, float t) {
    auto cr = [t](float a, float b, float c, float d) {
        const float t2 = t * t, t3 = t2 * t;
        return 0.5f * ((2.0f * b) + (-a + c) * t + (2.0f * a - 5.0f * b + 4.0f * c - d) * t2 + (-a + 3.0f * b - 3.0f * c + d) * t3);
    };
    CameraParams r = Lerp(p1, p2, t);  // flags and the zoom rule
    const float y1 = p1.yaw, y0 = y1 - std::remainder(y1 - p0.yaw, 360.0f), y2 = y1 + std::remainder(p2.yaw - y1, 360.0f),
                y3 = y2 + std::remainder(p3.yaw - p2.yaw, 360.0f);
    r.yaw = cr(y0, y1, y2, y3);
    r.pitch = cr(p0.pitch, p1.pitch, p2.pitch, p3.pitch);
    r.distance = std::max(0.5f, cr(p0.distance, p1.distance, p2.distance, p3.distance));
    r.height = cr(p0.height, p1.height, p2.height, p3.height);
    r.target = std::clamp(cr(p0.target, p1.target, p2.target, p3.target), 0.0f, 1.0f);
    for (int i = 0; i < 3; ++i) r.position[i] = cr(p0.position[i], p1.position[i], p2.position[i], p3.position[i]);
    return r;
}

void Edit::Reset(double start, double end) {
    start_ = start, end_ = std::max(end, start);
    in_ = start_, out_ = end_;
    markers_.clear();
    clips_.assign(1, Clip{});
    edited_ = false;
}

void Edit::Restore(double start, double end, double in, double out, const std::vector<double>& markers,
                   const std::vector<Clip>& clips) {
    Reset(start, end);
    bool ok = in >= start - 1e-9 && out <= end + 1e-9 && out - in >= kMinGap && clips.size() == markers.size() + 1 &&
              static_cast<int>(markers.size()) <= kMaxMarkers;
    for (std::size_t i = 0; ok && i < markers.size(); ++i)
        ok = markers[i] > in && markers[i] < out && (i == 0 || markers[i] > markers[i - 1]);
    if (!ok) return;
    in_ = in, out_ = out;
    markers_ = markers;
    clips_ = clips;
    edited_ = true;
}

EditResult Edit::SetIn(double t) {
    t = std::clamp(t, start_, out_ - kMinGap);
    // Markers at or before the new In go, with the clips left of them.
    std::size_t gone = 0;
    while (gone < markers_.size() && markers_[gone] <= t) ++gone;
    if (gone < markers_.size() && markers_[gone] - t < kMinGap) return EditResult::TooClose;
    markers_.erase(markers_.begin(), markers_.begin() + static_cast<std::ptrdiff_t>(gone));
    clips_.erase(clips_.begin(), clips_.begin() + static_cast<std::ptrdiff_t>(gone));
    auto& first = clips_.front().keys;  // keys before the new In go with the cut part
    first.erase(std::remove_if(first.begin(), first.end(), [t](const Key& k) { return k.time < t; }), first.end());
    in_ = t;
    edited_ = true;
    return EditResult::Ok;
}

EditResult Edit::SetOut(double t) {
    t = std::clamp(t, in_ + kMinGap, end_);
    std::size_t keep = markers_.size();
    while (keep > 0 && markers_[keep - 1] >= t) --keep;
    if (keep > 0 && t - markers_[keep - 1] < kMinGap) return EditResult::TooClose;
    const std::size_t gone = markers_.size() - keep;
    markers_.resize(keep);
    clips_.resize(clips_.size() - gone);
    auto& last = clips_.back().keys;
    last.erase(std::remove_if(last.begin(), last.end(), [t](const Key& k) { return k.time > t; }), last.end());
    out_ = t;
    edited_ = true;
    return EditResult::Ok;
}

EditResult Edit::PlaceMarker(double t) {
    if (static_cast<int>(markers_.size()) >= kMaxMarkers) return EditResult::TooMany;
    if (t - in_ < kMinGap || out_ - t < kMinGap) return EditResult::TooClose;
    for (double m : markers_)
        if (std::fabs(m - t) < kMinGap) return EditResult::TooClose;
    const auto at = std::upper_bound(markers_.begin(), markers_.end(), t);
    const std::size_t idx = static_cast<std::size_t>(at - markers_.begin());  // the clip being split
    markers_.insert(at, t);
    Clip right = clips_[idx];
    Clip& left = clips_[idx];
    std::vector<Key> leftKeys, rightKeys;
    for (const Key& k : left.keys) (k.time < t ? leftKeys : rightKeys).push_back(k);
    left.keys = std::move(leftKeys);
    right.keys = std::move(rightKeys);
    clips_.insert(clips_.begin() + static_cast<std::ptrdiff_t>(idx) + 1, std::move(right));
    edited_ = true;
    return EditResult::Ok;
}

EditResult Edit::DeleteMarker(double t, double within, bool confirmed) {
    int best = -1;
    double bestD = within;
    for (std::size_t i = 0; i < markers_.size(); ++i) {
        const double d = std::fabs(markers_[i] - t);
        if (d <= bestD) best = static_cast<int>(i), bestD = d;
    }
    if (best < 0) return EditResult::NotFound;
    if (!confirmed) return EditResult::NeedsConfirm;
    const auto i = static_cast<std::size_t>(best);
    Clip joined;  // camera and keys reset; the left clip's effects stay
    joined.speed = clips_[i].speed, joined.shake = clips_[i].shake;
    joined.filter = clips_[i].filter, joined.lens = clips_[i].lens;
    markers_.erase(markers_.begin() + best);
    clips_[i] = joined;
    clips_.erase(clips_.begin() + static_cast<std::ptrdiff_t>(i) + 1);
    edited_ = true;
    return EditResult::Ok;
}

int Edit::ClipAt(double t) const {
    t = std::clamp(t, in_, out_);
    return static_cast<int>(std::upper_bound(markers_.begin(), markers_.end(), t) - markers_.begin());
}

double Edit::ClipStart(int i) const { return i <= 0 ? in_ : markers_[static_cast<std::size_t>(i) - 1]; }
double Edit::ClipEnd(int i) const {
    return i >= static_cast<int>(markers_.size()) ? out_ : markers_[static_cast<std::size_t>(i)];
}

double Edit::PrevStop(double t) const {
    double best = in_;
    for (double m : markers_)
        if (m < t - 1e-6) best = m;
    return best;
}

double Edit::NextStop(double t) const {
    for (double m : markers_)
        if (m > t + 1e-6) return m;
    return out_;
}

EditResult Edit::SetCamera(int clip, CameraKind k, bool confirmed) {
    if (clip < 0 || clip >= static_cast<int>(clips_.size())) return EditResult::NotFound;
    Clip& c = clips_[static_cast<std::size_t>(clip)];
    if (c.camera == k) return EditResult::Ok;
    if (!confirmed && (!c.keys.empty() || c.camera != CameraKind::Game)) return EditResult::NeedsConfirm;
    c.camera = k;
    c.keys.clear();
    c.base = CameraParams{};
    edited_ = true;
    return EditResult::Ok;
}

EditResult Edit::AddKey(double t, const CameraParams& p) {
    Clip& c = clips_[static_cast<std::size_t>(ClipAt(t))];
    if (!IsAdvanced(c.camera)) return EditResult::NotFound;
    for (Key& k : c.keys)
        if (std::fabs(k.time - t) < 0.05) {
            k.p = p;
            edited_ = true;
            return EditResult::Ok;
        }
    const auto at = std::upper_bound(c.keys.begin(), c.keys.end(), t, [](double v, const Key& k) { return v < k.time; });
    c.keys.insert(at, Key{t, p});
    edited_ = true;
    return EditResult::Ok;
}

EditResult Edit::DeleteKey(double t, double within) {
    Clip& c = clips_[static_cast<std::size_t>(ClipAt(t))];
    auto best = c.keys.end();
    double bestD = within;
    for (auto it = c.keys.begin(); it != c.keys.end(); ++it) {
        const double d = std::fabs(it->time - t);
        if (d <= bestD) best = it, bestD = d;
    }
    if (best == c.keys.end()) return EditResult::NotFound;
    c.keys.erase(best);
    edited_ = true;
    return EditResult::Ok;
}

double Edit::PrevKey(double t) const {
    const Clip& c = clips_[static_cast<std::size_t>(ClipAt(t))];
    double best = t;
    for (const Key& k : c.keys)
        if (k.time < t - 1e-6) best = k.time;
    return best;
}

double Edit::NextKey(double t) const {
    const Clip& c = clips_[static_cast<std::size_t>(ClipAt(t))];
    for (const Key& k : c.keys)
        if (k.time > t + 1e-6) return k.time;
    return t;
}

Edit::Frame Edit::Evaluate(double t) const {
    Frame f;
    f.clip = ClipAt(t);
    const Clip& c = clips_[static_cast<std::size_t>(f.clip)];
    f.camera = c.camera;
    f.speed = kSpeeds[std::clamp(c.speed, 0, static_cast<int>(std::size(kSpeeds)) - 1)];
    f.shake = c.shake, f.filter = c.filter, f.lens = c.lens;
    f.p = c.base;
    if (IsAdvanced(c.camera) && !c.keys.empty()) {
        if (t <= c.keys.front().time) {
            f.p = c.keys.front().p;
        } else if (t >= c.keys.back().time) {
            f.p = c.keys.back().p;
        } else {
            for (std::size_t i = 0; i + 1 < c.keys.size(); ++i) {
                if (t >= c.keys[i].time && t <= c.keys[i + 1].time) {
                    const float s = static_cast<float>((t - c.keys[i].time) / std::max(c.keys[i + 1].time - c.keys[i].time, 1e-6));
                    f.p = Spline(c.keys[i > 0 ? i - 1 : i].p, c.keys[i].p, c.keys[i + 1].p,
                                 c.keys[i + 2 < c.keys.size() ? i + 2 : i + 1].p, s);
                    break;
                }
            }
        }
    }
    return f;
}

double Edit::EditedLength() const {
    double len = 0.0;
    for (int i = 0; i < static_cast<int>(clips_.size()); ++i) {
        const int s = std::clamp(clips_[static_cast<std::size_t>(i)].speed, 0, static_cast<int>(std::size(kSpeeds)) - 1);
        len += (ClipEnd(i) - ClipStart(i)) / kSpeeds[s];
    }
    return len;
}

}  // namespace replay
