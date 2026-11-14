#include "tutorial_teleport.h"
#include <cassert>
#include <chrono>
#include <iostream>
#include <limits>

int main() {
    using namespace tutorial;
    assert(IsInTutorialZone(-4500, -3200));
    assert(IsInTutorialZone(-5450, -3200));
    assert(IsInTutorialZone(-4100, -2800));
    assert(IsInTutorialZone(-5850, -3600));
    assert(!IsInTutorialZone(-4099, -3200));
    assert(!IsInTutorialZone(-4975, -3200));
    assert(!IsInTutorialZone(-4500, -2799));
    assert(!IsInTutorialZone(kVespucciParkReturn[0], kVespucciParkReturn[1]));
    assert(!IsInTutorialZone(std::numeric_limits<float>::quiet_NaN(), -3200));
    assert(!IsInTutorialZone(-4500, std::numeric_limits<float>::infinity()));
    assert(kSkateSchoolSpawn[2] >= 20.0f && kStartParkSpawn[2] >= 20.0f); // NO_UNDERWATER
    const auto dir = std::filesystem::temp_directory_path() / ("tutorial-teleport-" +
        std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
    std::filesystem::create_directories(dir);
    assert(!HasTutorialPack(dir));
    const auto root = dir / "addons" / "skate_school";
    std::filesystem::create_directories(root / "collision");
    std::filesystem::create_directories(root / "splines");
    assert(!HasTutorialPack(dir));
    for (const auto* name : {"DIST_SkateSchool", "DIST_StartPark"}) {
        std::ofstream(root / "collision" / (std::string(name) + ".svwc")) << "fixture";
        std::ofstream(root / "splines" / (std::string(name) + "_rails.json")) << "[]";
    }
    assert(HasTutorialPack(dir));
    std::filesystem::rename(dir / "addons", dir / "external-addons");
    std::filesystem::create_directories(dir / "cache");
    std::filesystem::rename(dir / "external-addons", dir / "cache" / "addons");
    assert(!HasTutorialPack(dir));
    std::ofstream(dir / "SkateVLegacy.ini") << "[Other]\nWorldCache=wrong\n[SkateV]\nWorldCache=cache/world.svwc\n";
    assert(HasTutorialPack(dir));
    // The flat layout is accepted by both host and runtime too.
    for (const auto* name : {"DIST_SkateSchool", "DIST_StartPark"}) {
        for (const auto* ext : {".svwc", "_rails.json"}) {
            const std::string file = std::string(name) + ext;
            const auto folder = std::string(ext) == ".svwc" ? "collision" : "splines";
            std::filesystem::rename(dir / "cache/addons/skate_school" / folder / file,
                                    dir / "cache/addons/skate_school" / file);
        }
    }
    assert(HasTutorialPack(dir));
    // Verify Windows UTF-16LE INI written by WritePrivateProfileStringW also resolves WorldCache.
    {
        std::ofstream u16(dir / "SkateVLegacy.ini", std::ios::binary);
        const std::string text = "[SkateV]\r\nWorldCache=cache/world.svwc\r\n";
        const unsigned char bom[2]{0xFF, 0xFE};
        u16.write(reinterpret_cast<const char*>(bom), 2);
        for (char ch : text) {
            const char pair[2]{ch, 0};
            u16.write(pair, 2);
        }
    }
    assert(HasTutorialPack(dir));
    std::filesystem::remove(dir / "cache/addons/skate_school/DIST_StartPark.svwc");
    assert(!HasTutorialPack(dir));
    std::filesystem::remove_all(dir);
    std::cout << "tutorial teleport tests passed\n";
}
