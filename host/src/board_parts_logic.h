#pragma once
#include <utility>
#include <cstdint>
#include <string>
#include <vector>
#include <unordered_set>

namespace boardpartslogic {
// `reserve` registrations stay for the ridden board; preloading shrinks early
// (a browsed entry's neighbours cost three times its own registrations).
inline int PrefetchRadius(int registered, int limit, int reserve = 0) {
    const int remaining = limit - reserve - registered;
    return remaining < 20 ? 0 : remaining < 80 ? 1 : 3;
}

// Entries are paired by distance, with the scroll direction first. Empty
// entries preserve distances at list boundaries and on group rows.
template<class Registered, class Request>
std::unordered_set<std::string> PrefetchModels(
    const std::vector<std::vector<std::string>>& neighbours, int limit,
    Registered registered, Request request, int reserve = 0) {
    std::unordered_set<std::string> wanted;
    for (std::size_t i = 0; i < neighbours.size() && i < 6; ++i) {
        if (static_cast<int>(i / 2) >= PrefetchRadius(registered(), limit, reserve)) break;
        for (const auto& name : neighbours[i]) {
            if (!PrefetchRadius(registered(), limit, reserve)) break;
            if (!name.empty() && wanted.insert(name).second) request(name);
        }
    }
    return wanted;
}

// Candidates remain invisible across frames until their skeletons can be posed.
// Creation failure releases partial objects; the current board stays intact.
template<class Item, class Create, class Pose, class Publish, class Remove>
bool Replace(std::vector<Item>& current, std::vector<Item>& pending,
             const std::vector<std::string>& models,
             Create create, Pose pose, Publish publish, Remove remove) {
    if (pending.empty()) {
        for (const auto& name : models) {
            Item item;
            if (!create(name, item)) {
                for (auto& partial : pending) remove(partial);
                pending.clear();
                return false;
            }
            pending.push_back(std::move(item));
        }
    }
    if (!pose(pending)) return false;
    for (auto& item : pending) publish(item);
    for (auto& item : current) remove(item);
    current = std::move(pending);
    pending.clear();
    return true;
}

struct Selection {
    std::vector<std::string> models;
    std::uint64_t changed = 0;

    bool Update(const std::vector<std::string>& next, std::uint64_t now) {
        if (next == models) return false;
        models = next;
        changed = now;
        return true;
    }
    bool Settled(std::uint64_t now) const { return now - changed >= 400; }
    bool CanPreview(std::uint64_t now, bool loaded) const { return loaded || Settled(now); }
};
} // namespace boardpartslogic
