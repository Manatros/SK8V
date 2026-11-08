// Skate Shop stall helpers (design 2026-11-07-skate-shop-rework).
#include "../src/shop_stall.h"
#include <cassert>
#include <cmath>
#include <iostream>

using namespace shopstall;

static bool Ortho(const float (&m)[16]) {
    for (int a = 0; a < 3; ++a)
        for (int b = 0; b < 3; ++b) {
            float d = 0;
            for (int k = 0; k < 3; ++k) d += m[a * 4 + k] * m[b * 4 + k];
            if (std::abs(d - (a == b ? 1.0f : 0.0f)) > 1e-3f) return false;
        }
    return true;
}

int main() {
    // Outline: the marks inside, the anchor (outside, at the front) and the boardwalk outside.
    assert(InStall(-1227.43f, -1439.43f));   // 1104-2
    assert(InStall(-1224.49f, -1439.31f));   // 1104-3
    assert(InStall(-1226.07f, -1438.69f));   // 1104-4
    assert(InStall(kSeller[0], kSeller[1]));
    assert(!InStall(-1224.45f, -1442.68f, 0.0f));  // the anchor stands before the front
    assert(!InStall(-1220.0f, -1445.0f));
    assert(!InStall(-1235.0f, -1435.0f));

    // The seller stands behind the glass counter, clear of its panes and the back wall.
    auto segDist = [](const float* a, const float* b, float x, float y) {
        const float ex = b[0] - a[0], ey = b[1] - a[1];
        float t = ((x - a[0]) * ex + (y - a[1]) * ey) / (ex * ex + ey * ey);
        t = t < 0 ? 0 : t > 1 ? 1 : t;
        return std::hypot(x - a[0] - ex * t, y - a[1] - ey * t);
    };
    for (int i = 0; i < 3; ++i) assert(segDist(kCounter[i], kCounter[i + 1], kSeller[0], kSeller[1]) > 0.4f);
    assert(segDist(kBackWall[0], kBackWall[1], kSeller[0], kSeller[1]) > 0.4f);
    // On the far side of the register's arm from the customer (1104-4 stood before the counter).
    const auto side = [](const float* a, const float* b, float x, float y) { return (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]); };
    assert(side(kCounter[2], kCounter[3], kSeller[0], kSeller[1]) * side(kCounter[2], kCounter[3], -1226.07f, -1438.69f) < 0);
    // Facing the customer's side: the heading's forward points back across the arm.
    const float h = kSeller[3] * 3.14159265f / 180.0f;
    const float fx = -std::sin(h), fy = std::cos(h);
    assert(side(kCounter[2], kCounter[3], kSeller[0] + fx, kSeller[1] + fy) * side(kCounter[2], kCounter[3], kSeller[0], kSeller[1]) < 0);

    float spot[4] = {kSeller[0], kSeller[1], kSeller[2], kSeller[3]};
    assert(ParseSpot("-1227.1, -1437.2, 3.40, 200", spot) && std::abs(spot[3] - 200.0f) < 1e-4f);
    assert(!ParseSpot("1,2,3", spot) && std::abs(spot[0] + 1227.1f) < 1e-3f);
    assert(!ParseSpot("1,2,3,4,5", spot));
    assert(!ParseSpot("a,b,c,d", spot));
    assert(!ParseSpot("", spot));

    // Seller: open and near -> there; closed with the player inside -> stays.
    assert(SellerWanted(true, 10.0f, false));
    assert(!SellerWanted(true, 70.0f, false));
    assert(!SellerWanted(false, 6.0f, true));           // closed: none made, even inside
    assert(!SellerGoes(false, 6.0f, true));             // closing while inside: stays
    assert(SellerGoes(false, 20.0f, false));            // closed, outside: goes
    assert(!SellerGoes(true, 70.0f, false));            // open, 60-80 m: kept (hysteresis)
    assert(SellerGoes(true, 81.0f, false));

    // Flight: endpoints exact, rigid half-way, also for a 180 deg turn.
    float a[16] = {1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 1, 2, 3, 1};
    float b[16] = {-1, 0, 0, 0, 0, -1, 0, 0, 0, 0, 1, 0, 4, 6, 3, 1};  // 180 deg about Z
    float m[16];
    Flight(a, b, 0.0f, m);
    for (int i = 0; i < 16; ++i) assert(std::abs(m[i] - a[i]) < 1e-4f);
    Flight(a, b, 1.0f, m);
    for (int i = 0; i < 16; ++i) assert(std::abs(m[i] - b[i]) < 1e-4f);
    Flight(a, b, 0.5f, m);
    assert(Ortho(m) && std::isfinite(m[0]));
    assert(std::abs(m[12] - 2.5f) < 1e-4f && std::abs(m[13] - 4.0f) < 1e-4f);
    float c[16] = {0, 0, -1, 0, 0, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1};  // 90 deg about Y
    Flight(a, c, 0.3f, m);
    assert(Ortho(m));
    assert(std::abs(Smoothstep(0.5f) - 0.5f) < 1e-6f && Smoothstep(-1.0f) == 0.0f && Smoothstep(2.0f) == 1.0f);

    // The rack board and the highlighted offer.
    const std::vector<std::string> rack = {"deck_a", "grip_b", "truck_c", "wheel_d"};
    assert(SameBoard(rack, {"deck_a", "grip_b", "truck_c", "wheel_d"}));
    assert(SameBoard(rack, {"deck_a", "grip_b", "truck_c", "wheel_d", "missing_bolts"}));
    assert(!SameBoard(rack, {"deck_a", "grip_x", "truck_c", "wheel_d"}));
    assert(!SameBoard(rack, {"deck_z", "grip_b", "truck_c", "wheel_d"}));
    assert(!SameBoard({}, {"deck_a"}));
    // Offers wear the player's own grip/paint: fall back to the deck.
    const std::vector<std::vector<std::string>> racks = {{"deck_q", "grip_b"}, rack};
    assert(RackFor(racks, {"deck_a", "grip_b", "truck_c", "wheel_d"}) == 1);
    assert(RackFor(racks, {"deck_a", "grip_own", "tb_paint", "th_paint", "wheel_d"}) == 1);
    assert(RackFor(racks, {"deck_none", "grip_b"}) == -1);

    std::cout << "shop stall tests passed\n";
    return 0;
}
