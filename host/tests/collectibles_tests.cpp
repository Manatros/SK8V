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

static void FiniteRewardTests() {
    auto items = ParsePlaces("D001\tboard\t0\t0\t0\tpark\nD002\tpart\t0\t0\t0\tground\nD003\tpart\t0\t0\t0\tsurplus\nD004\tsong\t0\t0\t0\tsong6\nD005\tsong\t0\t0\t0\tsong7\nL01-1\tline\t0\t0\t0\tline\nL01-2\tline\t1\t0\t0\tline\n");
    auto all = items;
    const auto boards = LineBoards(items);
    all.insert(all.end(), boards.begin(), boards.end());
    const std::string secrets = "deck:a\tDeck\tmodel\ngrip:b\tGrip\tmodel\n";
    const auto assigned = Assign(all, secrets);
    const std::set<int> tracks = {1, 2, 7}; // actual sparse music inventory
    assert(HasReward(items[0], assigned, tracks));
    assert(HasReward(items[1], assigned, tracks));
    assert(!HasReward(items[2], assigned, tracks)); // surplus part
    assert(!HasReward(items[3], assigned, tracks)); // absent fixed song6
    assert(HasReward(items[4], assigned, tracks)); // song7 never shifts to song6's id
    assert(!HasReward(items[5], assigned, tracks)); // surplus line board
    Progress p;
    p.state["old"] = State::Collected;
    p.reward["old"] = "cash:500";
    const auto before = FormatProgress(p);
    for (const auto& bad : {"", "cash:500", "part:", "board:", "song:0", "song:6junk"})
        assert(!RecordReward(p, "D002", bad));
    assert(FormatProgress(p) == before); // no empty reward consumes progress
    assert(RecordReward(p, "D001", "board:deck:a"));
    assert(!RecordReward(p, "D001", "board:deck:other"));
    p = ParseProgress(FormatProgress(p)); // restart preserves old cash and earned key
    assert(p.reward.at("old") == "cash:500");
    assert(EarnedKeys(p) == std::vector<std::string>{"deck:a"});
    const auto again = Assign(all, secrets);
    for (const Item& it : all) {
        assert(HasReward(it, assigned, tracks) == HasReward(it, again, tracks));
        assert(assigned.at(it.kind == Kind::Line ? LineGroup(it.id) : it.id).key ==
               again.at(it.kind == Kind::Line ? LineGroup(it.id) : it.id).key);
    }
    assert(!HasReward(items[2], again, tracks) && !HasReward(items[5], again, tracks));
    assert(!HasReward(items[3], again, tracks));
    assert(HasReward(items[3], again, {6, 7})); // restoring its file restores only that pickup
    assert(!HasReward(items[0], Assign(all, ""), tracks));
    assert(HasReward(items[4], Assign(all, ""), tracks)); // songs work with no secret list
}

int main() {
    AssignTests();
    FiniteRewardTests();
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

    // Trick lines (rev 4).
    const auto lines = ParsePlaces("L01-1\tline\t1\t2\t3\tVespucci\nL01-2\tline\t4\t5\t6\tVespucci\tair\nL01-3\tline\t7\t8\t9\tVespucci\n");
    assert(lines.size() == 3 && lines[0].kind == Kind::Line && lines[1].air);
    assert(LineGroup("L01-2") == "L01" && LineStep("L01-2") == 2 && LineStep("C01") == 0);
    const auto lb = LineBoards(lines);
    assert(lb.size() == 1 && lb[0].id == "L01" && lb[0].kind == Kind::Board);
    LineRun run;
    assert(LineUpdate(run, "L01", 2, 3, 100, false) == LineEvent::None);  // not from the middle
    assert(LineUpdate(run, "L01", 1, 3, 100, false) == LineEvent::Started && run.next == 2);
    assert(LineUpdate(run, "", 0, 3, 5000, false) == LineEvent::None);
    assert(LineUpdate(run, "L01", 2, 3, 5000, false) == LineEvent::Next && run.next == 3);
    assert(LineUpdate(run, "L01", 3, 3, 9000, false) == LineEvent::Done && run.line.empty());
    LineRun slow;
    LineUpdate(slow, "L01", 1, 3, 0, false);
    assert(LineUpdate(slow, "", 0, 3, kLegMs + 1, false) == LineEvent::Failed);
    LineRun bail;
    LineUpdate(bail, "L01", 1, 3, 0, false);
    assert(LineUpdate(bail, "", 0, 3, 10, true) == LineEvent::Failed);
    // A board earned unlocks only its deck (rev 4): the reward names the deck.
    assert(PopupEarned("board:deck:a:1", "Deck A") == "Board unlocked: Deck A");

    // Safe developer collectible travel (design 2026-11-05-safe-collectible-travel).
    {
        float z1 = 0.0f, z2 = 0.0f;
        TravelCapsuleSpheres(10.0f, z1, z2);
        assert(std::abs(z1 - 10.45f) < 1e-5f);
        assert(std::abs(z2 - 11.55f) < 1e-5f);

        float foot = 0.0f;
        TravelBounds goodBounds{true, {-0.30f, -0.30f, -1.00f}, {0.30f, 0.30f, 0.85f}};
        assert(TravelCheckBounds(goodBounds, foot) && std::abs(foot - 1.00f) < 1e-5f);
        TravelBounds wideBounds{true, {-0.50f, -0.30f, -1.00f}, {0.30f, 0.30f, 0.85f}};
        assert(!TravelCheckBounds(wideBounds, foot));
        TravelBounds tallBounds{true, {-0.30f, -0.30f, -1.20f}, {0.30f, 0.30f, 0.95f}};
        assert(!TravelCheckBounds(tallBounds, foot));

        const float marker[3] = {100.0f, 200.0f, 15.0f};
        const float req[2] = {104.0f, 200.0f};
        const float navOk[3] = {104.2f, 199.8f, 13.8f};
        assert(TravelCheckSafeCoord(marker, req, navOk));
        const float navTooClose[3] = {102.0f, 200.0f, 13.8f};  // 2.0 m < 2.5 m from marker
        assert(!TravelCheckSafeCoord(marker, req, navTooClose));
        const float navWrongLevel[3] = {104.0f, 200.0f, 11.5f};  // 3.5 m below marker
        assert(!TravelCheckSafeCoord(marker, req, navWrongLevel));
        const float navTooHigh[3] = {104.0f, 200.0f, 14.8f};  // only 0.2 m below marker
        assert(!TravelCheckSafeCoord(marker, req, navTooHigh));

        struct MockEnv {
            bool hasSelection = true;
            std::string id = "D001";
            std::string label = "Collectible: D001 board [hidden] Vespucci";
            float marker[3] = {100.0f, 200.0f, 15.0f};
            bool sceneActive = false;
            bool sceneLoaded = false;
            bool startSceneOk = true;
            int startCount = 0;
            int stopCount = 0;
            int reqCollisionCount = 0;
            int teleportCount = 0;
            int lastPed = 0;
            float teleportPos[3] = {0, 0, 0};
            int waypointCount = 0;
            float waypointPos[2] = {0, 0};
            TravelBounds bounds{true, {-0.30f, -0.30f, -0.98f}, {0.30f, 0.30f, 0.82f}};
            bool safeCoordOk = true;
            float navOffset[3] = {0.0f, 0.0f, -1.2f};  // relative to (req.x, req.y, marker.z)
            int probeIdx = 0;
            TravelLosSample floorSample{2, true, true, 13.82f, 0.99f};
            float sampleSpread = 0.0f;
            TravelCapsuleResult capsule{2, false};
        };

        auto makeAdapter = [](MockEnv& env) {
            TravelAdapter api{};
            api.ctx = &env;
            api.selectedItem = [](std::string& id, std::string& label, float (&xyz)[3], void* ctx) {
                auto& e = *static_cast<MockEnv*>(ctx);
                if (!e.hasSelection) return false;
                id = e.id;
                label = e.label;
                for (int i = 0; i < 3; ++i) xyz[i] = e.marker[i];
                return true;
            };
            api.isLoadSceneActive = [](void* ctx) { return static_cast<MockEnv*>(ctx)->sceneActive; };
            api.isLoadSceneLoaded = [](void* ctx) { return static_cast<MockEnv*>(ctx)->sceneLoaded; };
            api.requestCollision = [](float, float, float, void* ctx) { ++static_cast<MockEnv*>(ctx)->reqCollisionCount; };
            api.startLoadSceneSphere = [](float, float, float, float, void* ctx) {
                auto& e = *static_cast<MockEnv*>(ctx);
                ++e.startCount;
                if (!e.startSceneOk) return false;
                e.sceneActive = true;
                return true;
            };
            api.stopLoadScene = [](void* ctx) {
                auto& e = *static_cast<MockEnv*>(ctx);
                ++e.stopCount;
                e.sceneActive = false;
            };
            api.getPedBounds = [](int, void* ctx) { return static_cast<MockEnv*>(ctx)->bounds; };
            api.getSafeCoord = [](float x, float y, float z, float (&out)[3], void* ctx) {
                auto& e = *static_cast<MockEnv*>(ctx);
                if (!e.safeCoordOk) return false;
                out[0] = x + e.navOffset[0];
                out[1] = y + e.navOffset[1];
                out[2] = z + e.navOffset[2];
                return true;
            };
            api.probeFloor = [](float, float, float, float, int, void* ctx) {
                auto& e = *static_cast<MockEnv*>(ctx);
                TravelLosSample s = e.floorSample;
                if (e.sampleSpread != 0.0f && (e.probeIdx++ % 5) == 1) s.z += e.sampleSpread;
                return s;
            };
            api.probeCapsule = [](float, float, float, float, float, int, void* ctx) {
                return static_cast<MockEnv*>(ctx)->capsule;
            };
            api.teleportPed = [](int ped, float x, float y, float z, void* ctx) {
                auto& e = *static_cast<MockEnv*>(ctx);
                ++e.teleportCount;
                e.lastPed = ped;
                e.teleportPos[0] = x, e.teleportPos[1] = y, e.teleportPos[2] = z;
            };
            api.setWaypoint = [](float x, float y, void* ctx) {
                auto& e = *static_cast<MockEnv*>(ctx);
                ++e.waypointCount;
                e.waypointPos[0] = x, e.waypointPos[1] = y;
            };
            return api;
        };

        auto readyGuard = [] {
            TravelGuard g{};
            g.ready = true;
            g.onFoot = true;
            g.ped = 42;
            g.pos[0] = 10.0f, g.pos[1] = 20.0f, g.pos[2] = 5.0f;
            return g;
        };

        // 1. Valid standing arrival: streams, verifies floor & capsule, teleports to supportZ + footOffset.
        {
            MockEnv env;
            TravelAdapter api = makeAdapter(env);
            TravelSession s;
            const TravelGuard g = readyGuard();
            assert(TravelRequest(s, g, 1000, api).empty());
            assert(s.active && s.ownsScene && env.startCount == 1);
            // Not loaded yet on frame 1: stays active, no teleport.
            assert(TravelTick(s, g, 1050, api).empty());
            assert(s.active && env.teleportCount == 0);
            // Scene loaded on frame 2: teleports to first candidate (104, 200, 13.82 + 0.98 = 14.80).
            env.sceneLoaded = true;
            const std::string msg = TravelTick(s, g, 1100, api);
            assert(msg == env.label);
            assert(!s.active && env.stopCount == 1 && env.teleportCount == 1 && env.waypointCount == 0);
            assert(env.lastPed == 42);
            assert(std::abs(env.teleportPos[0] - 104.0f) < 1e-4f);
            assert(std::abs(env.teleportPos[1] - 200.0f) < 1e-4f);
            assert(std::abs(env.teleportPos[2] - (13.82f + 0.98f)) < 1e-4f);
        }

        // 2. Floor missing / steep slope / uneven spread / dynamic entity -> zero movement, GPS set.
        for (int badCase = 0; badCase < 4; ++badCase) {
            MockEnv env;
            env.sceneLoaded = true;
            if (badCase == 0) env.floorSample.hit = false;
            if (badCase == 1) env.floorSample.normalZ = 0.85f;      // too steep (< 0.94)
            if (badCase == 2) env.sampleSpread = 0.15f;             // uneven (> 0.10 m)
            if (badCase == 3) env.floorSample.worldEntity = false;  // dynamic prop/vehicle
            TravelAdapter api = makeAdapter(env);
            TravelSession s;
            const TravelGuard g = readyGuard();
            assert(TravelRequest(s, g, 1000, api).empty());
            const std::string msg = TravelTick(s, g, 1050, api);
            assert(msg.find("no safe floor verified") != std::string::npos);
            assert(!s.active && env.teleportCount == 0 && env.stopCount == 1 && env.waypointCount == 1);
        }

        // 3. Wall / headroom hit or incomplete capsule probe -> zero movement, GPS set.
        for (int capCase = 0; capCase < 2; ++capCase) {
            MockEnv env;
            env.sceneLoaded = true;
            if (capCase == 0) env.capsule = {2, true};   // blocked headroom/wall
            if (capCase == 1) env.capsule = {1, false};  // incomplete shape test
            TravelAdapter api = makeAdapter(env);
            TravelSession s;
            const TravelGuard g = readyGuard();
            assert(TravelRequest(s, g, 1000, api).empty());
            const std::string msg = TravelTick(s, g, 1050, api);
            assert(msg.find("no safe floor verified") != std::string::npos);
            assert(!s.active && env.teleportCount == 0 && env.stopCount == 1 && env.waypointCount == 1);
        }

        // 4. Preexisting load scene is never hijacked or stopped.
        {
            MockEnv env;
            env.sceneActive = true;  // GTA already owns a load scene
            TravelAdapter api = makeAdapter(env);
            TravelSession s;
            const std::string msg = TravelRequest(s, readyGuard(), 1000, api);
            assert(msg.find("load scene already active") != std::string::npos);
            assert(!s.active && env.startCount == 0 && env.stopCount == 0 && env.teleportCount == 0 && env.waypointCount == 1);
        }

        // 5. Mode & state refusals: in vehicle, skating, lingering pose/cam handoff, replay.
        for (int modeCase = 0; modeCase < 5; ++modeCase) {
            MockEnv env;
            TravelAdapter api = makeAdapter(env);
            TravelSession s;
            TravelGuard g = readyGuard();
            if (modeCase == 0) g.inVehicle = true;
            if (modeCase == 1) g.ready = false;
            if (modeCase == 2) g.poseFadingOut = true;
            if (modeCase == 3) g.fadingCam = true;
            if (modeCase == 4) g.replayActive = true;
            const std::string msg = TravelRequest(s, g, 1000, api);
            assert(msg.find("refused") != std::string::npos);
            assert(!s.active && env.startCount == 0 && env.teleportCount == 0 && env.waypointCount == 1);
        }

        // 6. Cancellation during streaming when player moves, selection changes, ped changes, or timeout.
        {
            MockEnv env;
            TravelAdapter api = makeAdapter(env);
            TravelSession s;
            TravelGuard g = readyGuard();
            assert(TravelRequest(s, g, 1000, api).empty());
            g.pos[0] += 0.50f;  // moved > 0.30 m
            const std::string msg = TravelTick(s, g, 1100, api);
            assert(msg.find("player moved") != std::string::npos);
            assert(!s.active && env.stopCount == 1 && env.teleportCount == 0);
        }
        {
            MockEnv env;
            TravelAdapter api = makeAdapter(env);
            TravelSession s;
            const TravelGuard g = readyGuard();
            assert(TravelRequest(s, g, 1000, api).empty());
            const std::string msg = TravelTick(s, g, 1000 + kTravelTimeoutMs + 1, api);
            assert(msg.find("timed out") != std::string::npos);
            assert(!s.active && env.stopCount == 1 && env.teleportCount == 0 && env.waypointCount == 1);
        }
    }

    // Hall of Meat and banked trick-line high-score milestones: 20 tiers (R01..R20),
    // sorting after D151 and L06 so existing assignments never shift.
    {
        const auto milestones = ScoreMilestones();
        assert(milestones.size() == 20);
        assert(milestones.front().id == "R01" && milestones.back().id == "R20");
        for (const Item& m : milestones) {
            assert(m.kind == Kind::Part);
            assert(m.id > "D151" && m.id > "L06");
        }

        // Assign stability: D001 (board), L01 (board), D082 (part) keep identical
        // secrets when R01..R20 are appended.
        const std::vector<Item> base = {
            {"D001", Kind::Board, 0, 0, 0, "Park"},
            {"L01", Kind::Board, 0, 0, 0, "Line"},
            {"D082", Kind::Part, 0, 0, 0, "Scattered"},
        };
        std::string list;
        for (int i = 1; i <= 25; ++i)
            list += "deck:m:d" + std::to_string(i) + "\tDeck " + std::to_string(i) + "\tskatev_d_d" + std::to_string(i) +
                    "\tskatev_d_d" + std::to_string(i) + ",skatev_t_default,skatev_w_default\n";
        const auto a1 = Assign(base, list);
        std::vector<Item> withScores = base;
        withScores.insert(withScores.end(), milestones.begin(), milestones.end());
        const auto a2 = Assign(withScores, list);
        assert(a1.at("D001").key == a2.at("D001").key);
        assert(a1.at("L01").key == a2.at("L01").key);
        assert(a1.at("D082").key == a2.at("D082").key);
        assert(a2.at("R01").key == "deck:m:d4");
        assert(a2.at("R20").key == "deck:m:d23");

        Progress p;
        assert(ScoreEarned(milestones, p, 1, 0).empty());
        assert(ScoreEarned(milestones, p, 1, 4999).empty());
        // Hall of Meat 61,531 pts earns R01..R09 (5,000..60,000), leaving R10 (75,000).
        const auto hom = ScoreEarned(milestones, p, 1, 61531);
        assert(hom.size() == 9 && hom.front().id == "R01" && hom.back().id == "R09");
        for (const Item& it : hom) assert(RecordReward(p, it.id, "part:" + a2.at(it.id).key));
        assert(ScoreEarned(milestones, p, 1, 61531).empty());
        const auto homTop = ScoreEarned(milestones, p, 1, 75000);
        assert(homTop.size() == 1 && homTop[0].id == "R10");

        // Trick line 8,605 pts earns R11..R18 (500..8,000), leaving R19 (10,000) and R20 (12,500).
        const auto line = ScoreEarned(milestones, p, 2, 8605);
        assert(line.size() == 8 && line.front().id == "R11" && line.back().id == "R18");
    }

    std::cout << "collectibles tests passed\n";
    return 0;
}
