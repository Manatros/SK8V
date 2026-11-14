#pragma once
#include "skatev_runtime.h"
#include <algorithm>
#include <bit>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <string>
#include <string_view>
#include <Windows.h>

namespace tutorial {
inline constexpr std::uint32_t kAllLessons = (1u << 17) - 1;
enum class UiPreset : std::uint32_t { Sk8v, GtaxSkate, Rp };
// These are the actual presentation switches used by the three preset definitions.
inline UiPreset DetectPreset(bool skateHud, bool hud) {
    return !skateHud ? UiPreset::Rp : hud ? UiPreset::Sk8v : UiPreset::GtaxSkate;
}
inline bool UsesCentral(UiPreset preset) { return preset == UiPreset::Sk8v; }
inline bool LessonsRunning(std::uint32_t state) { return state >= 1 && state <= 3; }
inline std::uint32_t StartPauseCommand(std::uint32_t state) {
    return LessonsRunning(state) ? SV_TUTORIAL_STOP : SV_TUTORIAL_START;
}
inline std::string ProgressText(std::uint32_t mask) {
    return std::to_string(std::popcount(mask & kAllLessons)) + "/17 lessons completed";
}
inline std::uint32_t LoadProgress(const std::filesystem::path& path) {
    std::ifstream in(path);
    std::string version, extra;
    std::uint32_t mask = 0;
    if (!(in >> version >> mask) || version != "SkateV-Tutorial-1" || (mask & ~kAllLessons) || (in >> extra)) return 0;
    return mask;
}
inline bool SaveProgress(const std::filesystem::path& path, std::uint32_t mask) {
    if (mask & ~kAllLessons) return false;
    auto temp = path; temp += L".tmp";
    { std::ofstream out(temp, std::ios::trunc); out << "SkateV-Tutorial-1 " << mask << '\n'; out.close(); if (!out) return false; }
    // Replace in one operation: an interrupted save keeps the previous valid file.
    if (MoveFileExW(temp.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) return true;
    std::error_code ec; std::filesystem::remove(temp, ec); return false;
}
inline SvTutorialCommandFn lessonCommand = nullptr;
inline SvTutorialStepFn lessonStep = nullptr;
inline void* lessonRuntime = nullptr;
inline SvTutorialStatus lessonStatus{sizeof(SvTutorialStatus)};
inline std::filesystem::path progressPath;
inline std::uint32_t savedMask = 0;
inline std::string shownBanner;
inline std::string lessonNotice;
inline bool IsLessonNotice(const std::string& text) { return !text.empty() && text == lessonNotice; }
inline bool resetPending = false;
inline void StartLessons(void* runtime, const std::filesystem::path& path) {
    lessonRuntime = runtime;
    const HMODULE module = GetModuleHandleW(L"SkateVRuntime.dll");
    lessonCommand = module ? reinterpret_cast<SvTutorialCommandFn>(GetProcAddress(module, "sv_tutorial_command")) : nullptr;
    lessonStep = module ? reinterpret_cast<SvTutorialStepFn>(GetProcAddress(module, "sv_tutorial_step")) : nullptr;
    progressPath = path;
    savedMask = LoadProgress(path);
    lessonStatus = {sizeof(SvTutorialStatus)};
    shownBanner.clear(); resetPending = false;
    if (lessonRuntime && lessonCommand && lessonStep) lessonCommand(lessonRuntime, SV_TUTORIAL_RESTORE, savedMask);
}
inline bool CommandLessons(std::uint32_t cmd) {
    if (!lessonRuntime || !lessonCommand || !lessonStep) return false;
    if (!lessonCommand(lessonRuntime, cmd, 0)) return false;
    if (cmd == SV_TUTORIAL_RESTART) resetPending = true;
    shownBanner.clear(); return true;
}
template<class Central, class Notice>
inline void TickLessons(SvTutorialInput input, bool display, Central central, Notice notice) {
    if (!lessonRuntime || !lessonStep || !lessonCommand) return;
    input.size = sizeof(input);
    SvTutorialStatus status{sizeof(SvTutorialStatus)};
    if (!lessonStep(lessonRuntime, &input, &status)) return;
    lessonStatus = status;
    if (status.completed_mask != savedMask || resetPending) {
        if (SaveProgress(progressPath, status.completed_mask)) { savedMask = status.completed_mask; resetPending = false; }
    }
    const std::string banner(status.banner_utf8, std::find(std::begin(status.banner_utf8), std::end(status.banner_utf8), '\0'));
    if (status.state == 0) { shownBanner.clear(); return; }
    if (!display || banner.empty() || banner == shownBanner) return;
    if (UsesCentral(static_cast<UiPreset>(status.preset))) central(banner.c_str());
    else { lessonNotice = banner; notice(banner.c_str()); }
    shownBanner = banner;
}
} // namespace tutorial
