#include "board_parts.h"
#include "board_native.h"
#include "board_composition_key.h"
#include "host_util.h"
#include "natives.h"
#include "pose_math.h"
#include "streaming.h"
#include <windows.h>
#include <cstdio>
#include <filesystem>
#include <map>
#include <fstream>
#include <iterator>

namespace boardparts {
namespace {
using namespace gta;

struct Model {
    std::uint32_t ydr = ~0u, ytyp = ~0u;
    std::string ydrPath, ytypPath;
    bool typed = false, failed = false, loaded = false;
    ULONGLONG requested = 0;
};
struct Shown {
    std::string name;
    int object = 0;
    boardnative::PoseCache cache;
};

void (*g_log)(const char*) = nullptr;
std::filesystem::path g_root;
std::filesystem::path g_compositionRoot;
std::string g_compositionSession;
bool g_compose = false;
std::map<std::filesystem::path, std::pair<std::uintmax_t, std::filesystem::file_time_type>> g_compositionInputs;
HANDLE g_compositionWorker = nullptr;
ULONGLONG g_compositionWorkerAttempt = 0;
bool g_started = false;
std::map<std::string, Model> g_models;  // registrations persist for the process
// A set of part objects making one board: the ridden one and the store's preview.
struct Set {
    std::vector<std::string> models;
    std::vector<Shown> shown;
    bool announced = false;
    int interior = 0;            // the room the objects are kept in (the player's)
    std::uint32_t room = 0;
    ULONGLONG waitingSince = 0;  // not all parts loaded yet (for the log)
    bool notRigidLogged = false;
    std::string composition;
    ULONGLONG compositionChecked = 0;
    bool compositionPublished = false;
    bool compositionErrorLogged = false;
};
Set g_ride, g_preview;
Set g_display[8];
Set g_crew[4];
Set g_hand;

bool ShownElsewhere(const std::string& name, const Set& except) {
    auto in = [&](const Set& set) {
        if (&set == &except) return false;
        for (const Shown& s : set.shown)
            if (s.name == name) return true;
        return false;
    };
    if (in(g_ride) || in(g_preview)) return true;
    for (const Set& set : g_display)
        if (in(set)) return true;
    for (const Set& set : g_crew)
        if (in(set)) return true;
    return in(g_hand);
}

void Log(const char* s) {
    if (g_log) g_log(s);
}

void Logf(const char* format, const char* a, const char* b = "") {
    char line[320];
    std::snprintf(line, sizeof(line), format, a, b);
    Log(line);
}

// Registers the part's loose YDR/YTYP once and asks for its model; true when loaded.
// GTA's loose-file registration and type load, guarded: in a long store visit
// (hundreds of parts looked at) the registrar wrote through a null pointer
// inside GTA and Script Hook V stopped the whole script (10-11). A fault
// here fails that one part instead.
bool SafeRegister(streaming::Register reg, std::uint32_t* out, const char* path, const char* name) {
    __try {
        reg(out, path, true, name, false, false);
        return true;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        return false;
    }
}

bool SafeTypeLoad(streaming::TypeLoad load, const char* name) {
    __try {
        load(name);
        return true;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        return false;
    }
}

// New part models registered this game session; past this the native board
// shows (GTA's streaming has room for a few hundred loose files).
constexpr int kMaxRegistered = 240;
int g_registered = 0;

bool Ready(const std::string& name) {
    Model& m = g_models[name];
    if (m.failed) return false;
    const std::uint32_t hash = util::Joaat(name);
    if (!m.typed) {
        if (!streaming::Ready()) return false;
        if (Call<bool>(IS_MODEL_IN_CDIMAGE, hash)) {
            Logf("board parts: %s: model name already occupied", name.c_str());
            m.failed = true;
            return false;
        }
        const auto& root = name.rfind("skatev_c_", 0) == 0 ? g_compositionRoot : g_root;
        m.ydrPath = streaming::NarrowAnsi(root / (name + ".ydr"));
        m.ytypPath = streaming::NarrowAnsi(root / (name + ".ytyp"));
        if (m.ydrPath.empty() || m.ytypPath.empty() || !std::filesystem::is_regular_file(root / (name + ".ydr"))) {
            Logf("board parts: %s: no model file", name.c_str());
            m.failed = true;
            return false;
        }
        if (g_registered >= kMaxRegistered) {
            static bool s_said = false;
            if (!s_said) s_said = true, Logf("board parts: %s: %s part models this session, no more (restart GTA for others)", name.c_str(), std::to_string(g_registered).c_str());
            m.failed = true;
            return false;
        }
        const auto reg = streaming::Registrar();
        const std::string ydrName = name + ".ydr", ytypName = name + ".ytyp";
        if (!SafeRegister(reg, &m.ydr, m.ydrPath.c_str(), ydrName.c_str()) ||
            !SafeRegister(reg, &m.ytyp, m.ytypPath.c_str(), ytypName.c_str())) {
            Logf("board parts: %s: GTA faulted registering it (%s registered)", name.c_str(), std::to_string(g_registered).c_str());
            m.failed = true;
            g_registered = kMaxRegistered;  // GTA's streaming is unwell: no more this session
            return false;
        }
        ++g_registered;
        if (m.ydr == ~0u || m.ytyp == ~0u) {
            Logf("board parts: %s: registration failed", name.c_str());
            m.failed = true;
            return false;
        }
        if (!SafeTypeLoad(streaming::TypeLoader(), ytypName.c_str())) {
            Logf("board parts: %s: GTA faulted loading its type", name.c_str());
            m.failed = true;
            g_registered = kMaxRegistered;
            return false;
        }
        m.typed = true;
        m.requested = GetTickCount64();
    }
    if (Call<bool>(HAS_MODEL_LOADED, hash)) {
        m.loaded = true;
        return true;
    }
    // Let go earlier (RemoveAll) and wanted again: the wait starts now.
    if (m.loaded) m.loaded = false, m.requested = GetTickCount64();
    if (Call<bool>(IS_MODEL_VALID, hash)) Call<void>(REQUEST_MODEL, hash);
    if (GetTickCount64() - m.requested > 15000) {
        Logf("board parts: %s: model load timed out", name.c_str());
        m.failed = true;
    }
    return false;
}

void Remove(Shown& s) {
    if (s.object && Call<bool>(DOES_ENTITY_EXIST, s.object)) Call<void>(DELETE_OBJECT, &s.object);
    s.object = 0;
}

bool ShownElsewhere(const std::string& name, const Set& except);

// Removes the set's objects; a model no other set shows is let go, so
// browsing the store does not keep every part it showed in memory.
void RemoveAll(Set& set) {
    for (Shown& s : set.shown) Remove(s);
    for (const Shown& s : set.shown)
        if (!ShownElsewhere(s.name, set)) Call<void>(SET_MODEL_AS_NO_LONGER_NEEDED, util::Joaat(s.name));
    set.shown.clear();
}

int Create(const std::string& name, const SvBoardPose& pose) {
    const int object = Call<int>(CREATE_OBJECT_NO_OFFSET, util::Joaat(name), pose.entity[12], pose.entity[13],
                                 pose.entity[14], false, false, false, 0);
    if (!object) return 0;
    // Presentation only, as the native board.
    Call<void>(SET_ENTITY_VISIBLE, object, false, false);
    Call<void>(SET_ENTITY_AS_MISSION_ENTITY, object, true, false);
    Call<void>(FREEZE_ENTITY_POSITION, object, true);
    Call<void>(SET_ENTITY_HAS_GRAVITY, object, false);
    Call<void>(SET_ENTITY_DYNAMIC, object, false);
    Call<void>(SET_ENTITY_COLLISION, object, false, false);
    Call<void>(SET_ENTITY_COMPLETELY_DISABLE_COLLISION, object, false, false);
    return object;
}

} // namespace

bool Start(const std::wstring& dataRoot, void (*log)(const char*)) {
    g_log = log;
    g_root = std::filesystem::path(dataRoot) / L"private" / L"board-parts";
    g_compositionRoot = std::filesystem::path(dataRoot) / L"private" / L"board-composed";
    std::error_code ec;
    g_compose = std::filesystem::is_regular_file(g_compositionRoot / "enabled", ec);
    // Unique per process/start: no already registered drawable is replaced and
    // stale output from previous inputs/sessions can never be selected.
    char session[17];
    std::snprintf(session, sizeof(session), "%llx", static_cast<unsigned long long>(GetTickCount64() ^ (static_cast<ULONGLONG>(GetCurrentProcessId()) << 32)));
    g_compositionSession = session;
    g_compositionInputs.clear();
    if (g_compose) {
        std::ofstream active(g_compositionRoot / "active-session", std::ios::binary | std::ios::trunc);
        active << g_compositionSession;
        active.close();
        if (!active) g_compose = false;
    }
    if (g_compose) Log("board composition: enabled; exact shop selections queued for external preparation, animation writer unchanged");
    g_started = std::filesystem::is_directory(g_root);
    if (!g_started) Log("board parts: none built (private\\board-parts missing); the native board shows");
    // The raw registration is checked against 3889's code, as the native board's.
    if (g_started && !streaming::Guards()) {
        Log("board parts: 3889 streaming function guards differ; the native board shows");
        g_started = false;
    }
    return g_started;
}

void SetLoadout(const std::vector<std::string>& models) {
    if (models == g_ride.models) return;
    g_ride.models = models;
    g_ride.announced = false;
    RemoveAll(g_ride);
}

const std::vector<std::string>& Loadout() {
    return g_ride.models;
}

bool Available(const std::string& model) {
    return g_started && std::filesystem::is_regular_file(g_root / (model + ".ydr"));
}

namespace {
void EnsureCompositionWorker() {
    if (g_compositionWorker) {
        DWORD code = 0;
        if (GetExitCodeProcess(g_compositionWorker, &code) && code == STILL_ACTIVE) return;
        CloseHandle(g_compositionWorker);
        g_compositionWorker = nullptr;
    }
    const auto now = GetTickCount64();
    if (g_compositionWorkerAttempt && now - g_compositionWorkerAttempt < 10000) return;
    g_compositionWorkerAttempt = now;
    const auto ini = g_compositionRoot / "worker.ini";
    wchar_t pythonPath[32768]{}, ragePath[32768]{};
    GetPrivateProfileStringW(L"Worker", L"Python", L"", pythonPath, 32768, ini.c_str());
    GetPrivateProfileStringW(L"Worker", L"Rage", L"", ragePath, 32768, ini.c_str());
    const std::filesystem::path python(pythonPath), rage(ragePath);
    const auto script = g_compositionRoot / "board-composition-worker.py";
    const auto composer = g_compositionRoot / "prepare-native-custom-board.py";
    std::error_code ec;
    if (!std::filesystem::is_regular_file(python, ec) || !std::filesystem::is_regular_file(rage, ec) ||
        !std::filesystem::is_regular_file(script, ec) || !std::filesystem::is_regular_file(composer, ec)) {
        Log("board composition: worker setup missing; selected parts retained");
        return;
    }
    auto quote = [](const std::filesystem::path& path) { return L"\"" + path.wstring() + L"\""; };
    std::wstring command = quote(python) + L" " + quote(script) + L" --watch --root " + quote(g_compositionRoot) +
        L" --native " + quote(g_root.parent_path() / "board-native" / "skatev_board.ydr") +
        L" --parts " + quote(g_root) + L" --rage " + quote(rage);
    STARTUPINFOW startup{};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION process{};
    if (!CreateProcessW(python.c_str(), command.data(), nullptr, nullptr, false, CREATE_NO_WINDOW, nullptr, nullptr, &startup, &process)) {
        Log("board composition: worker launch failed; selected parts retained");
        return;
    }
    CloseHandle(process.hThread);
    g_compositionWorker = process.hProcess;
    Log("board composition: preparation worker started without a window");
}

// Nonblocking cache lookup only. The separate worker owns all exporting and
// validation. A readiness marker is published last, for the exact vector.
std::string Composed(Set& set) {
    if (!g_compose) return {};
    const auto payload = boardcomposition::Payload(set.models);
    if (payload.empty()) return {};
    const auto name = "skatev_c_" + g_compositionSession + "_" + boardcomposition::Key(payload);
    if (set.composition != name) {
        set.composition = name;
        set.compositionChecked = 0;
        set.compositionPublished = false;
        set.compositionErrorLogged = false;
    }
    const auto now = GetTickCount64();
    if (set.compositionChecked && now - set.compositionChecked < 500) return set.compositionPublished ? name : std::string{};
    set.compositionChecked = now;
    std::error_code ec;
    std::vector<std::filesystem::path> inputs{g_root.parent_path() / "board-native" / "skatev_board.ydr"};
    for (const auto& model : set.models) inputs.push_back(g_root / (model + ".ydr"));
    for (const auto& path : inputs) {
        const auto size = std::filesystem::file_size(path, ec);
        if (ec) return {};
        const auto modified = std::filesystem::last_write_time(path, ec);
        if (ec) return {};
        const auto stamp = std::make_pair(size, modified);
        const auto prior = g_compositionInputs.emplace(path, stamp);
        if (!prior.second && prior.first->second != stamp) {
            g_compose = false;
            Log("board composition: source assets changed during this host session; preparation disabled, restart GTA after asset changes");
            return {};
        }
    }
    if (set.compositionPublished) return name;
    const auto ready = g_compositionRoot / (name + ".ready");
    if (std::filesystem::is_regular_file(ready, ec) && std::filesystem::file_size(ready, ec) <= 2048 && !ec) {
        std::ifstream input(ready, std::ios::binary);
        const std::string contents((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
        if (contents == payload && std::filesystem::is_regular_file(g_compositionRoot / (name + ".ydr"), ec) &&
            std::filesystem::is_regular_file(g_compositionRoot / (name + ".ytyp"), ec)) {
            set.compositionPublished = true;
            return name;
        }
    }
    if (!set.compositionErrorLogged && std::filesystem::is_regular_file(g_compositionRoot / (name + ".error"), ec)) {
        set.compositionErrorLogged = true;
        Logf("board composition: preparation failed for %s; details in matching .error file; selected parts retained%s", name.c_str());
    }
    const auto request = g_compositionRoot / (name + ".request");
    if (!std::filesystem::exists(request, ec)) {
        const auto temporary = g_compositionRoot / (name + ".tmp");
        std::ofstream output(temporary, std::ios::binary | std::ios::trunc);
        output << payload;
        output.close();
        if (output) {
            std::filesystem::rename(temporary, request, ec);
            if (!ec) Logf("board composition: queued %s; showing existing parts until prepared%s", name.c_str());
        }
    }
    if (!set.compositionErrorLogged) EnsureCompositionWorker();
    return {};
}

// Shows `set` posed by `pose`; true when every part is out and posed.
bool Show(Set& set, const SvBoardPose& pose) {
    if (pose.size != sizeof(SvBoardPose) || pose.bone_count != 7) return false;
    for (const auto& bone : pose.world)
        for (float f : bone)
            if (!std::isfinite(f)) return false;
    posemath::M entity;
    std::memcpy(&entity, pose.entity, sizeof(entity));
    posemath::Q q;
    if (!posemath::RigidQuaternion(entity, q)) {
        if (!set.notRigidLogged) set.notRigidLogged = true, Log("board parts: a pose's entity matrix is not a rotation; parts not shown");
        return false;
    }
    bool all = true;
    std::string missing;
    const auto combined = Composed(set);
    // Only switch when the verified composed model is loaded. Pending/failed
    // preparation keeps the selected parts, including paint and griptape.
    const auto models = !combined.empty() && Ready(combined) ? std::vector<std::string>{combined} : set.models;
    for (const std::string& name : models)
        if (!Ready(name)) all = false, missing = name;
    if (!all) {  // the native board shows until every part is in
        const ULONGLONG now = GetTickCount64();
        if (!set.waitingSince) set.waitingSince = now;
        else if (now - set.waitingSince > 3000) {
            Logf("board parts: still waiting for %s%s", missing.c_str(), &set == &g_preview ? " (store preview)" : "");
            set.waitingSince = now + 60000;
        }
        return false;
    }
    set.waitingSince = 0;
    if (!set.shown.empty() && (set.shown.size() != models.size() || set.shown.front().name != models.front())) {
        RemoveAll(set);
        set.announced = false;
    }
    if (set.shown.empty()) {
        for (const std::string& name : models) {
            Shown s;
            s.name = name;
            s.object = Create(name, pose);
            if (!s.object) {
                Logf("board parts: %s: CREATE_OBJECT_NO_OFFSET failed", name.c_str());
                g_models[name].failed = true;
                RemoveAll(set);
                return false;
            }
            set.shown.push_back(s);
        }
        if (!set.announced)
            Logf(&set == &g_ride ? "board parts: riding %s%s" : "board parts: store preview of %s%s", models.front().c_str(),
                 models.size() > 1 ? " and the rest of the setup" : " (single drawable)");
        set.interior = -1;
        set.announced = true;
    }
    bool posed = true;
    for (Shown& s : set.shown) {
        if (!Call<bool>(DOES_ENTITY_EXIST, s.object)) {
            RemoveAll(set);
            return false;
        }
        Call<void>(SET_ENTITY_COORDS_NO_OFFSET, s.object, pose.entity[12], pose.entity[13], pose.entity[14], false, false, false);
        Call<void>(SET_ENTITY_QUATERNION, s.object, q.x, q.y, q.z, q.w);
        posed = boardnative::PoseObject(s.object, pose, s.cache, g_log) && posed;
    }
    // Inside an interior GTA draws only what is in its rooms: the parts go
    // into the player's room (a store), and out again outside.
    // While skating SK8V moves the held (frozen) player by script and GTA may
    // not update their interior: in a tunnel the board was lit as outside.
    // Then the interior at the board itself, and the room the camera is in.
    const int ped = Call<int>(PLAYER_PED_ID);
    int interior = Call<int>(GET_INTERIOR_FROM_ENTITY, ped);
    std::uint32_t room = interior ? Call<std::uint32_t>(GET_ROOM_KEY_FROM_ENTITY, ped) : 0;
    if (!interior) {
        interior = Call<int>(GET_INTERIOR_AT_COORDS, pose.entity[12], pose.entity[13], pose.entity[14]);
        room = interior ? Call<std::uint32_t>(GET_ROOM_KEY_FOR_GAME_VIEWPORT) : 0;
        if (room == 0xFFFFFFFFu) room = 0;  // the camera in no room yet
    }
    if (interior != set.interior || room != set.room) {
        for (Shown& s : set.shown) {
            if (interior && room) Call<void>(FORCE_ROOM_FOR_ENTITY, s.object, interior, room);
            else Call<void>(CLEAR_ROOM_FOR_ENTITY, s.object);
        }
        if (&set == &g_ride) {
            static int s_logs = 0;
            if (s_logs++ < 40)
                Logf("board parts: riding in interior %s, room %s", std::to_string(interior).c_str(), std::to_string(room).c_str());
        }
        set.interior = interior, set.room = room;
    }
    for (Shown& s : set.shown) Call<void>(SET_ENTITY_VISIBLE, s.object, posed, false);
    return posed;
}
} // namespace

bool Tick(bool skating, const SvBoardPose& pose) {
    if (!g_started || g_ride.models.empty() || !skating) {
        RemoveAll(g_ride);
        return false;
    }
    return Show(g_ride, pose);
}

bool Preview(const std::vector<std::string>& models, const SvBoardPose& pose) {
    if (!g_started || models.empty()) {
        RemoveAll(g_preview);
        g_preview.models.clear();
        return false;
    }
    if (models != g_preview.models) {
        RemoveAll(g_preview);
        g_preview.models = models;
    }
    return Show(g_preview, pose);
}

bool Display(int index, const std::vector<std::string>& models, const SvBoardPose& pose) {
    if (!g_started || index < 0 || index >= 8) return false;
    Set& set = g_display[index];
    if (models != set.models) {
        RemoveAll(set);
        set.models = models;
        set.announced = true;  // boards on show are logged by the shop
    }
    return !models.empty() && Show(set, pose);
}

void ClearDisplay() {
    for (Set& set : g_display) {
        RemoveAll(set);
        set.models.clear();
    }
}

bool Hand(const std::vector<std::string>& models, const SvBoardPose& pose) {
    if (!g_started) return false;
    if (models != g_hand.models) {
        RemoveAll(g_hand);
        g_hand.models = models;
        g_hand.announced = true;
    }
    return !models.empty() && Show(g_hand, pose);
}

bool Crew(int slot, const std::vector<std::string>& models, const SvBoardPose& pose) {
    if (!g_started || slot < 0 || slot >= 4) return false;
    Set& set = g_crew[slot];
    if (models != set.models) {
        RemoveAll(set);
        set.models = models;
        set.announced = true;
    }
    return !models.empty() && Show(set, pose);
}

std::vector<int> Entities() {
    std::vector<int> out;
    for (const Set* set : {&g_ride, &g_preview})
        for (const Shown& s : set->shown)
            if (s.object) out.push_back(s.object);
    for (const Shown& s : g_hand.shown)
        if (s.object) out.push_back(s.object);
    for (const Set* group : {g_display, g_crew})
        for (std::size_t i = 0; i < (group == g_display ? 8u : 4u); ++i)
            for (const Shown& s : group[i].shown)
                if (s.object) out.push_back(s.object);
    return out;
}

void Stop() {
    RemoveAll(g_ride);
}

} // namespace boardparts
