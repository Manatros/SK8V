// Link with the host objects except main.obj; no GTA native is invoked.
#include "../src/main.cpp"
#include <cassert>
#include <iostream>
#include <fstream>
#include <set>
#ifdef EXPECTED_TEST_WORLD_BASELINE
static_assert(kTestWorldBaseline == (EXPECTED_TEST_WORLD_BASELINE != 0));
#endif
int wmain(int argc, wchar_t** argv) {
    assert(argc == 2);
    g_gameDir = std::wstring(argv[1]) + L"\\";
    g_logDir = g_gameDir;
    const auto ini = g_gameDir + L"SkateVLegacy.ini";
    DeleteFileW(ini.c_str());
    for (const auto* key : {L"BakedWorld", L"Fallbacks", L"RailBoxes", L"PlainGtaCompare"})
        WritePrivateProfileStringW(L"SkateV", key, L"1", ini.c_str());
    assert(WorldAtBest(LoadConfig()));

    // Saved diagnostic switches must not leak into an enforced local test.
    // Normalization is pure: it preserves both personal fields and the file.
    WritePrivateProfileStringW(L"SkateV", L"StepRails", L"1", ini.c_str());
    WritePrivateProfileStringW(L"SkateV", L"UnflaggedStairs", L"1", ini.c_str());
    WritePrivateProfileStringW(L"SkateV", L"HighResParks", L"0", ini.c_str());
    WritePrivateProfileStringW(L"SkateV", L"DynamicWorld", L"0", ini.c_str());
    auto bytes = [&] {
        std::ifstream f(std::filesystem::path(ini), std::ios::binary);
        return std::string(std::istreambuf_iterator<char>(f), {});
    };
    const auto before = bytes();
    Config custom = LoadConfig();
    custom.objectPush = 0.75f;
    custom.fovScale = 1.35f;
    custom.keyboard = false;
    const auto changed = SavedWorldState(custom, true);
    assert(!WorldAtBest(custom));
    NormalizeTestWorldConfig(custom, false);
    assert(SavedWorldState(custom, true) == changed);
    g_plainGtaCompare = true;
    NormalizeTestWorldConfig(custom, true);
    assert(WorldAtBest(custom) && g_plainGtaCompare); // helper never changes global state
    assert(custom.objectPush == 0.75f && custom.fovScale == 1.35f && !custom.keyboard);
    assert(bytes() == before);
    auto startup = LoadConfig();
    NormalizeTestWorldConfig(startup, kTestWorldBaseline);
    assert(WorldAtBest(startup) == kTestWorldBaseline);
    assert(SavedWorldState(LoadConfig(), true) == changed);
    g_plainGtaCompare = false;
    DeleteFileW(ini.c_str());
    for (const auto* key : {L"BakedWorld", L"Fallbacks", L"RailBoxes", L"PlainGtaCompare"})
        WritePrivateProfileStringW(L"SkateV", key, L"1", ini.c_str());

    assert(BestWorldSettings().size() == std::size(kWorldSwitches));
    std::map<std::string, std::string> best;
    for (const auto& value : BestWorldSettings()) assert(best.emplace(value).second);
    for (const auto& w : kWorldSwitches) assert(best.at(w.key) == (w.best ? "1" : "0"));
    std::set<std::string> presetKeys;
    for (const auto& [key, value] : GtaxSkatePreset()) {
        assert(presetKeys.insert(key).second);
        if (best.count(key)) assert(best.at(key) == value);
    }
    Config noObjects;
    noObjects.dynamicWorld = false;
    assert(!WorldAtBest(noObjects));
    assert(WorldState(noObjects) == "custom: Nearby entities collide");

    Config saved{};
    assert(WorldAtBest(saved));
    assert(WorldState(saved) == "best");
    saved.stepRails = true;
    saved.highResParks = false;
    assert(!WorldAtBest(saved));
    assert(WorldState(saved) == "custom: Grind every stair step, High-res skateparks");
    assert(WorldState(saved, true) == "custom: Grind every stair step=1, High-res skateparks=0");
    std::map<unsigned, bool> options;
    auto capture = [&](unsigned id, bool on) { options[id] = on; };
    SendWorldOptions(saved, capture);
    const auto original = options;
    assert(options.size() == 11 && options.at(8) && options.at(10) && !options.at(14));
    assert(!options.count(kHostOnlyWorldSwitch));
    std::set<unsigned> ids;
    for (const auto& [id, on] : options) ids.insert(id);
    assert((ids == std::set<unsigned>{0,2,3,4,5,6,7,8,9,10,14}));
    assert(!options.count(11) && !options.count(12));
    g_plainGtaCompare = true;
    SendWorldOptions(saved, capture);
    for (const auto& [id, on] : options) assert(!on);
    assert(WorldState(saved) == "COMPARE (plain GTA)");
    g_plainGtaCompare = false;
    SendWorldOptions(saved, capture);
    assert(options == original);
    assert(saved.stepRails && !saved.highResParks);
    assert(WorldState(saved).find("custom:") == 0);
    for (const auto& setting : BestWorldSettings()) {
        assert(setting.first != "BakedWorld" && setting.first != "Fallbacks" && setting.first != "RailBoxes");
    }
    for (const auto& key : kKeys) {
        assert(std::string(key.name) != "Fallbacks" && std::string(key.name) != "RailBoxes" && std::string(key.name) != "BakedWorld");
    }
    Session session{};
    auto pages = MenuPages(session);
    auto dev = std::find_if(pages.begin(), pages.end(), [](const auto& page) { return page.title == "World"; });
    assert(dev != pages.end());
    assert(dev->items[0].kind == menu::Kind::Heading && dev->items[0].label == "World collision");
    assert(menu::Label(dev->items[1]) == "World collision: best");
    auto& compare = dev->items[2];
    assert(compare.label == "Compare: plain GTA collision" && compare.kind == menu::Kind::Flag && compare.key.empty());
    assert(dev->items[3].label.find("Mark this spot") == 0);
    for (unsigned i = 0; i < std::size(kWorldSwitches); ++i) {
        assert(dev->items[4 + i].key == kWorldSwitches[i].key);
        assert(dev->items[4 + i].help.find("best:") != std::string::npos);
    }
    assert(dev->items[4 + std::size(kWorldSwitches)].label == "Reset all settings to GTAxSkate defaults");
    // Test marks (design 2026-10-31-mark-travel): own section under World collision.
    const size_t marksAt = 5 + std::size(kWorldSwitches);
    assert(dev->items[marksAt].kind == menu::Kind::Heading && dev->items[marksAt].label == "Test marks");
    const char* markRows[] = {"Mark", "Go to this mark", "Mark fixed", "Show fixed marks", "Delete this mark"};
    for (unsigned i = 0; i < std::size(markRows); ++i) {
        assert(dev->items[marksAt + 1 + i].label == markRows[i]);
        assert(dev->items[marksAt + 1 + i].key.empty());  // never saved to the INI or a preset
    }
    unsigned nearbyRows = 0;
    for (const auto& page : pages) for (const auto& item : page.items)
        if (item.key == "DynamicWorld") { ++nearbyRows; assert(page.title == "World"); }
    assert(nearbyRows == 1);
    g_config = saved;
    compare.apply("1");
    assert(g_plainGtaCompare && g_config.stepRails && !g_config.highResParks);
    assert(menu::Label(dev->items[1]) == "World collision: COMPARE (plain GTA)");
    compare.apply("0");
    assert(!g_plainGtaCompare && g_config.stepRails && !g_config.highResParks);
    HideUnavailable(pages);
    assert(!dev->items[8].visible()); // TubeRails is technical.
    g_config.devOptions = true;
    assert(dev->items[8].visible());
    WritePrivateProfileStringW(L"SkateV", L"ObjectPush", L"0.75", ini.c_str());
    menu::Start(ini, pages, {}, nullptr);
    g_config.objectPush = 0.75f;
    dev->items[1].run();
    assert(WorldAtBest(g_config) && !g_plainGtaCompare && g_config.objectPush == 0.75f);
    g_config = saved;
    compare.apply("1");
    dev->items[1].run();
    assert(WorldAtBest(g_config) && !g_plainGtaCompare);
    std::cout << "world settings tests passed; local test baseline=" << kTestWorldBaseline << "\n";
}
