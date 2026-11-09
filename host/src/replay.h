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
// Which keys the replay's help names (the device in use), every frame before Tick.
// The replay draws its own HUD (Instant Replay's status, the editor's timeline,
// lists and prompts): with GTA, or unbent over the fisheye by its overlay.
void SetKeyboard(bool keyboard);

}  // namespace replay
