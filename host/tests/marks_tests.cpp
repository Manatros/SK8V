#include "../src/marks.cpp"
#include <cassert>
#include <filesystem>
#include <iostream>

int main() {
    using namespace marks;
    const std::string text = std::string(kHeader) + "\r\n"
        "1029-M34\t-937.97\t-787.44\t15.92\t341\topen\tLittle Seoul halfpipe side broken\r\n"
        "broken line without tabs\r\n"
        "1029-M28\t-1395.45\t-1390.81\t4.00\t176\tfixed\tHandrail (control) \xC3\xBC\r\n"
        "1029-M39\t724.86\tnan-ish\t24.68\t215\topen\tbad number\r\n"
        "1029-M44\t711.83\t-1228.02\t24.67\t84\topen\r\n";
    File f = Parse(text);
    assert(f.lines.size() == 5 && f.marks.size() == 3);
    assert(f.marks[0].id == "1029-M34" && f.marks[0].note == "Little Seoul halfpipe side broken");
    assert(f.marks[1].fixed && f.marks[1].note == "Handrail (control) \xC3\xBC");
    assert(f.marks[2].id == "1029-M44" && f.marks[2].note.empty());
    // Malformed lines come back unchanged, in place.
    const std::string round = Format(f);
    assert(round.find("broken line without tabs\n") != std::string::npos);
    assert(round.find("1029-M39\t724.86\tnan-ish") != std::string::npos);
    assert(Parse(round).marks.size() == 3);

    // Stepping wraps and skips fixed marks unless shown.
    assert(Step(f, -1, +1, false) == 0);
    assert(Step(f, 0, +1, false) == 2);
    assert(Step(f, 2, +1, false) == 0);
    assert(Step(f, 0, -1, false) == 2);
    assert(Step(f, 0, +1, true) == 1);
    File none = Parse(std::string(kHeader) + "\n");
    assert(Step(none, -1, +1, true) == -1);

    // Status and delete rewrite only their own line.
    SetFixed(f, 0, true);
    assert(Format(f).find("1029-M34\t-937.97\t-787.44\t15.92\t341\tfixed\tLittle Seoul") != std::string::npos);
    assert(Step(f, -1, +1, false) == 2);
    Remove(f, 1);
    assert(f.marks.size() == 2 && f.lines.size() == 4 && f.marks[1].id == "1029-M44");
    assert(f.lines[f.line[1]].rfind("1029-M44", 0) == 0);

    // Appended session marks; tabs in notes cannot break the format.
    Append(f, {SessionId(10, 31, 3), 1.5f, 2.5f, 3.5f, 90.0f, false, "a\tb"});
    assert(f.marks.back().id == "1031-3");
    assert(Parse(Format(f)).marks.back().note == "a b");

    // Save replaces the file through a temporary one.
    const auto dir = std::filesystem::temp_directory_path() / ("marks-test-" + std::to_string(GetCurrentProcessId()));
    std::filesystem::create_directories(dir);
    const std::wstring path = (dir / "marks.tsv").wstring();
    assert(Load(path).marks.empty());
    assert(Save(path, f));
    assert(Save(path, f));
    assert(Load(path).marks.size() == f.marks.size());
    assert(!std::filesystem::exists(path + L".tmp"));
    std::filesystem::remove_all(dir);
    std::cout << "marks_tests OK\n";
}
