#pragma once
// Hidden collectibles (design 2026-11-01-collectibles): the places
// (GTAxSkate\collectibles.tsv, from scripts/collectibles/place.py), the
// player's progress (GTAxSkate\collectibles.txt) and the Self Radio songs
// they unlock. No GTA calls here, so it can be unit-tested.
#include <map>
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

// Trick lines (collectibles v3 rev 4). The line of a waypoint id ("L01-2" ->
// "L01") and its step (2); 0 when the id has no step.
std::string LineGroup(const std::string& id);
int LineStep(const std::string& id);
// One board item per trick line (id = the line, at its first waypoint), so the
// lines get their boards from Assign like any other board.
std::vector<Item> LineBoards(const std::vector<Item>& items);

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

}  // namespace collectibles
