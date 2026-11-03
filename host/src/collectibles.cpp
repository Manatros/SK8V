#include "collectibles.h"

#include <windows.h>
#include <shlobj.h>

#include <cmath>
#include <cstdio>
#include <deque>
#include <filesystem>
#include <fstream>
#include <algorithm>
#include <map>
#include <set>
#include <sstream>
#include <vector>

#include "board_parts.h"
#include "board_shop.h"
#include "collectibles_logic.h"
#include "hud_overlay.h"
#include "natives.h"

namespace collectibles {
namespace {
using namespace gta;

constexpr float kDrawRange = 300.0f;   // markers drawn this far (hunting while skating)
constexpr float kFindRange = 40.0f;    // found with a clear sight line from this close
constexpr float kReach = 1.5f;         // earned when the skater's root passes this close
constexpr float kAirReach = 1.0f;      // ... an air collectible: closer, and only in the air
constexpr float kRootBelow = 0.6f;     // the skater's root (hip) is below the marker centre
constexpr DWORD kSightEvery = 1000;    // ms between sight-line probes
constexpr int kBlipSprite = 66;        // question mark
constexpr int kPartColour = 5;         // yellow
constexpr int kSongColour = 27;        // purple
constexpr int kNoPartsLeftCash = 500;
// The burst where a collectible is earned (GTA's own effect, requested
// again whenever it is not loaded).
constexpr const char* kBurstAsset = "scr_rcbarry2";
constexpr const char* kBurstEffect = "scr_clown_appears";
constexpr float kBurstScale = 0.4f;
constexpr size_t kPopupChars = 40;  // skate_hud::central::MAX_CHARS
// Design 2026-11-02-collectibles-v2: up close a collectible shows its item
// (the deck, or a music prop for a song) instead of the question mark.
constexpr float kItemRange = 40.0f;
constexpr int kItemSlots = 4;
constexpr DWORD kSongRetryEvery = 30000;  // ms between retries of song renames in use
// The music prop: the first of these in the game (a tape, a record, a CD, a boombox).
constexpr const char* kMusicModels[] = {"prop_cs_cassette", "prop_cassette_01", "prop_vinyl_record_01", "ba_prop_battle_vinyl_01",
                                        "prop_cd_case_01", "prop_cs_cd_case", "prop_boombox_01", "prop_ghettoblast_01",
                                        "prop_portable_hifi_01"};

util::LogFn g_log = nullptr;
std::wstring g_placesPath, g_progressPath, g_songsDir;
std::vector<Item> g_items;
Progress g_progress;
std::map<std::string, int> g_blips;
std::deque<std::string> g_notices;
std::string g_notice;
bool g_wasEnabled = true;
bool g_popups = false;  // Skate's HUD shows: centre popups instead of notices
bool g_hadSkater = false;
float g_prev[3] = {};
DWORD g_nextSight = 0;
int g_sel = -1;
bool g_songFailLogged = false;
bool g_faulty = false;  // collectibles.txt exists but could not be read: change nothing
std::map<std::string, Assigned> g_assigned;  // each collectible's fixed item
bool g_partsAssigned = false;                // the runtime listed its secrets
std::set<std::string> g_unlockedKeys;        // shop keys earned so far
bool g_songsPending = false;                 // a song rename failed: retried
DWORD g_nextSongRetry = 0;
std::uint32_t g_musicModel = 0;  // 0: not chosen yet
bool g_musicChecked = false;
int g_musicObj[kItemSlots] = {};
std::map<std::string, DWORD> g_itemWaiting;  // a deck not shown yet: since when (logged once after 15 s)
std::set<std::string> g_itemFailLogged;
std::set<int> g_tracksOnDisk;  // song files present (either form), from the last reconcile

void Log(const std::string& text) {
    if (g_log) g_log(("collectibles: " + text).c_str());
}

// The file's text; false when it exists but cannot be read.
bool Read(const std::wstring& path, std::string& text) {
    text.clear();
    if (GetFileAttributesW(path.c_str()) == INVALID_FILE_ATTRIBUTES && GetLastError() == ERROR_FILE_NOT_FOUND) return true;
    std::ifstream in(path, std::ios::binary);
    if (!in) return false;
    std::ostringstream out;
    out << in.rdbuf();
    text = out.str();
    return !in.bad();
}

// Through a temporary file that replaces the original (never half a file).
bool Save() {
    if (g_faulty) return false;
    const std::wstring tmp = g_progressPath + L".tmp";
    bool written = false;
    {
        std::ofstream out(tmp, std::ios::binary | std::ios::trunc);
        if (!out) return false;
        const std::string text = FormatProgress(g_progress);
        out.write(text.data(), static_cast<std::streamsize>(text.size()));
        written = static_cast<bool>(out.flush());
    }
    if (written && MoveFileExW(tmp.c_str(), g_progressPath.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) return true;
    DeleteFileW(tmp.c_str());
    Log("could not write GTAxSkate\\collectibles3.txt; progress of this session is not kept");
    return false;
}

std::wstring SongsDir() {
    wchar_t docs[MAX_PATH]{};
    if (FAILED(SHGetFolderPathW(nullptr, CSIDL_PERSONAL, nullptr, SHGFP_TYPE_CURRENT, docs))) return {};
    return std::wstring(docs) + L"\\Rockstar Games\\GTA V\\User Music\\Skate 3";
}

std::vector<SongFile> SongFiles() {
    std::vector<SongFile> files;
    std::error_code ec;
    if (g_songsDir.empty() || !std::filesystem::is_directory(g_songsDir, ec)) return files;
    for (const auto& e : std::filesystem::directory_iterator(g_songsDir, ec))
        if (e.is_regular_file(ec))
            files.push_back({reinterpret_cast<const char*>(e.path().filename().u8string().c_str()), e.file_size(ec)});
    return files;
}

// Starters and earned songs playable, the others locked (all playable when off).
void Reconcile(bool enabled) {
    if (g_faulty) return;
    const auto files = SongFiles();
    g_tracksOnDisk.clear();
    for (const auto& f : files)
        if (const int t = TrackOf(f.name)) g_tracksOnDisk.insert(t);
    const auto ops = SongPlan(files, g_progress, enabled);
    int done = 0, failed = 0;
    for (const FileOp& op : ops) {
        const std::filesystem::path from = std::filesystem::path(g_songsDir) / std::filesystem::u8path(op.from);
        const bool ok = op.type == FileOp::Delete
                            ? DeleteFileW(from.c_str()) != 0
                            : MoveFileExW(from.c_str(), (std::filesystem::path(g_songsDir) / std::filesystem::u8path(op.to)).c_str(), 0) != 0;
        ok ? ++done : ++failed;
    }
    g_songsPending = failed > 0;
    g_nextSongRetry = GetTickCount() + kSongRetryEvery;
    if (done) Log(std::to_string(done) + " song file(s) " + (enabled ? "matched to the songs found" : "given back (Collectibles off)"));
    if (failed && !g_songFailLogged) {
        Log(std::to_string(failed) + " song file(s) in use or not writable; tried again later");
        g_songFailLogged = true;
    }
}

void Notice(const std::string& text) { g_notices.push_back(text); }

void Blip(const Item& it) {
    if (g_blips.count(it.id)) return;
    const int b = Call<int>(ADD_BLIP_FOR_COORD, it.x, it.y, it.z);
    if (!b) return;
    Call<void>(SET_BLIP_SPRITE, b, kBlipSprite);
    Call<void>(SET_BLIP_COLOUR, b, GivesParts(it.kind) ? kPartColour : kSongColour);
    Call<void>(SET_BLIP_AS_SHORT_RANGE, b, TRUE);
    Call<void>(BEGIN_TEXT_COMMAND_SET_BLIP_NAME, "STRING");
    Call<void>(ADD_TEXT_COMPONENT_SUBSTRING_PLAYER_NAME, it.kind == Kind::Board ? "Hidden board" : it.kind == Kind::Part ? "Hidden board part" : "Hidden song");
    Call<void>(END_TEXT_COMMAND_SET_BLIP_NAME, b);
    g_blips[it.id] = b;
}

void Unblip(const std::string& id) {
    const auto it = g_blips.find(id);
    if (it == g_blips.end()) return;
    int handle = it->second;
    if (Call<BOOL>(DOES_BLIP_EXIST, handle)) Call<void>(REMOVE_BLIP, &handle);
    g_blips.erase(it);
}

void Draw(const Item& it, float distance, bool found, bool mark) {
    const float s = MarkerScale(distance);
    const bool givesParts = GivesParts(it.kind);
    const int r = givesParts ? 255 : 170, g = givesParts ? 200 : 70, b = givesParts ? 0 : 255;
    // Type 32: the question mark, bobbing and turning (not where the item itself shows).
    if (mark)
        Call<void>(DRAW_MARKER, 32, it.x, it.y, it.z, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, s, s, s, r, g, b, 220, TRUE, FALSE, 2, TRUE,
                   static_cast<const char*>(nullptr), static_cast<const char*>(nullptr), FALSE);
    else  // the item's soft glow in the marker's colour
        Call<void>(DRAW_LIGHT_WITH_RANGE, it.x, it.y, it.z, r, g, b, 2.5f, 4.0f);
    // Found ones: a thin beam, to see from far away where to go.
    if (found)
        Call<void>(DRAW_MARKER, 1, it.x, it.y, it.z - 1.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.3f, 0.3f, 30.0f, r, g, b, 90, FALSE,
                   FALSE, 2, FALSE, static_cast<const char*>(nullptr), static_cast<const char*>(nullptr), FALSE);
}

// A clear sight line from the camera: nothing hit, or a hit at the marker's rail.
bool InSight(const Vector3& eye, const Item& it, int ped) {
    const int test = Call<int>(START_EXPENSIVE_SYNCHRONOUS_SHAPE_TEST_LOS_PROBE, eye.x, eye.y, eye.z, it.x, it.y, it.z, 1 | 16, ped, 7);
    BOOL any = 0;
    Vector3 end{}, normal{};
    Entity entity = 0;
    if (Call<int>(GET_SHAPE_TEST_RESULT, test, &any, &end, &normal, &entity) != 2) return false;
    if (!any) return true;
    const float dx = end.x - it.x, dy = end.y - it.y, dz = end.z - it.z;
    return dx * dx + dy * dy + dz * dz <= 1.5f * 1.5f;
}

std::string Track(int t) {
    char text[64];
    std::snprintf(text, sizeof(text), "Skate 3 Track %02d", t);
    return text;
}

// Skate's centre popup (`celebratory`: glow and crown); false when it cannot
// show now, then the caller keeps GTA's notice.
bool Popup(bool celebratory, const std::string& text) {
    // The movie keeps one line of 40 characters: a longer one would cut a
    // shop name, so GTA's notice says it in full instead.
    return g_popups && text.size() <= kPopupChars && hudoverlay::ShowCentral(celebratory ? 1 : 0, text.c_str());
}

void Burst(const Item& it) {
    if (!Call<bool>(HAS_NAMED_PTFX_ASSET_LOADED, kBurstAsset)) {
        Call<void>(REQUEST_NAMED_PTFX_ASSET, kBurstAsset);
        return;
    }
    Call<void>(USE_PARTICLE_FX_ASSET, kBurstAsset);
    Call<int>(START_PARTICLE_FX_NON_LOOPED_AT_COORD, kBurstEffect, it.x, it.y, it.z, 0.0f, 0.0f, 0.0f, kBurstScale,
              false, false, false, false);
}

void Earn(const Item& it) {
    Unblip(it.id);
    g_progress.state[it.id] = State::Collected;
    std::string reward, text;
    const auto mine = g_assigned.find(it.id);
    if (it.kind == Kind::Board) {
        // Its own complete board (collectibles v3): the deck unlocked and the
        // board in the garage.
        if (g_partsAssigned && mine != g_assigned.end() && !mine->second.key.empty() && boardshop::Unlock(mine->second.key) >= 0) {
            const std::string title = boardshop::GiveBoard(mine->second.key);
            if (!title.empty()) {
                reward = "board:" + mine->second.key;
                text = "A complete board in your garage: " + title;
                g_unlockedKeys.insert(mine->second.key);
            }
        }
    } else if (it.kind == Kind::Part) {
        // Its own deck or griptape (design 2026-11-02-collectibles-v2); one
        // already unlocked pays the cash below, never another's item.
        int result = -1;
        if (g_partsAssigned && mine != g_assigned.end() && !mine->second.key.empty()) result = boardshop::Unlock(mine->second.key);
        if (result == 1) {
            reward = "part:" + mine->second.key;
            text = "Unlocked in the skate shop: " + mine->second.title;
            g_unlockedKeys.insert(mine->second.key);
        } else if (result < 0 && !g_partsAssigned) {  // an older runtime: the next secret
            const std::string unlocked = boardshop::UnlockNext();
            if (!unlocked.empty()) {
                const size_t tab = unlocked.find('\t');
                reward = "part:" + unlocked.substr(0, tab);
                text = "Unlocked in the skate shop: " + (tab == std::string::npos ? unlocked : unlocked.substr(tab + 1));
            }
        }
    } else {
        std::vector<int> tracks;
        for (const auto& f : SongFiles())
            if (const int t = TrackOf(f.name)) tracks.push_back(t);
        const auto earned = EarnedSongs(g_progress);
        const int t = mine != g_assigned.end() ? mine->second.track : 0;
        if (t && std::find(tracks.begin(), tracks.end(), t) != tracks.end() && std::find(earned.begin(), earned.end(), t) == earned.end()) {
            reward = "song:" + std::to_string(t);
            text = "New song on Self Radio: " + Track(t) + ". Settings > Audio > Perform Quick Scan to hear it.";
        }
    }
    std::string name;  // the part's shop label, for the popup
    if (const size_t colon = text.find(": "); (reward.rfind("part:", 0) == 0 || reward.rfind("board:", 0) == 0) && colon != std::string::npos)
        name = text.substr(colon + 2);
    if (reward.empty()) {
        boardshop::Pay(kNoPartsLeftCash);
        reward = "cash:" + std::to_string(kNoPartsLeftCash);
        text = "Nothing left to unlock here: $" + std::to_string(kNoPartsLeftCash);
    }
    g_progress.reward[it.id] = reward;
    Save();
    if (it.kind == Kind::Song) Reconcile(true);
    Burst(it);
    if (Popup(true, PopupEarned(reward, name))) {
        Call<void>(PLAY_SOUND_FRONTEND, -1, "CHALLENGE_UNLOCKED", "HUD_AWARDS", 1);
        // Songs still need GTA's quick scan, which the popup cannot say.
        if (it.kind == Kind::Song && reward.rfind("song:", 0) == 0)
            Notice(Summary() + ": Settings > Audio > Perform Quick Scan to hear it.");
    } else {
        Call<void>(PLAY_SOUND_FRONTEND, -1, "PICK_UP", "HUD_FRONTEND_DEFAULT_SOUNDSET", 1);
        Notice(Summary() + ": " + text);
    }
    Log(it.id + " (" + it.place + ") collected: " + reward);
}

// Whether `it` shows its item: its key or track is still to earn (a song
// only when its file exists, so it never shows what it cannot give).
bool ShowsItem(const Item& it, const std::vector<int>& earnedSongs) {
    const auto a = g_assigned.find(it.id);
    if (a == g_assigned.end()) return false;
    if (GivesParts(it.kind)) return g_partsAssigned && !a->second.models.empty() && !g_unlockedKeys.count(a->second.key);
    const int t = a->second.track;
    return g_musicModel && t && g_tracksOnDisk.count(t) && std::find(earnedSongs.begin(), earnedSongs.end(), t) == earnedSongs.end();
}

void DeleteMusic(int slot) {
    int& obj = g_musicObj[slot];
    if (obj && Call<BOOL>(DOES_ENTITY_EXIST, obj)) {
        Call<void>(SET_ENTITY_AS_MISSION_ENTITY, obj, 1, 1);
        Call<void>(DELETE_OBJECT, &obj);
    }
    obj = 0;
}

void ClearItems() {
    boardparts::ClearPickups();
    for (int i = 0; i < kItemSlots; ++i) DeleteMusic(i);
    if (g_musicModel) Call<void>(SET_MODEL_AS_NO_LONGER_NEEDED, g_musicModel);
    g_itemWaiting.clear();
}

// The music prop, chosen once the game runs: the first of kMusicModels it has.
void ChooseMusicModel() {
    if (g_musicChecked) return;
    g_musicChecked = true;
    for (const char* name : kMusicModels) {
        const std::uint32_t hash = util::Joaat(name);
        if (Call<BOOL>(IS_MODEL_IN_CDIMAGE, hash)) {
            g_musicModel = hash;
            Log(std::string("songs show the music prop ") + name);
            return;
        }
    }
    Log("no music prop in the game: songs keep the question mark");
}

// Up close, the nearest collectibles show their item; returns their ids.
std::set<std::string> ShowItems(const Vector3& me) {
    std::vector<std::pair<float, const Item*>> closest;
    const auto earnedSongs = EarnedSongs(g_progress);  // once a frame
    for (const Item& it : g_items) {
        if (StateOf(g_progress, it.id) == State::Collected || !ShowsItem(it, earnedSongs)) continue;
        const float dx = it.x - me.x, dy = it.y - me.y, dz = it.z - me.z;
        const float d = std::sqrt(dx * dx + dy * dy + dz * dz);
        if (d <= kItemRange) closest.push_back({d, &it});
    }
    std::sort(closest.begin(), closest.end(), [](const auto& a, const auto& b) { return a.first < b.first; });
    if (closest.size() > static_cast<size_t>(kItemSlots)) closest.resize(kItemSlots);
    std::set<std::string> shown;
    const float t = (GetTickCount() % 3600000u) / 1000.0f;  // float seconds stay exact
    const float yaw = std::fmod(t * 90.0f, 360.0f), bob = 0.12f * std::sin(t * 2.0f);
    int part = 0, music = 0;
    for (const auto& [d, it] : closest) {
        const float at[3] = {it->x, it->y, it->z + bob};
        if (GivesParts(it->kind)) {
            SvBoardPose pose{};
            if (boardshop::PickupPose(at, yaw, pose) && boardparts::Pickup(part, g_assigned[it->id].models, pose)) {
                shown.insert(it->id);
                g_itemWaiting.erase(it->id);
            } else {
                // Loading takes a moment; only a long wait is worth a line.
                const DWORD now = GetTickCount();
                const DWORD since = g_itemWaiting.emplace(it->id, now).first->second;
                if (now - since > 15000 && g_itemFailLogged.insert(it->id).second)
                    Log(it->id + ": its deck model did not show in 15 s (the question mark shows)");
            }
            ++part;
        } else {
            int& obj = g_musicObj[music++];
            if (!obj) {
                Call<void>(REQUEST_MODEL, g_musicModel);
                if (Call<BOOL>(HAS_MODEL_LOADED, g_musicModel)) {
                    obj = Call<int>(CREATE_OBJECT_NO_OFFSET, g_musicModel, at[0], at[1], at[2], 0, 0, 0);
                    if (obj) Call<void>(SET_ENTITY_COLLISION, obj, 0, 0), Call<void>(FREEZE_ENTITY_POSITION, obj, 1);
                }
            }
            if (obj && Call<BOOL>(DOES_ENTITY_EXIST, obj)) {
                Call<void>(SET_ENTITY_COORDS_NO_OFFSET, obj, at[0], at[1], at[2], 0, 0, 0);
                Call<void>(SET_ENTITY_HEADING, obj, yaw);
                shown.insert(it->id);
            }
        }
    }
    for (int i = part; i < kItemSlots; ++i) boardparts::Pickup(i, {}, SvBoardPose{});
    for (int i = music; i < kItemSlots; ++i) DeleteMusic(i);
    return shown;
}

void Find(const Item& it) {
    g_progress.state[it.id] = State::Found;
    Save();
    Blip(it);
    Call<void>(PLAY_SOUND_FRONTEND, -1, "CHECKPOINT_NORMAL", "HUD_MINI_GAME_SOUNDSET", 1);
    if (!Popup(false, PopupFound(it.kind)))
        Notice(std::string("You found a hidden ") + (it.kind == Kind::Board ? "board" : it.kind == Kind::Part ? "board part" : "song") + " (" + it.place +
               "): reach it while skating");
    Log(it.id + " found");
}

}  // namespace

void Start(const std::wstring& gameDir, bool enabled, util::LogFn log) {
    g_log = log;
    g_placesPath = gameDir + L"GTAxSkate\\collectibles.tsv";
    // Collectibles v3 starts fresh (design 2026-11-03-collectibles-v3); v2's
    // collectibles.txt stays for a rollback.
    g_progressPath = gameDir + L"GTAxSkate\\collectibles3.txt";
    g_songsDir = SongsDir();
    std::string text;
    g_items = Read(g_placesPath, text) ? ParsePlaces(text) : std::vector<Item>{};
    g_faulty = !Read(g_progressPath, text);
    g_progress = ParseProgress(text);
    if (g_faulty) Log("GTAxSkate\\collectibles3.txt exists but cannot be read: progress, songs and the shop stay as they are this session");
    g_wasEnabled = enabled;
    Log(std::to_string(g_items.size()) + " places, " + Summary());
    if (g_items.empty()) return;  // no places file: songs stay as they are
    Reconcile(enabled);
}

void SyncShop() {
    // No places file: nothing to earn, so nothing is locked either.
    if (g_faulty || g_items.empty()) return;
    int parts = 0, earned = 0;
    for (const Item& it : g_items) parts += GivesParts(it.kind);
    earned = static_cast<int>(EarnedKeys(g_progress).size());
    int locked = boardshop::SetSecrets(parts);
    const std::string list = boardshop::SecretList();
    g_partsAssigned = !list.empty();
    g_unlockedKeys.clear();
    for (const std::string& key : EarnedKeys(g_progress)) g_unlockedKeys.insert(key);
    if (g_partsAssigned) {
        // A new secrets file (deleted, or a new install) after earlier finds:
        // the very keys earned are unlocked again.
        for (const std::string& key : g_unlockedKeys) boardshop::Unlock(key);
    } else {
        // An older runtime: as many as were earned, in the same order as before.
        while (locked > parts - earned && !boardshop::UnlockNext().empty()) --locked;
    }
    g_assigned = Assign(g_items, list);
    Log(std::string("each collectible holds its item") + (g_partsAssigned ? "" : " (songs only: the runtime lists no secrets)"));
}

std::string Reset() {
    if (g_faulty) return "Collectibles: the progress file cannot be read, nothing was reset";
    if (g_items.empty()) return "Collectibles: no places file, nothing to reset";
    boardshop::CloseMenu();  // its Found list would be stale
    const int locked = boardshop::RelockAll();
    if (locked < 0) return "Collectibles: the shop cannot lock its parts again (runtime or shop off), nothing was reset";
    SYSTEMTIME t;
    GetLocalTime(&t);
    wchar_t stamp[32];
    swprintf_s(stamp, L"%04u%02u%02u-%02u%02u%02u", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond);
    const std::wstring backup = g_progressPath.substr(0, g_progressPath.size() - 4) + L"-" + stamp + L".bak";
    // No backup, no reset: the parts earned are unlocked again at once.
    if (GetFileAttributesW(g_progressPath.c_str()) != INVALID_FILE_ATTRIBUTES && !CopyFileW(g_progressPath.c_str(), backup.c_str(), FALSE))
    {
        for (const std::string& key : g_unlockedKeys) boardshop::Unlock(key);  // the shop as it was
        return "Collectibles: the progress could not be backed up, nothing was reset";
    }
    g_progress = Progress{};
    g_unlockedKeys.clear();
    Save();
    Clear();
    Reconcile(g_wasEnabled);
    Log("reset (Developer): progress and " + std::to_string(locked) + " shop unlocks; the old progress kept as a .bak");
    return "Collectibles reset: " + Summary() + ". Bought parts stay in your garage. Settings > Audio > Perform Quick Scan to update Self Radio.";
}

void Tick(bool enabled, bool skating, bool popups, const float (&skater)[3], int ped, bool inAir) {
    g_popups = popups;
    // The burst's effect loaded ahead of the first collect.
    if (enabled && !g_items.empty() && !Call<bool>(HAS_NAMED_PTFX_ASSET_LOADED, kBurstAsset))
        Call<void>(REQUEST_NAMED_PTFX_ASSET, kBurstAsset);
    if (g_items.empty() || g_faulty) return;
    if (enabled != g_wasEnabled) {
        g_wasEnabled = enabled;
        if (!enabled) Clear();
        g_songFailLogged = false;
        Reconcile(enabled);
    }
    if (!enabled) return;
    if (g_songsPending && GetTickCount() >= g_nextSongRetry) Reconcile(true);
    ChooseMusicModel();
    const Vector3 me = Call<Vector3>(GET_ENTITY_COORDS, ped, 0);
    const std::set<std::string> items = ShowItems(me);
    // A teleport (travel, respawn) is no ride through everything between.
    const float jx = skater[0] - g_prev[0], jy = skater[1] - g_prev[1], jz = skater[2] - g_prev[2];
    if (jx * jx + jy * jy + jz * jz > 20.0f * 20.0f) g_hadSkater = false;
    const DWORD now = GetTickCount();
    const bool sight = now >= g_nextSight;
    const Item* nearestHidden = nullptr;
    float nearestHiddenDist = kFindRange;
    for (const Item& it : g_items) {
        const State st = StateOf(g_progress, it.id);
        if (st == State::Collected) continue;
        const float dx = it.x - me.x, dy = it.y - me.y, dz = it.z - me.z;
        const float d = std::sqrt(dx * dx + dy * dy + dz * dz);
        if (st == State::Found) Blip(it);  // after a load or Collectibles back on
        if (d > kDrawRange) continue;
        Draw(it, d, st == State::Found, !items.count(it.id));
        if (skating && g_hadSkater && (!it.air || inAir)) {
            const float c[3] = {it.x, it.y, it.z - kRootBelow};
            if (SegmentHits(g_prev, skater, c, it.air ? kAirReach : kReach)) {
                Earn(it);
                continue;
            }
        }
        if (st == State::Hidden && d < nearestHiddenDist) nearestHidden = &it, nearestHiddenDist = d;
    }
    if (sight && nearestHidden) {
        g_nextSight = now + kSightEvery;
        if (InSight(Call<Vector3>(GET_FINAL_RENDERED_CAM_COORD), *nearestHidden, ped)) Find(*nearestHidden);
    }
    g_hadSkater = skating;
    for (int i = 0; i < 3; ++i) g_prev[i] = skater[i];
}

const char* TakeNotice() {
    if (g_notices.empty()) return nullptr;
    g_notice = g_notices.front();
    g_notices.pop_front();
    return g_notice.c_str();
}

std::string Summary() {
    int all = 0, got = 0, parts = 0, partsGot = 0, songs = 0, songsGot = 0;
    for (const Item& it : g_items) {
        const bool done = StateOf(g_progress, it.id) == State::Collected;
        ++all, got += done;
        if (GivesParts(it.kind)) ++parts, partsGot += done;
        else ++songs, songsGot += done;
    }
    char text[128];
    std::snprintf(text, sizeof(text), "Collectibles: %d/%d (parts %d/%d, songs %d/%d)", got, all, partsGot, parts, songsGot, songs);
    return text;
}

const char* RouteToNearestFound() {
    const Vector3 me = Call<Vector3>(GET_ENTITY_COORDS, Call<int>(PLAYER_PED_ID), 0);
    const Item* best = nullptr;
    float bestD = 1e30f;
    for (const Item& it : g_items) {
        if (StateOf(g_progress, it.id) != State::Found) continue;
        const float d = (it.x - me.x) * (it.x - me.x) + (it.y - me.y) * (it.y - me.y);
        if (d < bestD) best = &it, bestD = d;
    }
    if (!best) return "No found collectible left to reach: look out for question marks while skating";
    Call<void>(SET_NEW_WAYPOINT, best->x, best->y);
    return "GPS: the nearest found collectible";
}

std::string SelectedLabel() {
    if (g_items.empty()) return "Collectible: none (GTAxSkate\\collectibles.tsv missing)";
    if (g_sel < 0 || g_sel >= static_cast<int>(g_items.size())) g_sel = 0;
    const Item& it = g_items[g_sel];
    const State st = StateOf(g_progress, it.id);
    return "Collectible: " + it.id + (it.kind == Kind::Board ? " board " : it.kind == Kind::Part ? " part " : " song ") +
           (st == State::Collected ? "[collected] " : st == State::Found ? "[found] " : "[hidden] ") + it.place;
}

void Step(int direction) {
    if (g_items.empty()) return;
    const int n = static_cast<int>(g_items.size());
    g_sel = ((g_sel < 0 ? 0 : g_sel) + (direction < 0 ? -1 : 1) + n) % n;
}

bool SelectedPlace(float (&xyz)[3]) {
    if (g_sel < 0 || g_sel >= static_cast<int>(g_items.size())) return false;
    const Item& it = g_items[g_sel];
    xyz[0] = it.x, xyz[1] = it.y, xyz[2] = it.z;
    return true;
}

std::vector<int> Entities() {
    std::vector<int> out;
    for (const int obj : g_musicObj)
        if (obj) out.push_back(obj);
    return out;
}

void Clear() {
    ClearItems();
    for (auto& [id, b] : g_blips) {
        int handle = b;
        if (Call<BOOL>(DOES_BLIP_EXIST, handle)) Call<void>(REMOVE_BLIP, &handle);
    }
    g_blips.clear();
}

}  // namespace collectibles
