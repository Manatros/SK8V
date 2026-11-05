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
        else if (f[1] == "board") it.kind = Kind::Board;
        else if (f[1] == "line") it.kind = Kind::Line;
        else continue;
        it.air = f.size() > 6 && Trim(f[6]) == "air";
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

std::map<std::string, Assigned> Assign(const std::vector<Item>& items, const std::string& secretList) {
    std::vector<Assigned> secrets;
    std::istringstream lines(secretList);
    for (std::string line; std::getline(lines, line);) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        const size_t t1 = line.find('\t');
        if (line.empty() || t1 == std::string::npos || t1 == 0) continue;
        const size_t t2 = line.find('\t', t1 + 1);
        Assigned a;
        a.key = line.substr(0, t1);
        a.keys = {a.key};
        a.title = line.substr(t1 + 1, t2 == std::string::npos ? std::string::npos : t2 - t1 - 1);
        if (t2 != std::string::npos) {
            const size_t t3 = line.find('\t', t2 + 1);
            std::istringstream models(line.substr(t2 + 1, t3 == std::string::npos ? std::string::npos : t3 - t2 - 1));
            for (std::string m; std::getline(models, m, ',');)
                if (!m.empty()) a.models.push_back(m);
            // A deck's complete board (collectibles v3), for a board collectible.
            if (t3 != std::string::npos) {
                std::istringstream board(line.substr(t3 + 1));
                for (std::string m; std::getline(board, m, ',');)
                    if (!m.empty()) a.boardModels.push_back(m);
            }
        }
        secrets.push_back(std::move(a));
    }
    std::vector<const Item*> sorted;
    for (const Item& it : items) sorted.push_back(&it);
    std::sort(sorted.begin(), sorted.end(), [](const Item* a, const Item* b) { return a->id < b->id; });
    std::map<std::string, Assigned> out;
    // Boards first (collectibles v3, peer review): each takes the next unused
    // secret deck and gives its complete board (deck, trucks, wheels) into the
    // garage; parts then take the rest in order (griptapes, trucks, wheels,
    // decks in turn), so the scarce kinds go to parts.
    std::vector<bool> taken(secrets.size(), false);
    for (const Item* it : sorted) {
        if (it->kind != Kind::Board) continue;
        Assigned board;
        for (size_t k = 0; k < secrets.size(); ++k) {
            if (taken[k] || secrets[k].key.rfind("deck:", 0) != 0) continue;
            taken[k] = true;
            board = secrets[k];
            board.keys = {board.key};
            if (!board.boardModels.empty()) board.models = board.boardModels;
            break;
        }
        out[it->id] = board;
    }
    size_t part = 0;
    int song = 0;
    for (const Item* it : sorted) {
        if (it->kind == Kind::Part) {
            while (part < secrets.size() && taken[part]) ++part;
            out[it->id] = part < secrets.size() ? secrets[part] : Assigned{};
            ++part;
        } else if (it->kind == Kind::Song) {
            out[it->id].track = kStarterSongs + 1 + song++;
        }
    }
    return out;
}

bool HasReward(const Item& item, const std::map<std::string, Assigned>& assigned, const std::set<int>& tracks) {
    const auto a = assigned.find(item.kind == Kind::Line ? LineGroup(item.id) : item.id);
    if (a == assigned.end()) return false;
    if (item.kind == Kind::Song) return a->second.track > kStarterSongs && tracks.count(a->second.track) != 0;
    return !a->second.key.empty();
}

bool RecordReward(Progress& progress, const std::string& id, const std::string& reward) {
    if (id.empty() || StateOf(progress, id) == State::Collected) return false;
    const bool part = reward.rfind("part:", 0) == 0 && reward.size() > 5;
    const bool board = reward.rfind("board:", 0) == 0 && reward.size() > 6;
    const bool song = reward.rfind("song:", 0) == 0 && reward.size() > 5 &&
        std::all_of(reward.begin() + 5, reward.end(), [](unsigned char c) { return std::isdigit(c) != 0; }) &&
        std::strtol(reward.c_str() + 5, nullptr, 10) > kStarterSongs;
    if (!part && !board && !song) return false;
    progress.state[id] = State::Collected;
    progress.reward[id] = reward;
    return true;
}

std::vector<std::string> EarnedKeys(const Progress& p) {
    std::vector<std::string> out;
    for (const auto& [id, r] : p.reward)
        if (r.rfind("part:", 0) == 0) {
            out.push_back(r.substr(5));
        } else if (r.rfind("board:", 0) == 0) {
            std::istringstream keys(r.substr(6));
            for (std::string k; std::getline(keys, k, ',');)
                if (!k.empty()) out.push_back(k);
        }
    return out;
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

std::string PopupFound(Kind kind) {
    if (kind == Kind::Board) return "Hidden board found";
    return kind == Kind::Part ? "Hidden board part found" : "Hidden song found";
}

std::string LineGroup(const std::string& id) {
    const size_t dash = id.rfind('-');
    return dash == std::string::npos ? id : id.substr(0, dash);
}

int LineStep(const std::string& id) {
    const size_t dash = id.rfind('-');
    return dash == std::string::npos ? 0 : std::atoi(id.c_str() + dash + 1);
}

std::vector<Item> LineBoards(const std::vector<Item>& items) {
    std::map<std::string, Item> first;
    for (const Item& it : items) {
        if (it.kind != Kind::Line || LineStep(it.id) != 1) continue;
        Item board = it;
        board.id = LineGroup(it.id);
        board.kind = Kind::Board;
        board.air = false;
        first[board.id] = board;
    }
    std::vector<Item> out;
    for (auto& [id, it] : first) out.push_back(it);
    return out;
}

const std::vector<ScoreMilestone>& ScoreMilestoneTable() {
    static const std::vector<ScoreMilestone> kTable = {
        {"R01", 1, 5000, "Hall of Meat 5,000 pts"},
        {"R02", 1, 10000, "Hall of Meat 10,000 pts"},
        {"R03", 1, 15000, "Hall of Meat 15,000 pts"},
        {"R04", 1, 20000, "Hall of Meat 20,000 pts"},
        {"R05", 1, 25000, "Hall of Meat 25,000 pts"},
        {"R06", 1, 30000, "Hall of Meat 30,000 pts"},
        {"R07", 1, 40000, "Hall of Meat 40,000 pts"},
        {"R08", 1, 50000, "Hall of Meat 50,000 pts"},
        {"R09", 1, 60000, "Hall of Meat 60,000 pts"},
        {"R10", 1, 75000, "Hall of Meat 75,000 pts"},
        {"R11", 2, 500, "Trick line 500 pts"},
        {"R12", 2, 1000, "Trick line 1,000 pts"},
        {"R13", 2, 1500, "Trick line 1,500 pts"},
        {"R14", 2, 2500, "Trick line 2,500 pts"},
        {"R15", 2, 3500, "Trick line 3,500 pts"},
        {"R16", 2, 5000, "Trick line 5,000 pts"},
        {"R17", 2, 6500, "Trick line 6,500 pts"},
        {"R18", 2, 8000, "Trick line 8,000 pts"},
        {"R19", 2, 10000, "Trick line 10,000 pts"},
        {"R20", 2, 12500, "Trick line 12,500 pts"},
    };
    return kTable;
}

std::vector<Item> ScoreMilestones() {
    std::vector<Item> out;
    for (const ScoreMilestone& m : ScoreMilestoneTable()) {
        Item it;
        it.id = m.id;
        it.kind = Kind::Part;
        it.place = m.place;
        out.push_back(it);
    }
    return out;
}

std::vector<Item> ScoreEarned(const std::vector<Item>& milestones, const Progress& progress,
                              unsigned category, unsigned score) {
    std::vector<Item> out;
    if (score == 0) return out;
    const auto& table = ScoreMilestoneTable();
    for (const Item& it : milestones) {
        if (StateOf(progress, it.id) == State::Collected) continue;
        for (const ScoreMilestone& m : table) {
            if (it.id == m.id && m.category == category && score >= m.threshold) {
                out.push_back(it);
                break;
            }
        }
    }
    return out;
}

LineEvent LineUpdate(LineRun& run, const std::string& hitLine, int hitStep, int steps, unsigned now, bool bailed) {
    if (!run.line.empty()) {
        if (bailed || now - run.legStart > kLegMs) {
            run = LineRun{};
            return LineEvent::Failed;
        }
        if (hitLine == run.line && hitStep == run.next) {
            if (run.next >= steps) {
                run = LineRun{};
                return LineEvent::Done;
            }
            ++run.next;
            run.legStart = now;
            return LineEvent::Next;
        }
        return LineEvent::None;
    }
    if (!bailed && hitStep == 1 && !hitLine.empty() && steps > 1) {
        run.line = hitLine;
        run.next = 2;
        run.legStart = now;
        return LineEvent::Started;
    }
    return LineEvent::None;
}

bool BoardInAir(const std::string& state) {
    return state.find("Air") != std::string::npos && state.rfind("Wipeout", 0) != 0 && state.rfind("Biped", 0) != 0;
}

std::string PopupEarned(const std::string& reward, const std::string& name) {
    auto starts = [&](const char* prefix) { return reward.rfind(prefix, 0) == 0; };
    if (starts("part:deck:")) return "Deck unlocked: " + name;
    if (starts("part:grip:")) return "Griptape unlocked: " + name;
    if (starts("part:trucks:")) return "Trucks unlocked: " + name;
    if (starts("part:wheels:")) return "Wheels unlocked: " + name;
    if (starts("board:")) return "Board unlocked: " + name;
    if (starts("part:")) return "Unlocked: " + name;
    if (starts("song:")) {
        char text[40];
        std::snprintf(text, sizeof(text), "New song: Skate 3 Track %02d", std::atoi(reward.c_str() + 5));
        return text;
    }
    if (starts("cash:")) return "Nothing left to unlock: $" + reward.substr(5);
    return "Unlocked";
}

void TravelCandidateOffset(int index, float (&dxDy)[2]) {
    static constexpr float kDiag = 2.82842712f;  // 4.0 * sqrt(0.5)
    static constexpr float kOffsets[kTravelCandidateCount][2] = {
        {4.0f, 0.0f}, {-4.0f, 0.0f}, {0.0f, 4.0f}, {0.0f, -4.0f},
        {kDiag, kDiag}, {-kDiag, kDiag}, {kDiag, -kDiag}, {-kDiag, -kDiag},
    };
    const int i = (index < 0 || index >= kTravelCandidateCount) ? 0 : index;
    dxDy[0] = kOffsets[i][0], dxDy[1] = kOffsets[i][1];
}

bool TravelCheckBounds(const TravelBounds& b, float& outFootOffset) {
    if (!b.valid) return false;
    for (int i = 0; i < 3; ++i)
        if (!std::isfinite(b.min[i]) || !std::isfinite(b.max[i]) || b.max[i] <= b.min[i]) return false;
    const float halfX = std::max(std::abs(b.min[0]), std::abs(b.max[0]));
    const float halfY = std::max(std::abs(b.min[1]), std::abs(b.max[1]));
    const float height = b.max[2] - b.min[2];
    const float foot = -b.min[2];
    if (halfX > 0.45f || halfY > 0.45f || height > 2.10f || foot < 0.50f || foot > 1.25f) return false;
    outFootOffset = foot;
    return true;
}

bool TravelCheckSafeCoord(const float (&marker)[3], const float (&req)[2], const float (&nav)[3]) {
    for (int i = 0; i < 3; ++i)
        if (!std::isfinite(marker[i]) || !std::isfinite(nav[i])) return false;
    if (!std::isfinite(req[0]) || !std::isfinite(req[1])) return false;
    const float rx = nav[0] - req[0], ry = nav[1] - req[1];
    if (rx * rx + ry * ry > 2.0f * 2.0f) return false;
    const float mx = nav[0] - marker[0], my = nav[1] - marker[1];
    if (mx * mx + my * my < 2.5f * 2.5f) return false;
    const float dz = nav[2] - marker[2];
    return dz >= -2.5f && dz <= -0.5f;
}

bool TravelCheckFloorSupport(float navZ, const TravelLosSample (&samples)[5], float& outSupportZ) {
    if (!std::isfinite(navZ)) return false;
    float lo = 1e30f, hi = -1e30f;
    for (const TravelLosSample& s : samples) {
        if (s.status != 2 || !s.hit || !s.worldEntity) return false;
        if (!std::isfinite(s.z) || !std::isfinite(s.normalZ)) return false;
        if (s.normalZ < 0.94f || std::abs(s.z - navZ) > 0.15f) return false;
        lo = std::min(lo, s.z), hi = std::max(hi, s.z);
    }
    if (hi - lo > 0.10f) return false;
    outSupportZ = hi;
    return true;
}

void TravelCapsuleSpheres(float supportZ, float& outZ1, float& outZ2) {
    const float zBase = supportZ + kTravelCapsuleBaseClearance;
    const float zTop = zBase + kTravelCapsuleHeight;
    outZ1 = zBase + kTravelCapsuleRadius;
    outZ2 = zTop - kTravelCapsuleRadius;
}

const char* TravelGuardReason(const TravelGuard& g) {
    if (g.ped == 0) return "no player ped";
    for (int i = 0; i < 3; ++i)
        if (!std::isfinite(g.pos[i])) return "invalid player position";
    if (g.replayActive) return "replay active";
    if (g.frontendActive) return "frontend active";
    if (g.shopOpen) return "board shop open";
    if (g.dropperOpen) return "object dropper open";
    if (!g.ready || g.poseFadingOut || g.fadingCam) return "leave skate mode first";
    if (g.inVehicle) return "leave vehicle first";
    if (!g.onFoot || g.ragdoll || g.falling || g.inAir || g.lifecycleBlocked) return "stand on foot first";
    return nullptr;
}

void TravelCancel(TravelSession& session, const TravelAdapter& api) {
    if (session.ownsScene && api.stopLoadScene) api.stopLoadScene(api.ctx);
    session = TravelSession{};
}

std::string TravelRequest(TravelSession& session, const TravelGuard& guard, unsigned nowMs, const TravelAdapter& api) {
    if (session.active) TravelCancel(session, api);
    std::string id, label;
    float targetPos[3] = {0, 0, 0};
    if (!api.selectedItem || !api.selectedItem(id, label, targetPos, api.ctx)) return "Collectibles: none selected";
    for (int i = 0; i < 3; ++i)
        if (!std::isfinite(targetPos[i])) return "Collectibles: invalid coordinates";
    if (const char* reason = TravelGuardReason(guard)) {
        if (api.setWaypoint) api.setWaypoint(targetPos[0], targetPos[1], api.ctx);
        return std::string("Collectibles travel refused (") + reason + "); GPS set instead";
    }
    float footOffset = 1.0f;
    if (!api.getPedBounds || !TravelCheckBounds(api.getPedBounds(guard.ped, api.ctx), footOffset)) {
        if (api.setWaypoint) api.setWaypoint(targetPos[0], targetPos[1], api.ctx);
        return "Collectibles travel refused (unsupported ped bounds); GPS set instead";
    }
    if (api.isLoadSceneActive && api.isLoadSceneActive(api.ctx)) {
        if (api.setWaypoint) api.setWaypoint(targetPos[0], targetPos[1], api.ctx);
        return "Collectibles travel refused (load scene already active); GPS set instead";
    }
    if (api.requestCollision) api.requestCollision(targetPos[0], targetPos[1], targetPos[2], api.ctx);
    if (!api.startLoadSceneSphere || !api.startLoadSceneSphere(targetPos[0], targetPos[1], targetPos[2], kTravelSceneRadius, api.ctx)) {
        if (api.setWaypoint) api.setWaypoint(targetPos[0], targetPos[1], api.ctx);
        return "Collectibles travel refused (load scene failed to start); GPS set instead";
    }
    session.active = true;
    session.ownsScene = true;
    session.startPed = guard.ped;
    session.targetId = id;
    session.targetLabel = label;
    for (int i = 0; i < 3; ++i) {
        session.targetPos[i] = targetPos[i];
        session.startPos[i] = guard.pos[i];
    }
    session.footOffset = footOffset;
    session.deadlineMs = nowMs + kTravelTimeoutMs;
    return "";
}

std::string TravelTick(TravelSession& session, const TravelGuard& guard, unsigned nowMs, const TravelAdapter& api) {
    if (!session.active) return "";
    if (const char* reason = TravelGuardReason(guard)) {
        TravelCancel(session, api);
        return std::string("Collectibles travel cancelled (") + reason + ")";
    }
    if (guard.ped != session.startPed) {
        TravelCancel(session, api);
        return "Collectibles travel cancelled (player ped changed)";
    }
    std::string curId, curLabel;
    float curPos[3] = {0, 0, 0};
    if (!api.selectedItem || !api.selectedItem(curId, curLabel, curPos, api.ctx) || curId != session.targetId) {
        TravelCancel(session, api);
        return "Collectibles travel cancelled (selection changed)";
    }
    const float dx = guard.pos[0] - session.startPos[0];
    const float dy = guard.pos[1] - session.startPos[1];
    const float dz = guard.pos[2] - session.startPos[2];
    if (dx * dx + dy * dy + dz * dz > kTravelMoveLimit * kTravelMoveLimit) {
        TravelCancel(session, api);
        return "Collectibles travel cancelled (player moved)";
    }
    if (static_cast<int>(nowMs - session.deadlineMs) > 0) {
        const float tx = session.targetPos[0], ty = session.targetPos[1];
        TravelCancel(session, api);
        if (api.setWaypoint) api.setWaypoint(tx, ty, api.ctx);
        return "Collectibles travel timed out; GPS set instead";
    }
    if (api.requestCollision) api.requestCollision(session.targetPos[0], session.targetPos[1], session.targetPos[2], api.ctx);
    if (!api.isLoadSceneActive || !api.isLoadSceneActive(api.ctx)) {
        session.ownsScene = false;
        const float tx = session.targetPos[0], ty = session.targetPos[1];
        TravelCancel(session, api);
        if (api.setWaypoint) api.setWaypoint(tx, ty, api.ctx);
        return "Collectibles travel failed (load scene interrupted); GPS set instead";
    }
    if (!api.isLoadSceneLoaded || !api.isLoadSceneLoaded(api.ctx)) return "";

    static constexpr float kSampleOffsets[5][2] = {
        {0.0f, 0.0f}, {0.4f, 0.0f}, {-0.4f, 0.0f}, {0.0f, 0.4f}, {0.0f, -0.4f}};
    for (int i = 0; i < kTravelCandidateCount; ++i) {
        float off[2] = {0, 0};
        TravelCandidateOffset(i, off);
        const float req[2] = {session.targetPos[0] + off[0], session.targetPos[1] + off[1]};
        float nav[3] = {0, 0, 0};
        if (!api.getSafeCoord || !api.getSafeCoord(req[0], req[1], session.targetPos[2], nav, api.ctx)) continue;
        if (!TravelCheckSafeCoord(session.targetPos, req, nav)) continue;
        if (!api.probeFloor) continue;
        TravelLosSample samples[5]{};
        for (int s = 0; s < 5; ++s) {
            samples[s] = api.probeFloor(nav[0] + kSampleOffsets[s][0], nav[1] + kSampleOffsets[s][1],
                                        nav[2] + 0.30f, nav[2] - 0.30f, guard.ped, api.ctx);
        }
        float supportZ = 0.0f;
        if (!TravelCheckFloorSupport(nav[2], samples, supportZ)) continue;
        float z1 = 0.0f, z2 = 0.0f;
        TravelCapsuleSpheres(supportZ, z1, z2);
        if (!api.probeCapsule) continue;
        const TravelCapsuleResult cap = api.probeCapsule(nav[0], nav[1], z1, z2, kTravelCapsuleRadius, guard.ped, api.ctx);
        if (cap.status != 2 || cap.hit) continue;
        if (TravelGuardReason(guard) != nullptr || guard.ped != session.startPed) break;
        const float originZ = supportZ + session.footOffset;
        if (!std::isfinite(originZ)) continue;
        const std::string arrived = session.targetLabel;
        const int ped = session.startPed;
        if (api.teleportPed) api.teleportPed(ped, nav[0], nav[1], originZ, api.ctx);
        TravelCancel(session, api);
        return arrived;
    }
    const float tx = session.targetPos[0], ty = session.targetPos[1];
    const std::string id = session.targetId;
    TravelCancel(session, api);
    if (api.setWaypoint) api.setWaypoint(tx, ty, api.ctx);
    return "Collectibles travel: no safe floor verified near " + id + "; GPS set instead";
}

}  // namespace collectibles
