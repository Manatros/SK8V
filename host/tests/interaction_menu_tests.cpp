// Standalone: cl /std:c++20 /EHsc /W4 interaction_menu_tests.cpp
#include "../src/interaction_menu_logic.h"
#include <cassert>
#include <iostream>

int main() {
    using namespace interactionmenu;
    for (bool keyboard : {false, true}) {
        for (bool blocked : {false, true}) {
            assert(!Allowed(keyboard, true, false, blocked));
            assert(!Allowed(keyboard, false, true, blocked));
            assert(Allowed(keyboard, false, false, blocked) == (keyboard || !blocked));
            assert(SuppressBack(keyboard, blocked) == (!keyboard && blocked));
        }
    }
    // A controller chord/timer cannot swallow M after GTA switches to keyboard.
    assert(Allowed(true, false, false, true));
    assert(!SuppressBack(true, true));
    std::cout << "interaction menu tests passed\n";
}
