#pragma once
// CPU time of every thread in GTA's process since the last call (VerboseLog's
// 10 s report): which threads skating makes busier, GTA's own included.
#include <string>

namespace threadcensus {
// "<total> ms/s over <n> threads; <label> <ms/s>, ..." for the `top` busiest;
// a label is the thread's description, else its start module+offset.
std::string Report(double seconds, int top);
// Drop an old baseline when diagnostics are disabled or toggled.
void Reset();
// Names the calling thread for the report (no-op before Windows 10 1607).
void Name(const wchar_t* name);
} // namespace threadcensus
