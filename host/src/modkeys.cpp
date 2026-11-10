#include "modkeys.h"

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <iterator>
#include <cwchar>
#include <set>
#include <vector>

#include <psapi.h>

#include "host_util.h"

namespace modkeys {
namespace {

using KeyFn = SHORT(WINAPI*)(int);

std::atomic<bool> g_block{false};
std::atomic<bool> g_blockMessages{false};
bool g_started = false;
void (*g_log)(const char*) = nullptr;
std::wstring g_gameDir;
KeyFn g_realAsync = nullptr, g_realKeyState = nullptr;
struct Slot {
    void** at;
    void* original;
};
std::vector<Slot> g_slots;  // every import entry pointed at the filter
std::set<HMODULE> g_seen;
HWND g_window = nullptr;
WNDPROC g_prevProc = nullptr;
DWORD g_nextScan = 0;

// Keys held when blocking ends stay hidden until let go: a mod watching for a
// fresh press would otherwise see one (the F10 that opened our menu, say).
std::atomic<bool> g_latched[256];

bool Quiet(int vk) {
    if (Hidden(vk, g_block.load())) return true;
    if (vk <= 0 || vk >= 256 || !g_latched[vk].load()) return false;
    if (g_realAsync(vk) & 0x8000) return true;
    g_latched[vk] = false;
    return false;
}

SHORT WINAPI FilteredAsync(int vk) { return Quiet(vk) ? 0 : g_realAsync(vk); }
SHORT WINAPI FilteredKeyState(int vk) { return Quiet(vk) ? 0 : g_realKeyState(vk); }

bool Patch(void** slot, void* to) {
    // Keep execute rights: in some modules the import table shares a page with code.
    MEMORY_BASIC_INFORMATION mbi{};
    if (!VirtualQuery(slot, &mbi, sizeof(mbi))) return false;
    const bool exec = (mbi.Protect & (PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY)) != 0;
    DWORD old = 0;
    if (!VirtualProtect(slot, sizeof(void*), exec ? PAGE_EXECUTE_READWRITE : PAGE_READWRITE, &old)) return false;
    *slot = to;
    VirtualProtect(slot, sizeof(void*), old, &old);
    return true;
}

// Points one module's USER32 imports of the two functions at the filters.
int PatchModule(HMODULE m) {
    auto* base = reinterpret_cast<std::uint8_t*>(m);
    const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(base);
    if (dos->e_magic != IMAGE_DOS_SIGNATURE) return 0;
    const auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS*>(base + dos->e_lfanew);
    if (nt->Signature != IMAGE_NT_SIGNATURE) return 0;
    const IMAGE_DATA_DIRECTORY dir = nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT];
    if (!dir.VirtualAddress || !dir.Size) return 0;
    int patched = 0;
    for (auto* d = reinterpret_cast<const IMAGE_IMPORT_DESCRIPTOR*>(base + dir.VirtualAddress); d->Name; ++d) {
        if (_stricmp(reinterpret_cast<const char*>(base + d->Name), "user32.dll") != 0) continue;
        for (auto* t = reinterpret_cast<IMAGE_THUNK_DATA*>(base + d->FirstThunk); t->u1.Function; ++t) {
            void** slot = reinterpret_cast<void**>(&t->u1.Function);
            void* const original = *slot;
            void* to = original == reinterpret_cast<void*>(g_realAsync)      ? reinterpret_cast<void*>(&FilteredAsync)
                       : original == reinterpret_cast<void*>(g_realKeyState) ? reinterpret_cast<void*>(&FilteredKeyState)
                                                                             : nullptr;
            if (to && Patch(slot, to)) g_slots.push_back({slot, original}), ++patched;
        }
    }
    return patched;
}

// The game folder's modules other than GTA itself and GTAxSkate's own.
void Scan() {
    HMODULE mods[1024];
    DWORD need = 0;
    if (!K32EnumProcessModules(GetCurrentProcess(), mods, sizeof(mods), &need)) return;
    HMODULE self = nullptr;
    GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                       reinterpret_cast<LPCWSTR>(&Scan), &self);
    const HMODULE exe = GetModuleHandleW(nullptr);
    const std::size_t n = std::min<std::size_t>(need / sizeof(HMODULE), std::size(mods));
    for (std::size_t i = 0; i < n; ++i) {
        const HMODULE m = mods[i];
        if (m == self || m == exe || g_seen.count(m)) continue;
        g_seen.insert(m);
        wchar_t path[MAX_PATH]{};
        if (!GetModuleFileNameW(m, path, MAX_PATH)) continue;
        if (_wcsnicmp(path, g_gameDir.c_str(), g_gameDir.size()) != 0) continue;  // Windows and other folders
        const wchar_t* name = std::wcsrchr(path, L'\\');
        name = name ? name + 1 : path;
        if (_wcsnicmp(name, L"SkateV", 6) == 0) continue;  // GTAxSkate's runtime reads its own keys
        const int count = PatchModule(m);
        if (count) util::Logf(g_log, "mod keys: %ls: %d key import(s) quiet while the replay or the menu is open", name, count);
    }
}

LRESULT CALLBACK Proc(HWND h, UINT msg, WPARAM w, LPARAM l) {
    if (g_blockMessages.load() && (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN || msg == WM_CHAR || msg == WM_SYSCHAR)) {
        const bool alt = (l & (1 << 29)) != 0;
        // Esc, Alt+Tab/F4/Enter, and GTAxSkate's own menu keys (F10, '/') always pass.
        const bool system = w == VK_ESCAPE || w == VK_F10 || w == VK_OEM_2 || (alt && (w == VK_TAB || w == VK_F4 || w == VK_RETURN));
        if (!system) return 0;  // held back: no other mod sees it (key-ups always pass)
    }
    return CallWindowProcW(g_prevProc, h, msg, w, l);
}

void HookWindow() {
    if (g_window) return;
    HWND w = FindWindowW(L"grcWindow", nullptr);
    if (!w) return;
    DWORD pid = 0;
    GetWindowThreadProcessId(w, &pid);
    if (pid != GetCurrentProcessId()) return;
    // The previous procedure is known before ours can be called.
    g_prevProc = reinterpret_cast<WNDPROC>(GetWindowLongPtrW(w, GWLP_WNDPROC));
    if (g_prevProc && SetWindowLongPtrW(w, GWLP_WNDPROC, reinterpret_cast<LONG_PTR>(&Proc))) {
        g_window = w;
        // GTA's window calls into this module from now on: it never unloads before the process ends.
        HMODULE pin = nullptr;
        GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN, reinterpret_cast<LPCWSTR>(&Proc), &pin);
        util::Logf(g_log, "mod keys: GTA's window messages filtered while the replay or the menu is open");
    }
}

}  // namespace

bool Hidden(int vk, bool blocking) {
    if (!blocking) return false;
    const bool mouse = vk == VK_LBUTTON || vk == VK_RBUTTON || vk == VK_MBUTTON || vk == VK_XBUTTON1 || vk == VK_XBUTTON2;
    return !mouse && vk > 0 && vk < 256;
}

void Start(const std::wstring& gameDir, void (*log)(const char*)) {
    if (g_started) return;
    g_log = log;
    g_gameDir = gameDir;
    const HMODULE user32 = GetModuleHandleW(L"user32.dll");
    g_realAsync = reinterpret_cast<KeyFn>(GetProcAddress(user32, "GetAsyncKeyState"));
    g_realKeyState = reinterpret_cast<KeyFn>(GetProcAddress(user32, "GetKeyState"));
    if (!g_realAsync || !g_realKeyState) return;
    g_started = true;
    Scan();
    HookWindow();
}

void Tick(bool keys, bool messages) {
    if (!g_started) return;
    if (g_block.load() && !keys)  // blocking ends: what is held stays hidden until let go
        for (int vk = 1; vk < 256; ++vk)
            if (Hidden(vk, true) && (g_realAsync(vk) & 0x8000)) g_latched[vk] = true;
    g_block = keys;
    g_blockMessages = messages;
    const DWORD now = GetTickCount();
    if (static_cast<int>(now - g_nextScan) >= 0) {  // scripts and ASIs that load later
        g_nextScan = now + 5000;
        Scan();
        HookWindow();
    }
}

void Stop() {
    g_block = false;
    g_blockMessages = false;
    for (const Slot& s : g_slots) Patch(s.at, s.original);
    g_slots.clear();
    // Only when no one subclassed after us; else our procedure stays and passes everything.
    if (g_window && reinterpret_cast<WNDPROC>(GetWindowLongPtrW(g_window, GWLP_WNDPROC)) == &Proc)
        SetWindowLongPtrW(g_window, GWLP_WNDPROC, reinterpret_cast<LONG_PTR>(g_prevProc));
    g_started = false;
}

}  // namespace modkeys
