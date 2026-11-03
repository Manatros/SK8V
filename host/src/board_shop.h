#pragma once
// The board shop (contributed by GTAxSkate): in Binco, the Discount Store and
// Suburban, G opens Skate 3's boards and parts (the runtime's board_shop.rs
// keeps the catalog and each character's garage). Buying takes the
// character's cash; the bought board is ridden at once (board_parts).
#include <string>
#include "skatev_runtime.h"

class RuntimeBridge;

namespace boardshop {

// After the runtime loaded: the catalog from `dataRoot`, garages in `gameDir`.
// `wheels`: called with the ridden wheels' hardness from their paint (0..1),
// or -1 for unpainted wheels, whenever the ridden board changes.
bool Start(RuntimeBridge& runtime, const std::string& dataRoot, const std::string& gameDir, void (*log)(const char*),
           void (*wheels)(float hardness));

// Shop availability and weapon presentation are separate: Skate's on-foot
// state may browse the shop, but SK8V still owns its animated held board.
// `carryBoard` is only true for the optional GTA weapon mode while Ready.
void Tick(bool busy, bool carryBoard);

// A frame of skating (Skate's state, speed in m/s): the ridden board wears;
// a part worn through shows Skate 3's worn finish of it.
void Riding(const char* state, float speed);

// A crew member's board (AI skater `slot`, 0..3, in `pose`): a complete
// board from the shop's stock, the same for that slot each time, from the
// HD part models. False when it cannot show (the caller shows its own);
// ClearCrewBoard removes it.
bool CrewBoard(unsigned slot, const SvBoardPose& pose);
void ClearCrewBoard(unsigned slot);

// Collectibles (design 2026-11-01-collectibles): the secret decks and
// griptapes `count` part collectibles unlock (right after Start), the next
// unlock ("key\ttitle", empty when none is left), and cash for the player.
int SetSecrets(int count);  // still locked; -1 without the shop
std::string UnlockNext();
// Collectibles v2 (design 2026-11-02-collectibles-v2): the secret list
// ("key\ttitle\tmodels" lines), unlock by key (1 now, 2 already, 0 no
// secret, -1 no runtime support), relock all (-1 without support), close an
// open menu, and the rest pose of a board standing on its tail at `xyz`
// turned `yawDegrees` (a collectible showing its deck).
std::string SecretList();
int Unlock(const std::string& key);
// A board collectible (collectibles v3): the complete board for deck `key` into
// the garage; its title, "" when none was given.
std::string GiveBoard(const std::string& key);
int RelockAll();
void CloseMenu();
bool PickupPose(const float (&xyz)[3], float yawDegrees, SvBoardPose& pose);
void Pay(int dollars);

// The store's menu is open (other menus and Skate's input stay out).
bool IsOpen();
// The Vespucci board shop works (board shop on, runtime with sv_shop_open_at).
bool BeachShopAvailable();

} // namespace boardshop
