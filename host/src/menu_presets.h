#pragma once
#include "menu.h"
#include <cerrno>
#include <map>
#include <set>

namespace menu::presets {
using Settings = std::map<std::string, std::string>;

inline bool Valid(const Item& item, const std::string& value) {
    if (value.find_first_of("\r\n") != std::string::npos) return false;
    if (item.kind == Kind::Choice) return ChoiceIndex(item, value) >= 0;
    if (item.kind == Kind::Flag) return value == "0" || value == "1";
    if (item.kind == Kind::Number) {
        char* end = nullptr;
        errno = 0;
        const double n = std::strtod(value.c_str(), &end);
        const double tolerance = 1e-6 * std::max({1.0, std::abs(static_cast<double>(item.lo)),
                                                 std::abs(static_cast<double>(item.hi))});
        return !value.empty() && end == value.c_str() + value.size() && !errno &&
               std::isfinite(n) && n >= item.lo - tolerance && n <= item.hi + tolerance;
    }
    if (item.kind == Kind::Chord) {
        char* end = nullptr;
        errno = 0;
        const auto n = std::strtoul(value.c_str(), &end, 0);
        return !value.empty() && end == value.c_str() + value.size() && !errno &&
               n <= 0xFFFF && !(n & ~0xF3FFul) && (n || item.allowOff);
    }
    return false;
}

// Duplicate menu rows describe one setting, but each row's live callback still runs.
inline bool Validate(const std::vector<Page>& pages, const Settings& settings, std::string& error) {
    std::set<std::string> seen;
    for (const auto& page : pages) for (const auto& item : page.items) {
        if (item.key.empty()) continue;
        seen.insert(item.key);
        const auto found = settings.find(item.key);
        if (found == settings.end() || !Valid(item, found->second)) {
            error = "Missing or invalid preset setting: " + item.key;
            return false;
        }
    }
    if (seen.size() != settings.size()) {
        error = "Preset contains unavailable settings";
        return false;
    }
    return true;
}

inline bool Build(const std::vector<Page>& pages,
                  const std::vector<std::pair<std::string, std::string>>& defaults,
                  const std::vector<std::pair<std::string, std::string>>& overrides,
                  Settings& out, std::string& error) {
    Settings next;
    for (const auto& page : pages) for (const auto& item : page.items)
        if (!item.key.empty()) next.emplace(item.key, item.def);
    for (const auto& group : {defaults, overrides}) for (const auto& [key, value] : group) {
        if (!next.count(key)) { error = "Preset setting unavailable: " + key; return false; }
        next[key] = value;
    }
    if (!Validate(pages, next, error)) return false;
    out = std::move(next);
    return true;
}
} // namespace menu::presets
