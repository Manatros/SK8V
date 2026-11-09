#pragma once
// Skate 3's replay editor in GTA, phase 1 (design 2026-11-02-replay-editor):
// the last seconds of skating (skater, board, camera) recorded once per
// Skate tick; Back opens a replay with the world frozen, five cameras,
// speeds and scrubbing; leaving continues play exactly where it stopped.
// The recording and playhead logic is replay_logic (unit-tested).
#include <cstdint>
#include <string>
#include "replay_logic.h"
#include "skatev_runtime.h"

namespace replay {

// INI: Replay, ReplaySeconds (10-60), ReplayWorldTimeScale (0-1).
void Configure(bool enabled, double seconds, float worldTimeScale, void (*log)(const char*));
bool Enabled();
// INI: ReplaySound, ReplaySoundGainDb, ReplayFisheye, FilmFilterStrength
// (design 2026-11-09-replay-look-sound).
void ConfigureLook(bool sound, float soundGainDb, bool fisheye, float filterStrength);

// Once per new Skate tick while skating and not in a replay: the output,
// where the ped was placed (3 floats, null: the skater root), the published bone matrices (`count` x 16 floats, GTA world) and the
// board pose (null when the board is not drawn).
void Record(const SvOutput& o, const float* placed, const float* bones, int count, const SvBoardPose* board);
// A new session: the old recording goes.
void Clear();
// Enough recorded to replay (one second).
bool Ready();

// Starts the replay: time scale, own camera, controls and HUD off, Skate's
// sound paused. False when not ready.
bool Enter(int ped);
bool Active();
// One script frame (`dt` real seconds): input from the controller and the
// keyboard and mouse (replay_logic Input), playhead, presentation.
// False when the player asked to leave (B, Back, Tab or Backspace).
bool Tick(int ped, const PadIn& pad, const KeysIn& keys, float dt);
// Ends a running replay (idempotent): time scale, camera (the caller makes
// its own camera render again), controls, Skate's sound. The caller puts the
// ped back where play stopped.
void Exit();
// Status and help line ("1/2x  Follow  12.3 / 30.0 s  ..."), naming the
// keyboard's keys or the controller's buttons.
std::string Status(bool keyboard);
// Where the caller draws the status line (screen 0-1).
float StatusY();
// The replay HUD (status line, timeline): the caller draws the status line
// only when HudOnGta(); under the fisheye the overlay draws both unbent
// (DrawHudOverlay). H / the right stick's click hides it.
bool HudShown();
bool HudOnGta();
void DrawHudOverlay(const std::string& status);

}  // namespace replay
