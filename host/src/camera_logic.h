#pragma once
#include <algorithm>

namespace camera {
// Keep stored indices: 4 (FirstPerson) and 5 (Head) both use GTA first person.
inline int CanonicalView(int view) {
    view = std::clamp(view, 0, 5);
    return view == 4 ? 5 : view;
}
inline int FootMode(int view) {
    constexpr int modes[] = {-1, 0, 1, 2, 4, 4};
    return modes[CanonicalView(view)];
}
inline int ViewFromFootMode(int mode) {
    return mode == 4 ? 5 : std::clamp(mode, 0, 2) + 1;
}
// GTA's own V cycle wrapped from its last view back to near (follow mode 0).
inline bool Wrapped(int before, int after) {
    return after == FootMode(1) && before != FootMode(1) && before >= 0;
}
inline int NextView(int view) {
    return CanonicalView((CanonicalView(view) + 1) % 6);
}
inline int StartView(int configured, bool remember, int last, int footMode) {
    if (!remember || last < 0) return CanonicalView(configured);
    // Remember the camera kind; GTA owns the distance and orientation on foot.
    return last == 0 ? 0 : ViewFromFootMode(footMode);
}
// -1 means leave GTA's mode untouched, including a remembered GTA entry.
inline int EntryFootMode(int configured, bool remember, int last) {
    return remember && last >= 0 ? -1 : FootMode(configured);
}
inline bool GameplayView(int view, bool overrideGta, int action) {
    return (overrideGta || FootMode(view) >= 0) && !action;
}
inline const char* Name(int view) {
    constexpr const char* names[] = {"Original SK8V / Skate 3", "GTA near", "GTA middle", "GTA far",
                                   "GTA first person", "GTA first person"};
    return names[CanonicalView(view)];
}
} // namespace camera
