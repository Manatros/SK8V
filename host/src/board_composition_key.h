#pragma once
#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

namespace boardcomposition {
inline std::string Payload(const std::vector<std::string>& models) {
    if (models.empty() || models.size() > 8) return {};
    std::string out;
    for (const auto& model : models) {
        if (model.size() > 140 || model.rfind("skatev_", 0) != 0) return {};
        const auto end = model.find('_', 7);
        if (end == std::string::npos) return {};
        const auto kind = model.substr(7, end - 7);
        if (kind != "d" && kind != "t" && kind != "tb" && kind != "th" && kind != "w" && kind != "g") return {};
        if (end + 1 == model.size() || model.size() - end - 1 > 128) return {};
        for (const auto& prior : models) { if (&prior == &model) break; if (prior == model) return {}; }
        for (char c : model) if (!(c >= 'a' && c <= 'z') && !(c >= '0' && c <= '9') && c != '_') return {};
        out += model + '\n';
    }
    return out;
}
inline std::string Key(const std::string& payload) {
    std::uint64_t hash = 14695981039346656037ull;
    for (unsigned char c : payload) hash = (hash ^ c) * 1099511628211ull;
    char out[17];
    std::snprintf(out, sizeof(out), "%016llx", static_cast<unsigned long long>(hash));
    return out;
}
}
