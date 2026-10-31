#pragma once
// Test marks: positions Manatros reported, kept in GTAxSkate\marks.tsv so he
// can travel to each one and set it fixed (design 2026-10-31-mark-travel).
// No GTA calls here: the menu wires these to the teleport natives.
#include <string>
#include <vector>

namespace marks {

struct Mark {
    std::string id;
    float x = 0, y = 0, z = 0, heading = 0;
    bool fixed = false;
    std::string note;
};

// The file as read: parsed marks and every original line (malformed lines are
// written back unchanged). `line` maps a mark to its line in `lines`.
struct File {
    std::vector<std::string> lines;  // without the header
    std::vector<Mark> marks;
    std::vector<size_t> line;
};

inline constexpr const char* kHeader = "id\tx\ty\tz\theading\tstatus\tnote";

File Parse(const std::string& text);
std::string Format(const File& file);
std::string FormatMark(const Mark& m);

// The next/previous selectable mark after `from` (wrapping); -1 if none.
// Fixed marks are skipped unless `showFixed`.
int Step(const File& file, int from, int direction, bool showFixed);

void SetFixed(File& file, int index, bool fixed);
void Remove(File& file, int index);
void Append(File& file, const Mark& m);

// "MMDD-n" for a spot marked in this session.
std::string SessionId(int month, int day, int number);

// Reads `path` (missing file: empty list). Writes via a temporary file in the
// same folder that then replaces the original (never half a file).
File Load(const std::wstring& path);
bool Save(const std::wstring& path, const File& file);

}  // namespace marks
