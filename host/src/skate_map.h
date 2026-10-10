#pragma once
// The map while skating (contributed by GTAxSkate): GTA's GPS route to the
// waypoint stays on in Skate mode, as it does in a vehicle, and GTA's skate
// spots (from its own map data) are blips on the map.

namespace skatemap {

// Once per script frame. `skating`: Skate mode is on. `gps`/`spots`: the
// INI switches (GpsWhileSkating, SkateSpots).
void Tick(bool skating, bool gps, bool spots);

// Removes what this module put on the map (script shutdown).
void Clear();

} // namespace skatemap
