#pragma once
// Skate 3's object dropper in GTA V (contributed by GTAxSkate, B36): GTA's
// own skate park props, stunt ramps, benches, tables and crates, dropped
// where the player wants them, in GTA on foot or while skating. Dropped
// props are ordinary frozen GTA objects, so DynamicWorld hands them to Skate
// as exact collision (the world cache has templates for every one offered).
#include <cstddef>
#include <vector>
#include "host_util.h"
#include "skatev_runtime.h"

namespace objectdropper {

// Opened from the menu: opens at the next Tick ahead of the camera.
void RequestOpen();
// Removes every dropped object (the menu's "remove all").
void RequestClear();
// Closes it at the next Tick (the DropperKey hotkey pressed again).
void RequestClose();
bool IsOpen();

// Once per frame on the script thread. `allowed`: the player may drop now
// (on foot in GTA or in skate mode, no menu over it). `controller`: `pad` is
// the controller's frame; otherwise the keyboard is read here.
void Tick(int ped, bool allowed, bool controller, const SvPad& pad, util::LogFn log);

// The ghost object (no collision; DynamicWorld must skip it), 0 without one.
int Ghost();
// Objects dropped now, and their GTA objects.
std::size_t Count();
std::vector<int> Objects();
// A message for the player, once; nullptr when there is none.
const char* TakeNotice();

}  // namespace objectdropper
