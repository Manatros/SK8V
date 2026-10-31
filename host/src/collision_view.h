#pragma once
// Developer collision view (the maintainer, 2026-10-17: "see clip boxes, where it is
// grindable, skatable"): what Skate collides with near the player, drawn in
// the world. Green floor, cyan bank, red wall, purple underside, grey curb
// bevel (no grind), orange moving bodies, yellow grind rails.
#include <windows.h>
#include <string>

namespace collisionview {

// Finds the runtime's optional exports; without them the view stays off.
void Start(HMODULE runtimeModule);
// CollisionView setting: on/off, and the radius drawn around the player.
void Set(bool on, float range);
void SetWorldState(const std::string& text);
// Once per frame on the script thread while on.
void Tick(int ped);

}  // namespace collisionview
