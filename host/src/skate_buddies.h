#pragma once
// Skate buddies (contributed by GTAxSkate): the menu calls skater friends
// (Skate's own skater peds, a_m_y_skater_01/02, a_f_y_skater_01) who join the
// player's GTA group and follow them, so a skate crew needs no trainer.
// Followers in the player's group are what the AI skater feature takes along.

namespace skatebuddies {

// Menu: one more friend (at most kMax), or send them all home.
void Summon();
void Dismiss();

// Once per script frame: loads the model and places a called friend.
void Tick(void (*log)(const char*));

} // namespace skatebuddies
