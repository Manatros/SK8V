#pragma once

namespace interactionmenu {
// GTA input mode is independent of Skate's remembered input route.
constexpr bool Allowed(bool keyboard, bool menuOpen, bool dropperOpen, bool backBlocked) {
    return !menuOpen && !dropperOpen && (keyboard || !backBlocked);
}
constexpr bool SuppressBack(bool keyboard, bool backBlocked) {
    return !keyboard && backBlocked;
}
} // namespace interactionmenu
