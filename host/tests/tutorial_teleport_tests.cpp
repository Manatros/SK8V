#include "tutorial_teleport.h"
#include "tutorial_missions.h"
#include <cassert>
#include <chrono>
#include <iostream>
#include <limits>

namespace {
unsigned lessonTestGroups = 0;
std::uint32_t fakeState = 3, fakeMask = 1;
std::uint32_t __cdecl FakeCommand(void*, std::uint32_t cmd, std::uint32_t) {
    if (cmd == SV_TUTORIAL_RESTART) fakeMask = 0;
    return 1;
}
std::uint32_t __cdecl FakeStep(void*, const SvTutorialInput* in, SvTutorialStatus* out) {
    out->state = fakeState; out->completed_mask = fakeMask; out->preset = in->preset;
    const char* banners[]{"Coach Frank: Nailed it! Ollie Basics - LESSON PASSED!", "Objective complete: Ollie Basics", "Completed Ollie Basics."};
    strcpy_s(out->banner_utf8, banners[in->preset]); return 1;
}
void TestLessons(const std::filesystem::path& dir) {
    using namespace tutorial;
    assert(DetectPreset(true,true)==UiPreset::Sk8v); ++lessonTestGroups;
    assert(DetectPreset(true,false)==UiPreset::GtaxSkate); ++lessonTestGroups;
    assert(DetectPreset(false,false)==UiPreset::Rp && DetectPreset(false,true)==UiPreset::Rp); ++lessonTestGroups;
    assert(UsesCentral(UiPreset::Sk8v) && !UsesCentral(UiPreset::GtaxSkate) && !UsesCentral(UiPreset::Rp)); ++lessonTestGroups;
    for (unsigned state=0;state<=4;++state) assert(StartPauseCommand(state)==(state>=1 && state<=3 ? SV_TUTORIAL_STOP : SV_TUTORIAL_START));
    ++lessonTestGroups;
    assert(ProgressText(0)=="0/17 lessons completed" && ProgressText(kAllLessons)=="17/17 lessons completed");
    assert(ProgressText(0xFFFFFFFF)=="17/17 lessons completed"); ++lessonTestGroups;
    const auto path=dir / "progress.txt";
    assert(LoadProgress(path)==0); assert(SaveProgress(path,7)); assert(LoadProgress(path)==7);
    assert(SaveProgress(path,3)); assert(LoadProgress(path)==3); assert(!SaveProgress(path,1u<<20));
    assert(LoadProgress(path)==3); ++lessonTestGroups;
    for (const char* invalid : {"garbage", "SkateV-Tutorial-1 4294967295", "SkateV-Tutorial-1 2 extra", "SkateV-Tutorial-2 1", "SkateV-Tutorial-1"}) {
        std::ofstream(path) << invalid; assert(LoadProgress(path)==0);
    }
    ++lessonTestGroups;
    lessonRuntime=reinterpret_cast<void*>(1); lessonCommand=FakeCommand; lessonStep=FakeStep; progressPath=path; savedMask=0;
    for(unsigned preset=0;preset<3;++preset) {
        SvTutorialInput in{}; in.preset=preset;
        unsigned centers=0, notices=0; std::string message; shownBanner.clear();
        auto center=[&](const char* t) { ++centers; message=t; };
        auto notice=[&](const char* t) { ++notices; message=t; };
        TickLessons(in,false,center,notice); assert(centers+notices==0);
        TickLessons(in,true,center,notice); assert(centers==(preset==0 ? 1u:0u) && notices==(preset==0 ? 0u:1u));
        assert(message==(preset==0 ? "Coach Frank: Nailed it! Ollie Basics - LESSON PASSED!" : preset==1 ? "Objective complete: Ollie Basics" : "Completed Ollie Basics."));
        TickLessons(in,true,center,notice); assert(centers+notices==1);
        if (preset != 0) assert(IsLessonNotice(message));
    }
    assert(LoadProgress(path)==1); ++lessonTestGroups;
    assert(CommandLessons(SV_TUTORIAL_RESTART));
    TickLessons(SvTutorialInput{},false,[](const char*){},[](const char*){}); assert(LoadProgress(path)==0);
    lessonStep=nullptr; assert(!CommandLessons(SV_TUTORIAL_START));
    TickLessons(SvTutorialInput{},true,[](const char*){assert(false);},[](const char*){assert(false);});
    ++lessonTestGroups;
}
}

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
    TestLessons(dir);
    std::filesystem::remove_all(dir);
    std::cout << "11 test groups passed (1 teleport, " << lessonTestGroups << " tutorial lessons)\n";
}
