#pragma once
// Hidden collectibles (design 2026-11-01-collectibles): the places
// (GTAxSkate\collectibles.tsv, from scripts/collectibles/place.py), the
// player's progress (GTAxSkate\collectibles.txt) and the Self Radio songs
// they unlock. No GTA calls here, so it can be unit-tested.
#include <map>
#include <set>
#include <string>
#include <vector>

namespace collectibles {

// A board (collectibles v3, design 2026-11-03-collectibles-v3) gives a deck,
// griptape, trucks and wheels at once; it counts as a part everywhere else.
// A line waypoint (rev 4): rows "L01-1", "L01-2", ... of one trick line,
// ridden through in order; the whole line earns a board.
enum class Kind { Part, Song, Board, Line };
inline bool GivesParts(Kind k) { return k == Kind::Part || k == Kind::Board; }

struct Item {
    std::string id;
    Kind kind = Kind::Part;
    float x = 0, y = 0, z = 0;  // the marker's centre, 0.9 m above a rail
    std::string place;
    bool air = false;  // counts only while the board is in the air (column 7 "air")
};

// Malformed lines and comments are skipped.
std::vector<Item> ParsePlaces(const std::string& text);

enum class State { Hidden, Found, Collected };

struct Progress {
    std::map<std::string, State> state;
    std::map<std::string, std::string> reward;  // what a collected one gave
};

Progress ParseProgress(const std::string& text);
std::string FormatProgress(const Progress& p);
State StateOf(const Progress& p, const std::string& id);

// Whether the segment a->b passes within `radius` of `c` (fast frames).
bool SegmentHits(const float (&a)[3], const float (&b)[3], const float (&c)[3], float radius);

// The marker's size at `distance` metres: 1 m up close, 4 m at 300 m.
float MarkerScale(float distance);

// Self Radio songs: "Skate 3 Track NN.mp3", locked as "...mp3.locked".
inline constexpr int kStarterSongs = 5;
// The track number of a song file name (either form), 0 if it is none.
int TrackOf(const std::string& name);
// Songs earned so far: rewards "song:NN".
std::vector<int> EarnedSongs(const Progress& p);
// The next song to unlock: the lowest track above the starters that is not
// earned yet, among `tracks` (those on disk); 0 when none is left.
int NextSong(const Progress& p, const std::vector<int>& tracks);

struct SongFile {
    std::string name;
    unsigned long long size = 0;
};

struct FileOp {
    enum Type { Rename, Delete } type = Rename;
    std::string from, to;  // names within the songs folder
};
// What makes `files` (the songs folder) match: starters and earned songs
// playable, the others locked; everything playable when `enabled` is false.
// A song in both forms (a tool rerun, or one cut short) keeps the larger
// copy: the other is deleted first.
std::vector<FileOp> SongPlan(const std::vector<SongFile>& files, const Progress& p, bool enabled);

// A collectible's fixed item (design 2026-11-02-collectibles-v2): parts in
// id order get the shop's secrets in unlock order (`secretList`: lines
// "key\ttitle\tmodels", models comma-separated), songs in id order get the
// tracks after the starters. A part beyond the list gets no key.
struct Assigned {
    std::string key, title;
    std::vector<std::string> keys;  // every key it gives
    std::vector<std::string> boardModels;  // a deck's complete board (collectibles v3)
    std::vector<std::string> models;
    int track = 0;
};
std::map<std::string, Assigned> Assign(const std::vector<Item>& items, const std::string& secretList);
// The shop keys earned so far: rewards "part:<key>" and "board:<k1>,<k2>,...".
std::vector<std::string> EarnedKeys(const Progress& p);
// A finite reward assigned to this id (line waypoints share their board id).
// Missing music files suppress that song only; assignments never shift.
bool HasReward(const Item& item, const std::map<std::string, Assigned>& assigned, const std::set<int>& tracks);
// Commit a real unlock once; old cash records remain readable, never created.
bool RecordReward(Progress& progress, const std::string& id, const std::string& reward);


// Trick lines (collectibles v3 rev 4). The line of a waypoint id ("L01-2" ->
// "L01") and its step (2); 0 when the id has no step.
std::string LineGroup(const std::string& id);
int LineStep(const std::string& id);
// One board item per trick line (id = the line, at its first waypoint), so the
// lines get their boards from Assign like any other board.
std::vector<Item> LineBoards(const std::vector<Item>& items);

// High-score milestones (design 2026-11-05-expanded-secret-and-score-unlocks):
// 10 Hall of Meat tiers ("R01".."R10", category 1) and 10 banked trick-line
// tiers ("R11".."R20", category 2), sorting after "D151" and "L06" so they
// take secrets after all spatial and park-line collectibles without shifting
// any existing assignment.
struct ScoreMilestone {
    const char* id;
    unsigned category;   // 1 Hall of Meat, 2 banked trick line
    unsigned threshold;  // points required
    const char* place;
};
const std::vector<ScoreMilestone>& ScoreMilestoneTable();
std::vector<Item> ScoreMilestones();
// Uncollected milestone items in `milestones` reached by `score` in `category`.
std::vector<Item> ScoreEarned(const std::vector<Item>& milestones, const Progress& progress,
                              unsigned category, unsigned score);

// A line being ridden: which line, the next step (1-based), when the leg began.
struct LineRun {
    std::string line;
    int next = 0;
    unsigned legStart = 0;
};
inline constexpr unsigned kLegMs = 8000;  // each leg within this
enum class LineEvent { None, Started, Next, Done, Failed };
// One frame: `hitLine`/`hitStep` the waypoint passed now ("" / 0 for none),
// `steps` that line's step count, `bailed` a wipeout or off the board.
LineEvent LineUpdate(LineRun& run, const std::string& hitLine, int hitStep, int steps, unsigned now, bool bailed);

// Whether Skate's state (SvOutput::state_utf8) is the board in the air: not
// grinding, walking or wiping out (an air collectible counts only then).
bool BoardInAir(const std::string& state);

// Skate's centre popups (design 2026-11-02-collectible-popups); the movie
// upper-cases and keeps one line. A part's kind is known only once earned.
std::string PopupFound(Kind kind);
// `reward`: "part:deck:...", "part:grip:...", "song:NN", "cash:N";
// `name`: the shop's label of the part.
std::string PopupEarned(const std::string& reward, const std::string& name);

// Safe developer collectible travel (design 2026-11-05-safe-collectible-travel).
inline constexpr int kTravelCandidateCount = 8;
inline constexpr unsigned kTravelTimeoutMs = 5000;
inline constexpr float kTravelSceneRadius = 25.0f;
inline constexpr float kTravelMoveLimit = 0.30f;
inline constexpr float kTravelCapsuleRadius = 0.40f;
inline constexpr float kTravelCapsuleHeight = 1.90f;
inline constexpr float kTravelCapsuleBaseClearance = 0.05f;

struct TravelGuard {
    bool ready = false;
    bool poseFadingOut = false;
    bool fadingCam = false;
    bool lifecycleBlocked = false;
    bool replayActive = false;
    bool dropperOpen = false;
    bool shopOpen = false;
    bool frontendActive = false;
    bool onFoot = false;
    bool inVehicle = false;
    bool ragdoll = false;
    bool falling = false;
    bool inAir = false;
    int ped = 0;
    float pos[3] = {0, 0, 0};
};

struct TravelBounds {
    bool valid = false;
    float min[3] = {0, 0, 0};
    float max[3] = {0, 0, 0};
};

struct TravelLosSample {
    int status = 0;  // 0 invalid, 1 pending, 2 complete
    bool hit = false;
    bool worldEntity = false;
    float z = 0.0f;
    float normalZ = 0.0f;
};

struct TravelCapsuleResult {
    int status = 0;  // 0 invalid, 1 pending, 2 complete
    bool hit = false;
};

struct TravelAdapter {
    bool (*selectedItem)(std::string& id, std::string& label, float (&xyz)[3], void* ctx) = nullptr;
    bool (*isLoadSceneActive)(void* ctx) = nullptr;
    bool (*isLoadSceneLoaded)(void* ctx) = nullptr;
    void (*requestCollision)(float x, float y, float z, void* ctx) = nullptr;
    bool (*startLoadSceneSphere)(float x, float y, float z, float radius, void* ctx) = nullptr;
    void (*stopLoadScene)(void* ctx) = nullptr;
    TravelBounds (*getPedBounds)(int ped, void* ctx) = nullptr;
    bool (*getSafeCoord)(float x, float y, float z, float (&out)[3], void* ctx) = nullptr;
    TravelLosSample (*probeFloor)(float x, float y, float zTop, float zBottom, int ignorePed, void* ctx) = nullptr;
    // GTA's capsule shape test is asynchronous: start returns a handle (0 =
    // not started), poll returns 1 while pending and 2 once complete.
    int (*startCapsule)(float x, float y, float z1, float z2, float radius, int ignorePed, void* ctx) = nullptr;
    TravelCapsuleResult (*pollCapsule)(int handle, void* ctx) = nullptr;
    void (*teleportPed)(int ped, float x, float y, float z, void* ctx) = nullptr;
    void (*setWaypoint)(float x, float y, void* ctx) = nullptr;
    void* ctx = nullptr;
};

// The standing volume the capsule test must find clear: never smaller than
// the accepted ped model (review 2026-11-06: bounds up to 0.45 m / 2.10 m
// were accepted while only 0.40 m / 1.90 m was tested).
struct TravelClearance {
    float radius = 0.40f;
    float height = 1.90f;
};

struct TravelSession {
    bool active = false;
    bool ownsScene = false;
    int startPed = 0;
    std::string targetId;
    std::string targetLabel;
    float targetPos[3] = {0, 0, 0};
    float startPos[3] = {0, 0, 0};
    float footOffset = 1.0f;
    TravelClearance clearance{};
    unsigned deadlineMs = 0;
    // Candidate scan state: one capsule test in flight at a time, polled on
    // later frames (review 2026-11-06: same-frame results were pending and
    // their handles were discarded, so every arrival fell back to GPS).
    int nextCandidate = 0;
    int capsuleHandle = 0;
    float pendingNav[3] = {0, 0, 0};
    float pendingSupportZ = 0.0f;
    // A capsule still pending when travel ended: read once more until GTA
    // completes it, so no shape-test result is left unread.
    int drainHandle = 0;
};

void TravelCandidateOffset(int index, float (&dxDy)[2]);
bool TravelCheckBounds(const TravelBounds& b, float& outFootOffset);
bool TravelCheckBounds(const TravelBounds& b, float& outFootOffset, TravelClearance& outClearance);
bool TravelCheckSafeCoord(const float (&marker)[3], const float (&req)[2], const float (&nav)[3]);
bool TravelCheckFloorSupport(float navZ, const TravelLosSample (&samples)[5], float& outSupportZ);
void TravelCapsuleSpheres(float supportZ, float& outZ1, float& outZ2);
void TravelCapsuleSpheres(float supportZ, const TravelClearance& c, float& outZ1, float& outZ2);
// Polls a capsule left pending by an ended session; true while still pending.
bool TravelDrain(TravelSession& session, const TravelAdapter& api);
const char* TravelGuardReason(const TravelGuard& g);
std::string TravelRequest(TravelSession& session, const TravelGuard& guard, unsigned nowMs, const TravelAdapter& api);
std::string TravelTick(TravelSession& session, const TravelGuard& guard, unsigned nowMs, const TravelAdapter& api);
void TravelCancel(TravelSession& session, const TravelAdapter& api);

}  // namespace collectibles
