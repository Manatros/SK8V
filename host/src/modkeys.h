#pragma once
// Other mods' keys stay quiet while the replay or the F10 menu is open
// (Manatros 2026-11-09: "disable the keys for other mods when in certain
// modes, like in the replay mode ... so you dont accidentally spawn a car like
// I did trying to use the editor"). Third-party mods are never edited; key
// bindings are not moved (custom bindings exist for that). Instead, while
// blocking:
//  - ASI mods in the game folder (Simple Trainer, Menyoo, ...) see no key
//    pressed: their imports of GetAsyncKeyState / GetKeyState are pointed at a
//    filter (GTA5.exe, GTAxSkate's own modules and Windows' DLLs are untouched);
//  - key-down and character messages to GTA's window are held back, so
//    ScriptHookV's keyboard handlers (ScriptHookVDotNet scripts such as Better
//    Chases+) get none; key-ups, Esc, Alt+Tab and Alt+F4 always pass.
// GTAxSkate reads its own keys with GetAsyncKeyState from its own module, so
// the replay's keys keep working. INI BlockModKeys=0 switches it off.
#include <windows.h>
#include <string>

namespace modkeys {
// Script thread, once: the game folder (trailing '\').
void Start(const std::wstring& gameDir, void (*log)(const char*));
// Script thread, every frame: `keys` hides key states from other ASI mods,
// `messages` also holds back key-down messages (ScriptHookV handlers). The
// replay uses both; the F10 menu only `keys` (it closes with a key message).
// New modules are patched every few seconds.
void Tick(bool keys, bool messages);
// DLL detach: everything put back.
void Stop();
// The filter's decision (tests): a keyboard key is hidden while blocking; mouse buttons never.
bool Hidden(int vk, bool blocking);
}  // namespace modkeys
