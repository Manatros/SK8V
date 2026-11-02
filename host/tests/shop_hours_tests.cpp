#include "../src/shop_hours.h"
#include <cassert>
#include <iostream>

int main() {
    using namespace shophours;
    Hours h;
    assert(Parse("8-22", h) && h.open == 8 && h.close == 22);
    assert(!OpenAt(7, h) && OpenAt(8, h) && OpenAt(21, h) && !OpenAt(22, h) && !OpenAt(0, h));
    assert(Text(h) == "08:00-22:00");
    assert(Parse(" 0 - 24 ", h) && h.always());
    for (int hour = 0; hour < 24; ++hour) assert(OpenAt(hour, h));
    // Across midnight.
    assert(Parse("20-4", h) && OpenAt(20, h) && OpenAt(23, h) && OpenAt(3, h) && !OpenAt(4, h) && !OpenAt(12, h));
    // Bad text: always open.
    assert(!Parse("", h) && h.always());
    assert(!Parse("nine to five", h) && h.always());
    assert(!Parse("8-30", h) && h.always());
    assert(!Parse("8-22x", h) && h.always());
    std::cout << "shop_hours_tests passed\n";
    return 0;
}
