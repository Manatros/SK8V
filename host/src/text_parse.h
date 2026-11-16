#pragma once

#include <cmath>
#include <cstdlib>
#include <string>
#include <vector>

namespace text_parse {

inline std::vector<std::string> Split(const std::string& line, char sep) {
    std::vector<std::string> out;
    size_t start = 0;
    for (size_t i = 0; i <= line.size(); ++i) {
        if (i == line.size() || line[i] == sep) {
            out.push_back(line.substr(start, i - start));
            start = i + 1;
        }
    }
    return out;
}

inline bool Number(const std::string& text, float& out) {
    if (text.empty()) return false;
    char* end = nullptr;
    const float v = std::strtof(text.c_str(), &end);
    if (end != text.c_str() + text.size() || !std::isfinite(v)) return false;
    out = v;
    return true;
}

}  // namespace text_parse
