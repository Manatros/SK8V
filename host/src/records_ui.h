#pragma once
// Player records over GTA: a call-out when a Hall
// of Meat bail or a banked line places in the top three (the book keeps ten),
// the record under the Hall of Meat score box during a bail, and a top-ten
// board from the settings menu (no zones). Drawn on the script thread with GTA's
// own text and shard (MIDSIZED_MESSAGE); the book and Skate 3's labels come
// from the runtime (rust/skatev-runtime/src/records.rs).
//
#include <cstdint>
#include <string>

namespace recordsui {
using LogFn = void (*)(const char*);

struct ScoreSnapshot {
    bool ready = false;
    std::uint32_t sequence = 0;
    std::uint32_t category = 0;  // 1 Hall of Meat bail, 2 banked line
    std::uint32_t score = 0;
    std::uint32_t homBest = 0;
    std::uint32_t lineBest = 0;
};

// After the runtime loaded: looks up the optional record exports and opens
// the book at %LOCALAPPDATA%\SkateV\records.json (runtime nullptr: inert).
void Start(void* runtime, LogFn log);
// Opens or closes the top-ten board (menu item; '/' closes it while shown).
void NextPage();
bool Showing();
// Latest record state from the runtime (for score-milestone unlocks).
ScoreSnapshot PollScores();
// Script thread, once per frame. `skating`: Skate is active; `homBail`: a
// Hall of Meat bail is on screen. Hidden UI still polls record state, consumes
// announcements and closes the board, so restoring visibility cannot replay it.
void Tick(bool skating, bool homBail, bool visible = true);
}
