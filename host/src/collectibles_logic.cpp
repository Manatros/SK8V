#include "collectibles_logic.h"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <set>
#include <sstream>

namespace collectibles {
namespace {

std::vector<std::string> Split(const std::string& line, char sep) {
    std::vector<std::string> out;
    size_t start = 0;
    for (size_t i = 0; i <= line.size(); ++i) {
        if (i == line.size() || line[i] == sep) {
            out.push_back(line.substr(start, i - start));
            start = i + 1;
        }
    }
    return out;
}

bool Number(const std::string& text, float& out) {
    if (text.empty()) return false;
    char* end = nullptr;
    const float v = std::strtof(text.c_str(), &end);
    if (end != text.c_str() + text.size() || !std::isfinite(v)) return false;
    out = v;
    return true;
}

std::string Trim(std::string s) {
    while (!s.empty() && (s.back() == '\r' || s.back() == ' ')) s.pop_back();
    return s;
}

constexpr const char* kPrefix = "Skate 3 Track ";
constexpr const char* kLocked = ".locked";

std::string SongName(int track) {
    char name[64];
    std::snprintf(name, sizeof(name), "%s%02d.mp3", kPrefix, track);
    return name;
}

}  // namespace

std::vector<Item> ParsePlaces(const std::string& text) {
    std::vector<Item> out;
    std::istringstream in(text);
    std::string line;
    while (std::getline(in, line)) {
        line = Trim(line);
        if (line.empty() || line[0] == '#') continue;
        const auto f = Split(line, '\t');
        if (f.size() < 6 || f[0] == "id") continue;
        Item it;
        it.id = f[0];
        if (f[1] == "part") it.kind = Kind::Part;
        else if (f[1] == "song") it.kind = Kind::Song;
        else continue;
        if (it.id.empty() || !Number(f[2], it.x) || !Number(f[3], it.y) || !Number(f[4], it.z)) continue;
        it.place = f[5];
        out.push_back(it);
    }
    return out;
}

Progress ParseProgress(const std::string& text) {
    Progress p;
    std::istringstream in(text);
    std::string line;
    while (std::getline(in, line)) {
        line = Trim(line);
        const auto f = Split(line, ' ');
        if (f.size() >= 2 && f[0] == "found") {
            if (StateOf(p, f[1]) == State::Hidden) p.state[f[1]] = State::Found;
        } else if (f.size() >= 2 && f[0] == "collected") {
            p.state[f[1]] = State::Collected;
            const size_t at = line.find(' ', line.find(' ') + 1);
            p.reward[f[1]] = at == std::string::npos ? std::string() : line.substr(at + 1);
        }
    }
    return p;
}

std::string FormatProgress(const Progress& p) {
    std::string out = "# GTAxSkate collectibles: found <id> / collected <id> <reward>\n";
    for (const auto& [id, s] : p.state) {
        if (s == State::Found) out += "found " + id + "\n";
        else if (s == State::Collected) {
            const auto r = p.reward.find(id);
            out += "collected " + id + (r != p.reward.end() && !r->second.empty() ? " " + r->second : std::string()) + "\n";
        }
    }
    return out;
}

State StateOf(const Progress& p, const std::string& id) {
    const auto it = p.state.find(id);
    return it == p.state.end() ? State::Hidden : it->second;
}

bool SegmentHits(const float (&a)[3], const float (&b)[3], const float (&c)[3], float radius) {
    float d[3], w[3];
    float dd = 0, wd = 0;
    for (int i = 0; i < 3; ++i) {
        d[i] = b[i] - a[i];
        w[i] = c[i] - a[i];
        dd += d[i] * d[i];
        wd += w[i] * d[i];
    }
    const float t = dd > 1e-9f ? std::clamp(wd / dd, 0.0f, 1.0f) : 0.0f;
    float dist2 = 0;
    for (int i = 0; i < 3; ++i) {
        const float e = a[i] + t * d[i] - c[i];
        dist2 += e * e;
    }
    return dist2 <= radius * radius;
}

float MarkerScale(float distance) {
    return 1.0f + 3.0f * std::clamp(distance / 300.0f, 0.0f, 1.0f);
}

int TrackOf(const std::string& name) {
    std::string n = name;
    if (n.size() > 7 && n.compare(n.size() - 7, 7, kLocked) == 0) n.resize(n.size() - 7);
    const std::string prefix = kPrefix;
    if (n.size() != prefix.size() + 6 || n.compare(0, prefix.size(), prefix) != 0 || n.compare(n.size() - 4, 4, ".mp3") != 0)
        return 0;
    const std::string digits = n.substr(prefix.size(), 2);
    if (!std::isdigit(static_cast<unsigned char>(digits[0])) || !std::isdigit(static_cast<unsigned char>(digits[1]))) return 0;
    return std::atoi(digits.c_str());
}

std::vector<int> EarnedSongs(const Progress& p) {
    std::vector<int> out;
    for (const auto& [id, r] : p.reward)
        if (r.rfind("song:", 0) == 0) {
            const int t = std::atoi(r.c_str() + 5);
            if (t > 0) out.push_back(t);
        }
    std::sort(out.begin(), out.end());
    return out;
}

int NextSong(const Progress& p, const std::vector<int>& tracks) {
    const auto earned = EarnedSongs(p);
    std::vector<int> sorted = tracks;
    std::sort(sorted.begin(), sorted.end());
    for (const int t : sorted)
        if (t > kStarterSongs && std::find(earned.begin(), earned.end(), t) == earned.end()) return t;
    return 0;
}

std::vector<FileOp> SongPlan(const std::vector<SongFile>& files, const Progress& p, bool enabled) {
    const auto earned = EarnedSongs(p);
    std::set<int> playable, locked;
    std::map<int, unsigned long long> playableSize, lockedSize;
    for (const auto& f : files) {
        const std::string& n = f.name;
        const int t = TrackOf(n);
        if (!t) continue;
        const bool isLocked = n.size() > 7 && n.compare(n.size() - 7, 7, kLocked) == 0;
        (isLocked ? locked : playable).insert(t);
        (isLocked ? lockedSize : playableSize)[t] = f.size;
    }
    std::vector<FileOp> ops;
    std::set<int> all = playable;
    all.insert(locked.begin(), locked.end());
    for (const int t : all) {
        const std::string mp3 = SongName(t), dead = mp3 + kLocked;
        if (playable.count(t) && locked.count(t)) {
            // The larger copy stays (a song cut short is never the one kept).
            if (playableSize[t] >= lockedSize[t]) {
                ops.push_back({FileOp::Delete, dead, {}});
                locked.erase(t);
            } else {
                ops.push_back({FileOp::Delete, mp3, {}});
                playable.erase(t);
            }
        }
        const bool play = !enabled || t <= kStarterSongs || std::find(earned.begin(), earned.end(), t) != earned.end();
        if (play && !playable.count(t)) ops.push_back({FileOp::Rename, dead, mp3});
        if (!play && playable.count(t)) ops.push_back({FileOp::Rename, mp3, dead});
    }
    return ops;
}

}  // namespace collectibles
