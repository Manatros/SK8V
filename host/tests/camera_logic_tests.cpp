// cl /std:c++20 /EHsc /W4 camera_logic_tests.cpp; no game or SDK needed.
#include "../src/camera_logic.h"
#include "../src/menu_presets.h"
#include <cassert>
#include <iostream>

int main() {
    // GTA's own cycle: near 0 -> middle 1 -> far 2 -> first person 4 -> near 0 (wrap: SK8V's camera next).
    assert(!camera::Wrapped(0, 1) && !camera::Wrapped(1, 2) && !camera::Wrapped(2, 4));
    assert(camera::Wrapped(4, 0));
    assert(camera::Wrapped(2, 0)); // a cycle without first person wraps from far
    assert(!camera::Wrapped(-1, 0) && !camera::Wrapped(0, 0));
    using namespace camera;
    // First session uses the configured value, including both legacy first-person IDs.
    for (int view = 0; view <= 5; ++view) {
        for (int mode : {0, 1, 2, 4}) {
            assert(StartView(view, true, -1, mode) == CanonicalView(view));
            assert(EntryFootMode(view, true, -1) == FootMode(view));
            assert(EntryFootMode(view, false, 2) == FootMode(view));
            for (int last = 0; last <= 5; ++last) assert(EntryFootMode(view, true, last) == -1);
            assert(StartView(view, false, 2, mode) == CanonicalView(view));
            assert(StartView(view, true, 0, mode) == 0);
            for (int last = 1; last <= 5; ++last)
                assert(StartView(view, true, last, mode) == ViewFromFootMode(mode));
        }
    }
    // A full V cycle includes exactly the original camera and four GTA views.
    const int views[] = {0, 1, 2, 3, 5, 0};
    const int modes[] = {-1, 0, 1, 2, 4, -1};
    for (int i = 0; i < 5; ++i) {
        assert(NextView(views[i]) == views[i + 1]);
        assert(FootMode(views[i]) == modes[i]);
    }
    assert(NextView(4) == 0 && FootMode(4) == 4);
    for (int mode : {0, 1, 2, 4}) assert(FootMode(ViewFromFootMode(mode)) == mode);
    assert(CanonicalView(-8) == 0 && CanonicalView(20) == 5);
    for (int view = 0; view <= 5; ++view) {
        assert(GameplayView(view, false, 0) == (view != 0));
        assert(GameplayView(view, true, 0));
        assert(!GameplayView(view, false, 1));
        assert(!GameplayView(view, true, 2));
    }
    const auto item = menu::Choice("View", "CameraView", {"Skate", "Near", "Middle", "Far", "Head"},
                                   {"Original SK8V / Skate 3", "GTA near", "GTA middle", "GTA far", "GTA first person"});
    for (const char* value : {"FirstPerson", "firstperson", "Head", "head"}) {
        assert(menu::ChoiceIndex(item, value) == 4);
        assert(menu::Show(item, value) == "GTA first person");
        assert(menu::presets::Valid(item, value));
        assert(menu::Step(item, value, 1) == "Skate");
        assert(menu::Step(item, value, -1) == "Far");
    }
    assert(!menu::presets::Valid(item, "unknown"));
    std::cout << "camera logic tests passed\n";
}
