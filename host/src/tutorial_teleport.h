#pragma once
#include <array>
#include <cmath>
#include <cctype>
#include <filesystem>
#include <fstream>
#include <string>

namespace tutorial {
inline constexpr std::array<float, 3> kSkateSchoolSpawn{-4500.0f, -3200.0f, 95.5f};
inline constexpr std::array<float, 3> kStartParkSpawn{-5450.0f, -3200.0f, 22.5f};
inline constexpr std::array<float, 3> kVespucciParkReturn{-1355.0f, -1385.0f, 4.5f};
static_assert(kSkateSchoolSpawn[2] >= 20.0f && kStartParkSpawn[2] >= 20.0f, "NO_UNDERWATER");

inline bool IsInTutorialZone(float x, float y) {
    return std::isfinite(x) && std::isfinite(y) && std::abs(y + 3200.0f) <= 400.0f &&
        (std::abs(x + 4500.0f) <= 400.0f || std::abs(x + 5450.0f) <= 400.0f);
}

inline bool HasTutorialPack(const std::filesystem::path& gameDir) {
    const auto installed = [](const std::filesystem::path& cacheRoot) {
        const auto root = cacheRoot / "addons" / "skate_school";
        const auto exists = [](const std::filesystem::path& p) {
            std::error_code error;
            return std::filesystem::is_regular_file(p, error);
        };
        for (const auto* name : {"DIST_SkateSchool", "DIST_StartPark"}) {
            const std::string collision = std::string(name) + ".svwc";
            const std::string rails = std::string(name) + "_rails.json";
            if (!(exists(root / "collision" / collision) || exists(root / collision)) ||
                !(exists(root / "splines" / rails) || exists(root / rails))) return false;
        }
        return true;
    };
    // WorldCache can be outside the game directory; handle both UTF-8/ANSI and UTF-16LE INIs.
    std::ifstream ini(gameDir / "SkateVLegacy.ini", std::ios::binary);
    std::string raw((std::istreambuf_iterator<char>(ini)), std::istreambuf_iterator<char>());
    if (raw.size() >= 2 && static_cast<unsigned char>(raw[0]) == 0xFF && static_cast<unsigned char>(raw[1]) == 0xFE) {
        std::string ascii;
        ascii.reserve(raw.size() / 2);
        for (size_t i = 2; i + 1 < raw.size(); i += 2) ascii.push_back(raw[i + 1] == 0 ? raw[i] : '?');
        raw = std::move(ascii);
    }
    const auto trim = [](std::string s) {
        const auto first = s.find_first_not_of(" \t\r\n\"");
        return first == std::string::npos ? std::string{} : s.substr(first, s.find_last_not_of(" \t\r\n\"") - first + 1);
    };
    bool skateSection = false;
    std::size_t pos = 0;
    while (pos < raw.size()) {
        const std::size_t nl = raw.find('\n', pos);
        std::string line = trim(raw.substr(pos, nl == std::string::npos ? raw.size() - pos : nl - pos));
        pos = nl == std::string::npos ? raw.size() : nl + 1;
        if (line.empty() || line[0] == ';' || line[0] == '#') continue;
        const auto eq = line.find('=');
        auto key = trim(line.substr(0, eq));
        for (auto& c : key) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
        if (line[0] == '[') { skateSection = key == "[skatev]"; continue; }
        if (skateSection && key == "worldcache" && eq != std::string::npos) {
            auto cache = std::filesystem::u8path(trim(line.substr(eq + 1)));
            if (cache.empty()) continue;
            if (cache.is_relative()) cache = gameDir / cache;
            return installed(cache.parent_path());
        }
    }
    return installed(gameDir);
}
} // namespace tutorial
