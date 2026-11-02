#pragma once
// Hidden collectibles (contributed by GTAxSkate, design 2026-11-01-collectibles):
// question marks on grind rails all over the map. Seen from afar while
// hunting, found up close (then on the map), earned by skating through
// them: each unlocks a deck or griptape in the skate shop or a Skate 3 song
// on Self Radio. Logic and files: collectibles_logic.
#include <string>
#include "host_util.h"

namespace collectibles {

// After the game folder is known: reads GTAxSkate\collectibles.tsv and
// collectibles.txt, and matches the songs folder (`enabled`: INI Collectibles).
void Start(const std::wstring& gameDir, bool enabled, util::LogFn log);
// After the board shop started: its secret items, as many as there are
// part collectibles, with those already earned unlocked.
void SyncShop();

// Once per script frame. `skating`: Skate rides (not walking, not bailed);
// `popups`: Skate's HUD shows now (skating, HUD on, not filming), so the
// moments use Skate's centre popups instead of GTA notices;
// `skater`: the skater's root; `ped`: the player ped (ignored by the sight line).
void Tick(bool enabled, bool skating, bool popups, const float (&skater)[3], int ped);

// A message for the player, once; nullptr when there is none.
const char* TakeNotice();

// "Collectibles: 12/96 (parts 5/55, songs 7/41)".
std::string Summary();
// GPS to the nearest found, not collected one; returns a notice.
const char* RouteToNearestFound();

// Developer travel: the selected collectible, stepping and its place.
std::string SelectedLabel();
void Step(int direction);
bool SelectedPlace(float (&xyz)[3]);

// Removes blips and the items shown (script shutdown, Collectibles off).
void Clear();

// Developer: every collectible hidden again and the shop's parts locked again
// (design 2026-11-02-collectibles-v2); returns the notice.
std::string Reset();

}  // namespace collectibles
