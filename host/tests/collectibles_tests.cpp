#include "../src/collectibles_logic.cpp"
#include <algorithm>
#include <cassert>
#include <iostream>

using namespace collectibles;

static bool Has(const std::vector<FileOp>& ops, FileOp::Type type, const std::string& from, const std::string& to = {}) {
    return std::any_of(ops.begin(), ops.end(), [&](const FileOp& o) { return o.type == type && o.from == from && o.to == to; });
}

static void AssignTests() {
    using namespace collectibles;
    // Ids out of order on purpose: assignment goes by id.
    std::vector<Item> items(5);
    items[0].id = "C03", items[0].kind = Kind::Part;
    items[1].id = "C01", items[1].kind = Kind::Part;
    items[2].id = "C02", items[2].kind = Kind::Song;
    items[3].id = "C05", items[3].kind = Kind::Part;
    items[4].id = "C04", items[4].kind = Kind::Song;
    const auto a = Assign(items, "deck:a:1\tDeck A\tskatev_d_a\ngrip:g\tGrip G\tskatev_d_default,skatev_g_g\n");
    assert(a.at("C01").key == "deck:a:1" && a.at("C01").title == "Deck A" && a.at("C01").models.size() == 1);
    assert(a.at("C03").key == "grip:g" && a.at("C03").models.size() == 2 && a.at("C03").models[1] == "skatev_g_g");
    assert(a.at("C05").key.empty());  // more parts than secrets: none for the last
    assert(a.at("C02").track == kStarterSongs + 1 && a.at("C04").track == kStarterSongs + 2);
    assert(Assign(items, "").at("C01").key.empty());  // an old runtime: no list
    Progress p;
    p.reward["C01"] = "part:deck:a:1";
    p.reward["C02"] = "song:6";
    const auto keys = EarnedKeys(p);
    assert(keys.size() == 1 && keys[0] == "deck:a:1");
}

int main() {
    AssignTests();
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

    // Collectibles v3 (design 2026-11-03-collectibles-v3): boards, trucks,
    // wheels, air items.
    const auto v3 = ParsePlaces("D001\tboard\t1\t2\t3\tVespucci\nD002\tpart\t4\t5\t6\tDavis\tair\nD003\tpart\t7\t8\t9\tDavis\n");
    assert(v3.size() == 3 && v3[0].kind == Kind::Board && !v3[0].air && v3[1].air && !v3[2].air);
    const std::string list =
        "deck:a:1\tDeck A\tdA\tdA,t0,w0\ngrip:g1\tGrip 1\tdA,g1\ntrucks:t:1\tTrucks 1\tt1\nwheels:w:1\tWheels 1\tw1\n"
        "deck:a:2\tDeck B\tdB\tdB,t0,w0\ngrip:g2\tGrip 2\tdA,g2\n";
    const auto a3 = Assign(v3, list);
    // The board takes the first secret deck and shows its complete board; the
    // parts take the rest in order (griptape, trucks, ... before the next deck).
    assert((a3.at("D001").keys == std::vector<std::string>{"deck:a:1"}));
    assert(a3.at("D001").title == "Deck A");
    assert((a3.at("D001").models == std::vector<std::string>{"dA", "t0", "w0"}));
    assert(a3.at("D002").key == "grip:g1" && a3.at("D003").key == "trucks:t:1");
    Progress boardWon = ParseProgress("collected D001 board:deck:a:1\ncollected D002 part:grip:g1\n");
    assert(EarnedKeys(boardWon).size() == 2);
    assert(PopupFound(Kind::Board) == "Hidden board found");
    assert(PopupEarned("board:deck:a:1,grip:g1", "Deck A") == "Board unlocked: Deck A");
    assert(PopupEarned("part:trucks:t:1", "Trucks 1") == "Trucks unlocked: Trucks 1");
    assert(PopupEarned("part:wheels:w:1", "Wheels 1") == "Wheels unlocked: Wheels 1");
    // Air: Skate's air states, never a grind, walking or a wipeout.
    assert(BoardInAir("PhysicsAir") && BoardInAir("KnownAir") && BoardInAir("PhysicsAirSecondary"));
    assert(!BoardInAir("PhysicsGround") && !BoardInAir("GrindFiveO") && !BoardInAir("BipedGround") && !BoardInAir("WipeoutAir"));

    std::cout << "collectibles tests passed\n";
    return 0;
}
