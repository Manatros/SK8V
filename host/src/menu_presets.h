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

// Whether a stored snapshot section is complete (design 2026-11-16-replay-lens-and-prompts #7).
// `stored` counts every non-"__" key in the section, `storedKnown` those that are menu keys now and
// `count` is the section's own __count. Keys that are not menu keys (a removed row, a key another
// tool wrote into the section) no longer make it invalid: it is truncated only when fewer keys are
// stored than __count promised, and it has grown past it only when more known keys are stored.
// `reason` names the failed check; the caller logs it.
inline bool SnapshotComplete(std::size_t present, std::size_t stored, std::size_t storedKnown, long long count, std::string& reason) {
    if (!present) return reason = "no known setting is stored in it", false;
    if (count < 0) return reason = "__count is missing", false;
    if (stored < static_cast<std::size_t>(count))
        return reason = "truncated: " + std::to_string(stored) + " keys stored, __count says " + std::to_string(count), false;
    if (storedKnown > static_cast<std::size_t>(count))
        return reason = "more menu keys stored (" + std::to_string(storedKnown) + ") than __count says (" + std::to_string(count) + ")", false;
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
