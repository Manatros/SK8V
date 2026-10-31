// cl /std:c++20 /EHsc /W4 board_parts_tests.cpp
#include "../src/board_parts_logic.h"
#include <cassert>

struct Object { std::string name; };

int main() {
    using namespace boardpartslogic;
    Selection selection;
    assert(selection.Update({"deck", "wheels-a"}, 0));
    assert(!selection.Settled(399));
    assert(selection.Settled(400));
    assert(!selection.Update({"deck", "wheels-a"}, 401));
    assert(selection.Settled(401));
    assert(selection.Update({"deck", "wheels-b"}, 450));
    assert(!selection.Settled(849));
    assert(selection.Update({"deck", "wheels-a"}, 849));
    assert(!selection.Settled(1248));
    assert(selection.Settled(1249));
    selection = {};
    assert(selection.Update({"deck"}, 2000));
    assert(!selection.Settled(2399));
    assert(selection.CanPreview(2000, true));
    assert(!selection.CanPreview(2399, false));
    assert(selection.CanPreview(2400, false));

    assert(PrefetchRadius(160, 240) == 3);
    assert(PrefetchRadius(161, 240) == 1);
    assert(PrefetchRadius(220, 240) == 1);
    assert(PrefetchRadius(221, 240) == 0);
    assert(PrefetchRadius(240, 240) == 0);
    // The ridden board's reserve comes off the preloading budget.
    assert(PrefetchRadius(120, 240, 40) == 3);
    assert(PrefetchRadius(121, 240, 40) == 1);
    assert(PrefetchRadius(181, 240, 40) == 0);
    const std::vector<std::vector<std::string>> neighbours{
        {"next", "shared"}, {"previous", "shared"}, {"next2"}, {"previous2"}, {"next3"}, {"previous3"}};
    std::vector<std::string> requested;
    int registered = 0;
    auto request = [&](const auto& name) { requested.push_back(name); ++registered; };
    auto count = [&] { return registered; };
    auto wanted = PrefetchModels(neighbours, 240, count, request);
    assert((requested == std::vector<std::string>{"next", "shared", "previous", "next2", "previous2", "next3", "previous3"}));
    assert(wanted.size() == 7);
    requested.clear();
    registered = 201;
    wanted = PrefetchModels(neighbours, 240, count, request);
    assert((requested == std::vector<std::string>{"next", "shared", "previous"}));
    assert(wanted.size() == 3);
    // Recheck the budget during requests, leaving registrations for selection.
    requested.clear();
    registered = 220;
    wanted = PrefetchModels(neighbours, 240, count, request);
    assert((requested == std::vector<std::string>{"next"}));
    assert(registered == 221 && wanted.size() == 1);
    requested.clear();
    assert(PrefetchModels(neighbours, 240, count, request).empty());
    assert(requested.empty());
    registered = 0;
    assert(PrefetchModels({}, 240, count, request).empty());
    // Empty rows must not pull more distant entries inside the reduced radius.
    registered = 201;
    wanted = PrefetchModels({{}, {"previous"}, {"too-far"}}, 240, count, request);
    assert(wanted == std::unordered_set<std::string>{"previous"});
    auto upward = neighbours;
    for (std::size_t i = 0; i < upward.size(); i += 2) std::swap(upward[i], upward[i + 1]);
    registered = 0;
    requested.clear();
    wanted = PrefetchModels(upward, 240, count, request);
    assert(requested.front() == "previous");
    assert(wanted.count("next3") && wanted.count("previous3"));
    // Moving away drops old neighbours while retaining shared models.
    registered = 0;
    wanted = PrefetchModels({{"new", "shared"}}, 240, count, request);
    assert(wanted.size() == 2 && wanted.count("shared") && !wanted.count("previous"));

    std::vector<Object> current{{"old-deck"}, {"old-wheels"}};
    std::vector<Object> pending;
    std::vector<std::string> removed, visible;
    bool posed = false;
    auto remove = [&](Object& object) { removed.push_back(object.name); };
    auto publish = [&](Object& object) {
        assert(posed && removed.empty()); // All new objects posed before old removal.
        assert(current.front().name == "old-deck");
        visible.push_back(object.name);
    };
    auto create = [](const std::string& name, Object& object) { object.name = name; return name != "missing"; };
    auto pose = [&](std::vector<Object>& objects) { assert(objects.size() == 2); posed = true; return true; };
    // Partial creation failure releases only the new partial board.
    assert(!Replace(current, pending, {"new-deck", "missing"}, create, pose, publish, remove));
    assert(current.size() == 2 && current.front().name == "old-deck");
    assert(removed == std::vector<std::string>{"new-deck"} && visible.empty() && !posed);
    removed.clear();
    // Skeleton readiness may take multiple frames: retain invisible candidates.
    assert(!Replace(current, pending, {"new-deck", "new-wheels"}, create,
                    [](auto&) { return false; }, publish, remove));
    assert(current.front().name == "old-deck" && visible.empty());
    assert(removed.empty() && pending.size() == 2);
    // Complete replacement publishes the whole new board, then retires the old.
    assert(Replace(current, pending, {"new-deck", "new-wheels"},
                   [](const auto&, auto&) { assert(false); return false; }, pose, publish, remove));
    assert(pending.empty());
    assert(posed && current.front().name == "new-deck");
    assert((visible == std::vector<std::string>{"new-deck", "new-wheels"}));
    assert((removed == std::vector<std::string>{"old-deck", "old-wheels"}));
}
