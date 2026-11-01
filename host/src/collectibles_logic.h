#pragma once
// Hidden collectibles (design 2026-11-01-collectibles): the places
// (GTAxSkate\collectibles.tsv, from scripts/collectibles/place.py), the
// player's progress (GTAxSkate\collectibles.txt) and the Self Radio songs
// they unlock. No GTA calls here, so it can be unit-tested.
#include <map>
#include <string>
#include <vector>

namespace collectibles {

enum class Kind { Part, Song };

struct Item {
    std::string id;
    Kind kind = Kind::Part;
    float x = 0, y = 0, z = 0;  // the marker's centre, 0.9 m above a rail
    std::string place;
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

}  // namespace collectibles
