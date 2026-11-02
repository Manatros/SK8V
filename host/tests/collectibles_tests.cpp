#include "../src/collectibles_logic.cpp"
#include <algorithm>
#include <cassert>
#include <iostream>

using namespace collectibles;

static bool Has(const std::vector<FileOp>& ops, FileOp::Type type, const std::string& from, const std::string& to = {}) {
    return std::any_of(ops.begin(), ops.end(), [&](const FileOp& o) { return o.type == type && o.from == from && o.to == to; });
}

int main() {
    // Places: comments, the header and malformed lines are skipped.
    const auto places = ParsePlaces(
        "# seed 1\r\nid\tkind\tx\ty\tz\tplace\r\n"
        "C01\tpart\t-1350.91\t-1386.68\t4.63\tVespucci Beach Skatepark\r\n"
        "C02\tsong\t264.32\t-211.35\t57.16\tHawick Skate Spot\n"
        "C03\tcoin\t1\t2\t3\tunknown kind\n"
        "C04\tpart\t1\tnope\t3\tbad number\n");
    assert(places.size() == 2);
    assert(places[0].id == "C01" && places[0].kind == Kind::Part && places[0].place == "Vespucci Beach Skatepark");
    assert(places[1].kind == Kind::Song && std::abs(places[1].z - 57.16f) < 1e-3f);

    // Progress round trip; collected wins over found.
    Progress p = ParseProgress("found C01\ncollected C02 song:06\nfound C02\ncollected C03 deck:deckmodel:d1\tDeck One\n");
    assert(StateOf(p, "C01") == State::Found && StateOf(p, "C02") == State::Collected && StateOf(p, "C09") == State::Hidden);
    assert(p.reward["C02"] == "song:06");
    const Progress back = ParseProgress(FormatProgress(p));
    assert(back.state == p.state && back.reward == p.reward);

    // Earning: a fast frame that passes the marker still counts.
    const float c[3] = {0, 0, 0};
    const float a[3] = {-3, 1.0f, 0}, b[3] = {3, 1.0f, 0};
    assert(SegmentHits(a, b, c, 1.5f));
    const float far1[3] = {-3, 2.0f, 0}, far2[3] = {3, 2.0f, 0};
    assert(!SegmentHits(far1, far2, c, 1.5f));
    const float still[3] = {0.5f, 0.5f, 0.5f};
    assert(SegmentHits(still, still, c, 1.5f));
    assert(MarkerScale(0) == 1.0f && MarkerScale(300) == 4.0f && MarkerScale(1000) == 4.0f);

    // Songs: names, the next unlock, and the folder plan.
    assert(TrackOf("Skate 3 Track 07.mp3") == 7 && TrackOf("Skate 3 Track 07.mp3.locked") == 7);
    assert(TrackOf("skate-music-files.txt") == 0 && TrackOf("Skate 3 Track 7.mp3") == 0);
    std::vector<int> tracks;
    for (int t = 1; t <= 46; ++t) tracks.push_back(t);
    assert(NextSong(p, tracks) == 7);  // 06 is earned
    Progress none;
    assert(NextSong(none, tracks) == 6);
    assert(NextSong(none, {1, 2, 3}) == 0);

    const std::vector<SongFile> names = {
        {"Skate 3 Track 01.mp3", 9},          // starter: stays
        {"Skate 3 Track 06.mp3.locked", 9},   // earned: back on the radio
        {"Skate 3 Track 07.mp3", 9},          // not earned: locked
        {"Skate 3 Track 08.mp3", 9}, {"Skate 3 Track 08.mp3.locked", 9},  // a tool rerun: stale copy goes, fresh one locked
        {"skate-music-files.txt", 1}};
    const auto ops = SongPlan(names, p, true);
    assert(Has(ops, FileOp::Rename, "Skate 3 Track 06.mp3.locked", "Skate 3 Track 06.mp3"));
    assert(Has(ops, FileOp::Rename, "Skate 3 Track 07.mp3", "Skate 3 Track 07.mp3.locked"));
    assert(Has(ops, FileOp::Delete, "Skate 3 Track 08.mp3.locked"));
    assert(Has(ops, FileOp::Rename, "Skate 3 Track 08.mp3", "Skate 3 Track 08.mp3.locked"));
    assert(ops.size() == 4);
    // The delete comes before the rename onto the same name.
    const auto del = std::find_if(ops.begin(), ops.end(), [](const FileOp& o) { return o.type == FileOp::Delete; });
    const auto ren = std::find_if(ops.begin(), ops.end(), [](const FileOp& o) { return o.from == "Skate 3 Track 08.mp3"; });
    assert(del < ren);
    // Collectibles off: everything playable again.
    const auto off = SongPlan(names, p, false);
    assert(Has(off, FileOp::Rename, "Skate 3 Track 06.mp3.locked", "Skate 3 Track 06.mp3"));
    assert(!Has(off, FileOp::Rename, "Skate 3 Track 07.mp3", "Skate 3 Track 07.mp3.locked"));
    assert(Has(off, FileOp::Delete, "Skate 3 Track 08.mp3.locked"));
    // A matched folder needs nothing.
    assert(SongPlan({{"Skate 3 Track 01.mp3", 9}, {"Skate 3 Track 09.mp3.locked", 9}}, none, true).empty());
    // A playable copy cut short (0 bytes) goes; the good locked one stays locked.
    const auto cut = SongPlan({{"Skate 3 Track 10.mp3", 0}, {"Skate 3 Track 10.mp3.locked", 9}}, none, true);
    assert(cut.size() == 1 && Has(cut, FileOp::Delete, "Skate 3 Track 10.mp3"));
    // ... and comes back when earned.
    Progress ten = ParseProgress("collected C05 song:10\n");
    const auto back10 = SongPlan({{"Skate 3 Track 10.mp3", 0}, {"Skate 3 Track 10.mp3.locked", 9}}, ten, true);
    assert(Has(back10, FileOp::Delete, "Skate 3 Track 10.mp3"));
    assert(Has(back10, FileOp::Rename, "Skate 3 Track 10.mp3.locked", "Skate 3 Track 10.mp3"));

    // Skate's centre popups (design 2026-11-02-collectible-popups).
    assert(PopupFound(Kind::Part) == "Hidden board part found");
    assert(PopupFound(Kind::Song) == "Hidden song found");
    assert(PopupEarned("part:deck:f2c5:2c7f", "Busenitz Seizures") == "Deck unlocked: Busenitz Seizures");
    assert(PopupEarned("part:grip:g1", "Black") == "Griptape unlocked: Black");
    assert(PopupEarned("song:7", "") == "New song: Skate 3 Track 07");
    assert(PopupEarned("cash:500", "") == "Nothing left to unlock: $500");

    std::cout << "collectibles tests passed\n";
    return 0;
}
