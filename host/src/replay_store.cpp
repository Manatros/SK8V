#include "replay_store.h"

#include <algorithm>
#include <cmath>
#include <cstring>

namespace replay {
namespace {

constexpr char kMagic[4] = {'S', 'V', 'R', 'P'};
constexpr std::uint32_t kVersion = 1;

struct Writer {
    std::vector<std::uint8_t>& out;
    template <class T>
    void Pod(T v) {
        std::uint8_t b[sizeof(T)];
        std::memcpy(b, &v, sizeof(T));  // x64: little-endian
        out.insert(out.end(), b, b + sizeof(T));
    }
    void Floats(const float* f, std::size_t n) {
        for (std::size_t i = 0; i < n; ++i) Pod(f[i]);
    }
    void Str(const std::string& s) {
        Pod(static_cast<std::uint16_t>(s.size()));
        out.insert(out.end(), s.begin(), s.end());
    }
};

struct Reader {
    const std::vector<std::uint8_t>& in;
    std::size_t at = 0;
    bool ok = true;
    template <class T>
    T Pod() {
        T v{};
        if (at + sizeof(T) > in.size()) {
            ok = false;
            return v;
        }
        std::memcpy(&v, in.data() + at, sizeof(T));
        at += sizeof(T);
        return v;
    }
    void Floats(float* f, std::size_t n) {
        for (std::size_t i = 0; i < n; ++i) f[i] = Pod<float>();
    }
    std::string Str() {
        const auto n = Pod<std::uint16_t>();
        if (!ok || at + n > in.size()) {
            ok = false;
            return {};
        }
        std::string s(in.begin() + static_cast<std::ptrdiff_t>(at), in.begin() + static_cast<std::ptrdiff_t>(at + n));
        at += n;
        return s;
    }
    // A count that cannot be larger than what is left (`each` bytes per item).
    std::uint32_t Count(std::size_t each) {
        const auto n = Pod<std::uint32_t>();
        if (!ok || static_cast<std::uint64_t>(n) * each > in.size() - at) {
            ok = false;
            return 0;
        }
        return n;
    }
};

void WriteParams(Writer& w, const CameraParams& p) {
    w.Pod(p.yaw), w.Pod(p.pitch), w.Pod(p.distance), w.Pod(p.height), w.Pod(p.target), w.Pod(p.zoomMm);
    w.Floats(p.position, 3);
    w.Pod(static_cast<std::uint8_t>(p.positioned));
}

CameraParams ReadParams(Reader& r) {
    CameraParams p;
    p.yaw = r.Pod<float>(), p.pitch = r.Pod<float>(), p.distance = r.Pod<float>(), p.height = r.Pod<float>();
    p.target = r.Pod<float>(), p.zoomMm = r.Pod<float>();
    r.Floats(p.position, 3);
    p.positioned = r.Pod<std::uint8_t>() != 0;
    return p;
}

// Filters and the lens by name, so a later list order cannot change them.
std::string FilterKey(int filter) {
    const char* m = FilterModifier(filter);
    return m ? m : "";
}
int FilterFromKey(const std::string& key) {
    for (int i = 1; i < kFilterCount; ++i)
        if (key == FilterModifier(i)) return i;
    return 0;
}

}  // namespace

std::vector<std::uint8_t> Serialize(const SavedReplay& r) {
    std::vector<std::uint8_t> out;
    Writer w{out};
    out.insert(out.end(), kMagic, kMagic + 4);
    w.Pod(kVersion);
    w.Pod(r.modelHash);
    const std::uint32_t bones = r.frames.empty() ? 0 : static_cast<std::uint32_t>(r.frames.front().bones.size() / 16);
    w.Pod(bones);
    w.Pod(static_cast<std::uint32_t>(r.frames.size()));
    w.Pod(r.savedUnix);
    // The edited length, for the list without reading everything.
    Edit e;
    e.Restore(0.0, r.frames.empty() ? 0.0 : r.frames.back().time, r.in, r.out, r.markers, r.clips);
    w.Pod(e.EditedLength());
    for (const Frame& f : r.frames) {
        w.Pod(f.time);
        w.Pod(static_cast<std::uint8_t>(f.cut));
        w.Floats(f.root, 3);
        w.Pod(f.heading);
        w.Floats(f.velocity, 3);
        w.Floats(f.camera, 7);
        w.Pod(f.gameMs);
        w.Pod(static_cast<std::uint8_t>(f.board));
        w.Floats(f.boardEntity, 16);
        w.Floats(&f.boardWorld[0][0], 7 * 16);
        w.Floats(f.bones.data(), static_cast<std::size_t>(bones) * 16);
    }
    w.Pod(r.in), w.Pod(r.out);
    w.Pod(static_cast<std::uint32_t>(r.markers.size()));
    for (double m : r.markers) w.Pod(m);
    w.Pod(static_cast<std::uint32_t>(r.clips.size()));
    for (const Clip& c : r.clips) {
        w.Pod(static_cast<std::uint8_t>(c.camera));
        WriteParams(w, c.base);
        w.Pod(static_cast<std::uint32_t>(c.keys.size()));
        for (const Key& k : c.keys) w.Pod(k.time), WriteParams(w, k.p);
        w.Pod(static_cast<std::int32_t>(c.speed)), w.Pod(static_cast<std::int32_t>(c.shake));
        w.Str(FilterKey(c.filter));
        w.Str(c.lens == kLensFisheye ? "Fisheye" : "");
    }
    w.Pod(r.soundStartMs);
    w.Pod(static_cast<std::uint32_t>(r.pcm.size()));
    for (std::int16_t s : r.pcm) w.Pod(s);
    return out;
}

bool ReadInfo(const std::vector<std::uint8_t>& head, SavedInfo& info) {
    Reader r{head};
    if (head.size() < 4 || std::memcmp(head.data(), kMagic, 4) != 0) return false;
    r.at = 4;
    if (r.Pod<std::uint32_t>() != kVersion) return false;
    r.Pod<std::uint32_t>();  // model
    r.Pod<std::uint32_t>();  // bones
    info.frames = r.Pod<std::uint32_t>();
    info.savedUnix = r.Pod<std::uint64_t>();
    info.length = r.Pod<double>();
    return r.ok;
}

bool Deserialize(const std::vector<std::uint8_t>& bytes, SavedReplay& out, std::string& error) {
    Reader r{bytes};
    if (bytes.size() < 4 || std::memcmp(bytes.data(), kMagic, 4) != 0) return error = "not a saved replay", false;
    r.at = 4;
    if (r.Pod<std::uint32_t>() != kVersion) return error = "saved by another version", false;
    SavedReplay s;
    s.modelHash = r.Pod<std::uint32_t>();
    const auto bones = r.Pod<std::uint32_t>();
    if (bones == 0 || bones > 512) return error = "no skeleton in it", false;
    const std::size_t frameBytes = 8 + 1 + 12 + 4 + 12 + 28 + 4 + 1 + 64 + 448 + static_cast<std::size_t>(bones) * 64;
    const auto count = r.Count(frameBytes);
    s.savedUnix = r.Pod<std::uint64_t>();
    r.Pod<double>();  // edited length (the list's)
    if (!r.ok || count < 2) return error = "too short", false;
    s.frames.resize(count);
    for (Frame& f : s.frames) {
        f.time = r.Pod<double>();
        f.cut = r.Pod<std::uint8_t>() != 0;
        r.Floats(f.root, 3);
        f.heading = r.Pod<float>();
        r.Floats(f.velocity, 3);
        r.Floats(f.camera, 7);
        f.gameMs = r.Pod<std::uint32_t>();
        f.board = r.Pod<std::uint8_t>() != 0;
        r.Floats(f.boardEntity, 16);
        r.Floats(&f.boardWorld[0][0], 7 * 16);
        f.bones.resize(static_cast<std::size_t>(bones) * 16);
        r.Floats(f.bones.data(), f.bones.size());
    }
    // Numbers from a file: finite, times rising, at most two minutes.
    const auto finite = [](const float* f, std::size_t n) {
        for (std::size_t i = 0; i < n; ++i)
            if (!std::isfinite(f[i])) return false;
        return true;
    };
    for (std::size_t i = 0; i < s.frames.size(); ++i) {
        const Frame& f = s.frames[i];
        if (!std::isfinite(f.time) || (i && f.time < s.frames[i - 1].time) || f.time > 120.0 || !finite(f.root, 3) ||
            !finite(f.camera, 7) || !finite(f.velocity, 3) || !std::isfinite(f.heading) || !finite(f.bones.data(), f.bones.size()) ||
            !finite(f.boardEntity, 16) || !finite(&f.boardWorld[0][0], 7 * 16))
            return error = "the file is damaged", false;
    }
    s.in = r.Pod<double>(), s.out = r.Pod<double>();
    const auto markers = r.Count(8);
    for (std::uint32_t i = 0; i < markers; ++i) s.markers.push_back(r.Pod<double>());
    const auto clips = r.Count(1);
    if (!r.ok || clips != markers + 1) return error = "the edit is damaged", false;
    for (std::uint32_t i = 0; i < clips; ++i) {
        Clip c;
        const auto kind = r.Pod<std::uint8_t>();
        c.camera = kind < static_cast<std::uint8_t>(CameraKind::Count) ? static_cast<CameraKind>(kind) : CameraKind::Game;
        c.base = ReadParams(r);
        const auto keys = r.Count(8);
        for (std::uint32_t k = 0; k < keys; ++k) {
            Key key;
            key.time = r.Pod<double>();
            key.p = ReadParams(r);
            c.keys.push_back(key);
        }
        c.speed = std::clamp(static_cast<int>(r.Pod<std::int32_t>()), 0, 3);
        c.shake = std::clamp(static_cast<int>(r.Pod<std::int32_t>()), 0, 3);
        c.filter = FilterFromKey(r.Str());
        c.lens = r.Str() == "Fisheye" ? kLensFisheye : kLensAuto;
        s.clips.push_back(std::move(c));
    }
    s.soundStartMs = r.Pod<std::uint32_t>();
    const auto samples = r.Count(2);
    s.pcm.resize(samples);
    for (auto& v : s.pcm) v = r.Pod<std::int16_t>();
    if (!r.ok) return error = "the file is damaged", false;
    out = std::move(s);
    return true;
}

}  // namespace replay
