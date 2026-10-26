#pragma once
// The board shop's parts (contributed by GTAxSkate): decks, griptapes, trucks
// and wheels built by tools/prepare-board-parts.py with the native board's
// seven-bone skeleton. The chosen ones are registered from loose files when
// first needed and posed like the native board, which they then replace.
#include <string>
#include <vector>
#include "skatev_runtime.h"

namespace boardparts {

// Call on the script thread. Reads <dataRoot>\private\board-parts.
bool Start(const std::wstring& dataRoot, void (*log)(const char*));

// The models to show (file stems without extension, e.g. skatev_d_default).
// Empty: the native board shows.
void SetLoadout(const std::vector<std::string>& models);
const std::vector<std::string>& Loadout();
bool Available(const std::string& model);
// Glowing wheels: wheel parts show their emissive `<name>_glow` copies (built
// by tools/make-glow-wheels.ps1) while on; boards swap like any loadout change.
// colour: "" = the wheels' own texture (<name>_glow), else a fixed colour's
// tinted copy <name>_glow_<colour> (lower case, as GlowColours) when built.
void SetGlowWheels(bool on, const std::string& colour = {});

// True while a custom board is shown; the caller then hides the native board.
// Loadout changes retain and pose the old objects until a replacement is ready.
// Not skating: removes the objects.
bool Tick(bool skating, const SvBoardPose& pose);

// The store's preview: `models` posed by `pose` (a rest pose from the
// runtime). Unloaded selections wait for 400 ms; loaded ones show at once.
// Empty `models` removes it.
bool Preview(const std::vector<std::string>& models, const SvBoardPose& pose);

// Neighbours paired by distance (1..3), scroll direction first in each pair.
// Requests only, never objects. Empty clears the store's prefetch requests.
void Prefetch(const std::vector<std::vector<std::string>>& neighbours);

// The store's boards on show: board `index` (0..7) as `models` at rest in
// `pose`. ClearDisplay removes them all.
bool Display(int index, const std::vector<std::string>& models, const SvBoardPose& pose);
void ClearDisplay();

// The board in hand (GTA mode, the board picked in the weapon wheel) as
// `models` in `pose`; empty `models` removes it.
bool Hand(const std::vector<std::string>& models, const SvBoardPose& pose);

// A crew member's board (AI skater `slot`, 0..3) as `models` in `pose`;
// empty `models` removes it.
bool Crew(int slot, const std::vector<std::string>& models, const SvBoardPose& pose);

// Every part object shown (riding and preview): presentation only, never collision.
std::vector<int> Entities();

void Stop();

} // namespace boardparts
