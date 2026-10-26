#include "../src/prop_collision.cpp"
#include <cassert>
#include <sstream>
#include <iostream>
int main() {
    PropCollisionIndex index;
    SvDynamicBody b{};
    b.model_hash = 0x123;
    using Route = PropCollisionIndex::Route;
    for (float size : {0.1f, 1.0f, 10.0f}) {
        b.fallback.half_extents = {size, size, size};
        assert(index.Select(b, true, false, false) == Route::None);
        assert(index.Select(b, true, true, false) == Route::Box);
        assert(index.Select(b, true, true, true) == Route::None);
        assert(index.Select(b, false, false, false) == Route::Box);
    }
    std::istringstream models("123\n");
    index.ReadModels(models);
    assert(index.Select(b, true, false, false) == Route::Exact);
    std::cout << "prop collision tests passed\n";
}
