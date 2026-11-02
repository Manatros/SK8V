#pragma once
// The Skate Shop's opening hours (design 2026-11-02-skate-shop-consistency):
// INI SkateShopHours "open-close" in game hours; 0-24 is always open and a
// range may cross midnight (20-4).
#include <cstdio>
#include <string>

namespace shophours {

struct Hours {
    int open = 8, close = 22;
    bool always() const { return close - open >= 24 || open == close; }
};

// "8-22" -> {8, 22}; false (and always open) on anything else.
inline bool Parse(const std::string& text, Hours& out) {
    int a = -1, b = -1;
    char tail = 0;
    if (std::sscanf(text.c_str(), " %d - %d %c", &a, &b, &tail) != 2 || a < 0 || a > 24 || b < 0 || b > 24) {
        out = Hours{0, 24};
        return false;
    }
    out = Hours{a, b};
    return true;
}

inline bool OpenAt(int hour, const Hours& h) {
    if (h.always()) return true;
    if (h.open < h.close) return hour >= h.open && hour < h.close;
    return hour >= h.open || hour < h.close;  // across midnight
}

// "08:00-22:00", for the closed sign.
inline std::string Text(const Hours& h) {
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%02d:00-%02d:00", h.open % 24, h.close % 24);
    return buf;
}

} // namespace shophours
