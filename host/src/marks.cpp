#include "marks.h"

#include <windows.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <sstream>

namespace marks {
namespace {

std::vector<std::string> Split(const std::string& line) {
    std::vector<std::string> out;
    size_t start = 0;
    for (size_t i = 0; i <= line.size(); ++i) {
        if (i == line.size() || line[i] == '\t') {
            out.push_back(line.substr(start, i - start));
            start = i + 1;
        }
    }
    return out;
}

bool Number(const std::string& text, float& out) {
    if (text.empty()) return false;
    char* end = nullptr;
    const float v = std::strtof(text.c_str(), &end);
    if (end != text.c_str() + text.size() || !std::isfinite(v)) return false;
    out = v;
    return true;
}

std::string Clean(std::string s) {
    for (char& c : s)
        if (c == '\t' || c == '\r' || c == '\n') c = ' ';
    return s;
}

}  // namespace

File Parse(const std::string& text) {
    File file;
    std::istringstream in(text);
    std::string line;
    bool first = true;
    while (std::getline(in, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (first) {
            first = false;
            if (line == kHeader) continue;
        }
        if (line.empty()) continue;
        file.lines.push_back(line);
        const auto f = Split(line);
        Mark m;
        if (f.size() < 6 || f[0].empty() || !Number(f[1], m.x) || !Number(f[2], m.y) || !Number(f[3], m.z) ||
            !Number(f[4], m.heading) || (f[5] != "open" && f[5] != "fixed"))
            continue;
        m.id = f[0];
        m.fixed = f[5] == "fixed";
        if (f.size() > 6) m.note = f[6];
        file.marks.push_back(m);
        file.line.push_back(file.lines.size() - 1);
    }
    return file;
}

std::string FormatMark(const Mark& m) {
    char pos[96];
    std::snprintf(pos, sizeof(pos), "%.2f\t%.2f\t%.2f\t%.0f", m.x, m.y, m.z, m.heading);
    return Clean(m.id) + "\t" + pos + "\t" + (m.fixed ? "fixed" : "open") + "\t" + Clean(m.note);
}

std::string Format(const File& file) {
    std::string out = std::string(kHeader) + "\n";
    for (const auto& l : file.lines) out += l + "\n";
    return out;
}

int Step(const File& file, int from, int direction, bool showFixed) {
    const int n = static_cast<int>(file.marks.size());
    if (n == 0) return -1;
    int i = from < 0 || from >= n ? (direction >= 0 ? -1 : 0) : from;
    for (int k = 0; k < n; ++k) {
        i = ((i + (direction >= 0 ? 1 : -1)) % n + n) % n;
        if (showFixed || !file.marks[i].fixed) return i;
    }
    return -1;
}

void SetFixed(File& file, int index, bool fixed) {
    if (index < 0 || index >= static_cast<int>(file.marks.size())) return;
    file.marks[index].fixed = fixed;
    file.lines[file.line[index]] = FormatMark(file.marks[index]);
}

void Remove(File& file, int index) {
    if (index < 0 || index >= static_cast<int>(file.marks.size())) return;
    const size_t at = file.line[index];
    file.lines.erase(file.lines.begin() + static_cast<std::ptrdiff_t>(at));
    file.marks.erase(file.marks.begin() + index);
    file.line.erase(file.line.begin() + index);
    for (auto& l : file.line)
        if (l > at) --l;
}

void Append(File& file, const Mark& m) {
    file.lines.push_back(FormatMark(m));
    file.marks.push_back(m);
    file.line.push_back(file.lines.size() - 1);
}

std::string SessionId(int month, int day, int number) {
    char id[32];
    std::snprintf(id, sizeof(id), "%02d%02d-%d", month, day, number);
    return id;
}

File Load(const std::wstring& path) {
    std::ifstream in(path, std::ios::binary);
    if (!in) return {};
    std::ostringstream text;
    text << in.rdbuf();
    return Parse(text.str());
}

bool Save(const std::wstring& path, const File& file) {
    const std::wstring tmp = path + L".tmp";
    {
        std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
        if (!out) return false;
        const std::string text = Format(file);
        out.write(text.data(), static_cast<std::streamsize>(text.size()));
        if (!out.flush()) return false;
    }
    return MoveFileExW(tmp.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) != 0;
}

}  // namespace marks
