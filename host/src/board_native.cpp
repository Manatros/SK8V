#include "board_native.h"
#include "board_native_detail.h"
#include "game_probe.h"
#include "host_util.h"
#include "natives.h"
#include "streaming.h"
#include <windows.h>
#include <main.h>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <cstdio>

namespace boardnative {
namespace {
using posemath::M;
constexpr std::uint32_t Model=util::Joaat("skatev_board");
void (*g_log)(const char*)=nullptr;
std::string g_ydr,g_ytyp; int g_object=0;
bool g_started=false,g_registered=false,g_type=false,g_failed=false;
PoseCache g_cache;
std::uint32_t g_ydrId=~0u,g_ytypId=~0u;
ULONGLONG g_begin=0;
void Log(const char* s){if(g_log)g_log(s);}
bool Pose(const SvBoardPose& pose) { return PoseObject(g_object,pose,g_cache,g_log); }
// One bounded summary for the whole board renderer per five seconds. No
// allocation or extra matrix reads while Developer verbose logging is off.
bool ReadDiagnosticRows(std::uintptr_t address,const int* indices,M (&out)[7]) {
    __try {
        for(int i=0;i<7;++i) {
            out[i]=posemath::Identity();
            const auto* source=reinterpret_cast<const float*>(address)+indices[i]*16;
            for(int r=0;r<4;++r)for(int c=0;c<3;++c)out[i].r[r][c]=source[r*4+c];
        }
        return true;
    } __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
}
namespace {
bool Fail(int object,PoseCache& cache,void (*log)(const char*),const char* why) {
    if(cache.failed!=why&&log){char line[200];std::snprintf(line,sizeof(line),"board pose: object %d not posed: %s",object,why);log(line);}
    cache.failed=why;
    return false;
}
}
bool PoseObject(int object,const SvBoardPose& pose,PoseCache& cache,void (*log)(const char*)) {
    probe::SkeletonInfo s; const char* why=nullptr;
    if(!probe::ResolvePedSkeleton(object,s,why)||s.count!=7){if(!cache.reported&&log){char line[256];std::snprintf(line,sizeof(line),"native board: skeleton unavailable (%s, bones=%d); object hidden",why?why:"expected seven bones",s.count);log(line);cache.reported=true;}return false;}
    if(s.skeleton!=cache.skeleton) {
        cache.skeleton=0;
        cache.diagnostic=diagnostics::State{};
        for(int i=0;i<7;++i) { cache.indices[i]=probe::BoneIndexByTag(s,static_cast<std::uint16_t>(i)); if(cache.indices[i]<0||cache.indices[i]>=7)return Fail(object,cache,log,"no bone with tag 0..6"); }
        cache.skeleton=s.skeleton;
    }
    const int* indices=cache.indices;
    __try {
        const auto data=*reinterpret_cast<const std::uintptr_t*>(s.skeleton);
        const auto parents=*reinterpret_cast<const std::int16_t* const*>(data+0x38);
        if(!parents||!s.parentMtx||!s.objectMtx||!s.globalMtx)return Fail(object,cache,log,"skeleton matrices missing");
        for(int i=0;i<7;++i) if(parents[indices[i]]!=(detail::Parents[i]<0?-1:indices[detail::Parents[i]]))return Fail(object,cache,log,"bone parents differ from the board's");
        M world[7],objects[7],locals[7],entity;
        std::memcpy(world,pose.world,sizeof(world)); std::memcpy(&entity,reinterpret_cast<void*>(s.parentMtx),64);
        if(!detail::Convert(world,entity,objects,locals))return Fail(object,cache,log,"pose not convertible");
        const bool verbose=util::g_verbose.load(std::memory_order_relaxed);
        const auto now=verbose?GetTickCount64():0;
        bool sample=diagnostics::ShouldSample(cache.diagnostic,verbose,pose.tick,now);
        M before[7],after[7],objectsAfter[7];
        if(sample)sample=ReadDiagnosticRows(s.objectMtx,indices,before);
        for(int i=0;i<7;++i) for(int r=0;r<4;++r) for(int c=0;c<3;++c) {
            reinterpret_cast<float*>(s.objectMtx)[indices[i]*16+r*4+c]=locals[i].r[r][c];
            reinterpret_cast<float*>(s.globalMtx)[indices[i]*16+r*4+c]=objects[i].r[r][c];
        }
        if(sample&&ReadDiagnosticRows(s.objectMtx,indices,after)&&ReadDiagnosticRows(s.globalMtx,indices,objectsAfter)) {
            auto& d=cache.diagnostic;
            diagnostics::Sample(d,pose.tick,now,locals,before,after,objects,objectsAfter);
            if(log&&now-cache.logAt>=5000) {
                cache.logAt=now;
                char line[768];
                std::snprintf(line,sizeof(line),
                    "board anim diagnostic: object %d tick %llu samples %u; parent-local motion deg TF/TB/WLF/WLB/WRF/WRB [%.3f %.3f %.3f %.3f %.3f %.3f]; before-vs-prior-written deg [%.3f %.3f %.3f %.3f %.3f %.3f]; immediate readback rotation max error local %.6g object %.6g; observations only, not render/overwrite proof",
                    object,static_cast<unsigned long long>(pose.tick),d.samples,
                    d.motion[0],d.motion[1],d.motion[2],d.motion[3],d.motion[4],d.motion[5],
                    d.beforeWritten[0],d.beforeWritten[1],d.beforeWritten[2],d.beforeWritten[3],d.beforeWritten[4],d.beforeWritten[5],
                    d.localError,d.objectError);
                log(line);diagnostics::ClearWindow(d);
            }
        }
        if(verbose)diagnostics::Written(cache.diagnostic,locals);
        cache.failed=nullptr;
        return true;
    } __except(EXCEPTION_EXECUTE_HANDLER){return Fail(object,cache,log,"fault reading the skeleton");}
}
// GTA moves the board between rooms by itself (logs 2026-10-17: the object's
// own room followed the player through the subway). Forcing rooms fought it
// and an interior/room pair from two places hid the board, so the board is
// left to GTA like any other object; this only logs where GTA has it.
void LogRoom(int object,RoomCache& cache,const char* who,void (*log)(const char*)) {
    if(!log||!object||!util::g_verbose||cache.logs>=40)return;
    Room actual{gta::Call<int>(gta::GET_INTERIOR_FROM_ENTITY,object),0};
    if(actual.interior)actual.room=gta::Call<std::uint32_t>(gta::GET_ROOM_KEY_FROM_ENTITY,object);
    if(actual.interior==cache.last.interior&&actual.room==cache.last.room)return;
    cache.last=actual;
    if(cache.logs++>=40)return;
    const int ped=gta::Call<int>(gta::PLAYER_PED_ID);
    const int pi=gta::Call<int>(gta::GET_INTERIOR_FROM_ENTITY,ped);
    char line[200];
    std::snprintf(line,sizeof(line),"board room: %s in interior %d room %u (GTA's own); player in interior %d room %u",who,actual.interior,actual.room,
        pi,pi?gta::Call<std::uint32_t>(gta::GET_ROOM_KEY_FROM_ENTITY,ped):0u);
    log(line);
}
namespace {
// GTA scales each entity's ambient light by two bytes it keeps on the entity:
// natural (sky) at +0xC4 and artificial (interior fill) at +0xC5. SkateVLight.log
// 2026-10-17 (build 3889): the player's were 254/0 outdoors and 0/51..153 in the
// subway; every board object stayed at 255/255, full sky and lamp light even
// underground ("right in daylight, too bright in tunnels").
constexpr std::size_t kNaturalAmbient = 0xC4;
constexpr float kShareReach = 4.0f;
ULONGLONG g_ambientLogAt = 0;
bool CopyAmbient(const std::uint8_t* from, std::uint8_t* to, std::uint8_t (&had)[2]) {
    __try {
        had[0] = to[kNaturalAmbient]; had[1] = to[kNaturalAmbient + 1];
        to[kNaturalAmbient] = from[kNaturalAmbient]; to[kNaturalAmbient + 1] = from[kNaturalAmbient + 1];
        return true;
    } __except (EXCEPTION_EXECUTE_HANDLER) { return false; }
}
}
// GTA works these out for the player's spot every frame but not for our board
// objects, so a board in the same room within a few metres of the player
// (riding, carried, store preview) takes the player's: the light of the place
// the board is. A board elsewhere keeps GTA's own.
void ShareAmbient(int object, void (*log)(const char*)) {
    const int ped = gta::Call<int>(gta::PLAYER_PED_ID);
    if (!object || !ped || gta::Call<int>(gta::GET_INTERIOR_FROM_ENTITY, object) != gta::Call<int>(gta::GET_INTERIOR_FROM_ENTITY, ped)) return;
    const Vector3 a = gta::Call<Vector3>(gta::GET_ENTITY_COORDS, object, true), b = gta::Call<Vector3>(gta::GET_ENTITY_COORDS, ped, true);
    const float dx = a.x - b.x, dy = a.y - b.y, dz = a.z - b.z;
    if (dx * dx + dy * dy + dz * dz > kShareReach * kShareReach) return;
    const auto* from = getScriptHandleBaseAddress(ped);
    auto* to = getScriptHandleBaseAddress(object);
    std::uint8_t had[2]{};
    if (!from || !to || !CopyAmbient(from, to, had)) return;
    const auto now = GetTickCount64();
    if (!log || !util::g_verbose || now - g_ambientLogAt < 5000) return;
    g_ambientLogAt = now;
    char line[160];
    std::snprintf(line, sizeof(line), "board ambient: sky %u interior %u from the player (the board had %u %u)", from[kNaturalAmbient],
                  from[kNaturalAmbient + 1], had[0], had[1]);
    log(line);
}
namespace {
float g_glow = 0.0f;
int g_glowRgb[3] = {255, 255, 255};
constexpr float kGlowOut = 0.02f;
// Manatros (test 22:1x): "a little dimmer, softer / fade faster to the edges and
// only glow strong when on 100%". Brightness follows strength squared (50 %
// gives a quarter), at 100 % below the previous build's (30 / 4).
// The ground glow reaches further the brighter it is (Manatros 2026-10-18: "the
// light should leak further and more realistic"): 0.15 m at the faintest,
// 1.3 m at 100 %, while its intensity falls off with distance as GTA's does.
constexpr float kGroundRange = 0.15f, kGroundReach = 1.15f, kGroundBright = 3.0f;
}
namespace {
int g_wheelRgb[3] = {255, 255, 255};
}
void SetWheelColour(int r, int g, int b) { g_wheelRgb[0] = r, g_wheelRgb[1] = g, g_wheelRgb[2] = b; }
const int* WheelColour() { return g_wheelRgb; }
void SetWheelGlow(float strength, int r, int g, int b) {
    g_glow = strength < 0.0f ? 0.0f : strength > 1.0f ? 1.0f : strength;
    g_glowRgb[0] = r; g_glowRgb[1] = g; g_glowRgb[2] = b;
}
void GlowWheels(const SvBoardPose& pose) {
    if (g_glow <= 0.0f || pose.bone_count != 7) return;
    const float level = g_glow * g_glow;
    const int* rgb = g_glowRgb[0] < 0 ? g_wheelRgb : g_glowRgb;
    for (int w = 3; w < 7; ++w) {
        const float* wheel = pose.world[w] + 12;
        const float* truck = pose.world[detail::Parents[w]] + 12;
        float out[3] = {wheel[0] - truck[0], wheel[1] - truck[1], wheel[2] - truck[2]};
        const float length = std::sqrt(out[0] * out[0] + out[1] * out[1] + out[2] * out[2]);
        if (!std::isfinite(length)) return;
        for (float& f : out) f = length > 1e-4f ? f / length : 0.0f;
        // The wheel itself glows through its emissive model copy (board_parts
        // SetGlowWheels): no light aimed at it from outside any more (Manatros
        // 2026-10-18: "lit by an invisible light, and only on the outsides").
        // A small glow on the ground, just past the outer face (a wheel is ~3.5 cm
        // wide). Was 0.45 m at 12x: "too aggressive ... even 5% is too much".
        gta::Call<void>(gta::DRAW_LIGHT_WITH_RANGE, wheel[0] + out[0] * kGlowOut, wheel[1] + out[1] * kGlowOut, wheel[2] + out[2] * kGlowOut,
                        rgb[0], rgb[1], rgb[2], kGroundRange + kGroundReach * g_glow, kGroundBright * level);
    }
}
// SK8V's own placement: a frozen object set to the pose every frame. The
// attached (Skater) and unfrozen (Free) test modes of 2026-10-17 let GTA move
// the board a frame late, off the pose by the distance travelled ("clipping
// sometimes works great sometimes it doesnt"); the dark-area brightness they
// chased was the ambient scale (ShareAmbient).
void Place(int object, const SvBoardPose& pose, float qx, float qy, float qz, float qw) {
    // Native board and shop parts are separate objects, so GTA's first-person
    // head/hat hiding does not hide them. Keep these presentation-only objects
    // out of the gameplay camera's collision scan, including carried/grab poses.
    gta::Call<void>(gta::SET_GAMEPLAY_CAM_IGNORE_ENTITY_COLLISION_THIS_UPDATE, object);
    if (gta::Call<bool>(gta::IS_ENTITY_ATTACHED, object)) gta::Call<void>(gta::DETACH_ENTITY, object, false, false);
    gta::Call<void>(gta::FREEZE_ENTITY_POSITION, object, true);
    gta::Call<void>(gta::SET_ENTITY_COORDS_NO_OFFSET, object, pose.entity[12], pose.entity[13], pose.entity[14], false, false, false);
    gta::Call<void>(gta::SET_ENTITY_QUATERNION, object, qx, qy, qz, qw);
}
namespace {
RoomCache g_room;
void Remove(){if(g_object&&gta::Call<bool>(gta::DOES_ENTITY_EXIST,g_object))gta::Call<void>(gta::DELETE_OBJECT,&g_object);g_object=0;}
bool Hide(){if(g_object&&gta::Call<bool>(gta::DOES_ENTITY_EXIST,g_object))gta::Call<void>(gta::SET_ENTITY_VISIBLE,g_object,false,false);return false;}
void Fail(const char* why){Log(why);Remove();g_failed=true;}
}
bool Start(const std::wstring& iniPath,const std::wstring& dataRoot,void (*log)(const char*)) {
    Stop(); g_log=log;g_failed=false;g_cache.reported=false;
    if(!GetPrivateProfileIntW(L"SkateV",L"BoardNative",1,iniPath.c_str()))return false;
    const auto root=std::filesystem::path(dataRoot)/L"private"/L"board-native";
    const auto ydr=root/L"skatev_board.ydr",ytyp=root/L"skatev_board.ytyp";
    if(!std::filesystem::is_regular_file(ydr)||!std::filesystem::is_regular_file(ytyp)){Log("native board: private YDR/YTYP missing");return false;}
    auto dr=streaming::NarrowAnsi(ydr),ty=streaming::NarrowAnsi(ytyp);
    if(dr.empty()||ty.empty()||((g_ydrId!=~0u||g_ytypId!=~0u)&&(dr!=g_ydr||ty!=g_ytyp))){Log("native board: unsupported or changed resource path");return false;}
    if(!streaming::Guards()){Log("native board: 3889 streaming function guards differ");return false;}
    g_ydr=dr;g_ytyp=ty;g_started=true;g_begin=GetTickCount64();return true;
}
bool Tick(bool skating,const SvBoardPose& pose) {
    if(!g_started||g_failed)return false;
    if(!skating){Remove();return false;}
    if(pose.size!=sizeof(SvBoardPose)||pose.bone_count!=7)return Hide();
    for(const auto& bone:pose.world)for(float f:bone)if(!std::isfinite(f))return Hide();
    M entity;std::memcpy(&entity,pose.entity,sizeof(entity));posemath::Q quaternion;if(!posemath::RigidQuaternion(entity,quaternion))return Hide();
    if(!g_type) {
        g_begin=GetTickCount64();
        if(!streaming::Ready())return false;
        if(!g_registered) {
            if(gta::Call<bool>(gta::IS_MODEL_IN_CDIMAGE,Model)||gta::Call<bool>(gta::IS_MODEL_VALID,Model)){Fail("native board: custom model name already occupied");return false;}
            const auto reg=streaming::Registrar();
            if(g_ydrId==~0u)reg(&g_ydrId,g_ydr.c_str(),true,"skatev_board.ydr",false,false);
            if(g_ytypId==~0u)reg(&g_ytypId,g_ytyp.c_str(),true,"skatev_board.ytyp",false,false);
            if(g_ydrId==~0u||g_ytypId==~0u){Fail("native board: raw resource registration failed");return false;}
            g_registered=true;
        }
        streaming::TypeLoader()("skatev_board.ytyp");g_type=true;Log("native board: custom YDR/YTYP registered; archetype requested");
    }
    if(!gta::Call<bool>(gta::IS_MODEL_VALID,Model)||!gta::Call<bool>(gta::IS_MODEL_IN_CDIMAGE,Model)||!gta::Call<bool>(gta::HAS_MODEL_LOADED,Model)) {
        if(gta::Call<bool>(gta::IS_MODEL_VALID,Model))gta::Call<void>(gta::REQUEST_MODEL,Model);
        if(GetTickCount64()-g_begin>15000)Fail("native board: model load timed out");return false;
    }
    if(!g_object) {
        // The pinned Legacy DB and owned +CDCB74 wrapper both require eight
        // slots; +37F4D7D reads p7 at args+0x38 even in a local script.
        g_object=gta::Call<int>(gta::CREATE_OBJECT_NO_OFFSET,Model,pose.entity[12],pose.entity[13],pose.entity[14],false,false,false,0);
        if(!g_object){Fail("native board: CREATE_OBJECT_NO_OFFSET failed");return false;}
        gta::Call<void>(gta::SET_ENTITY_VISIBLE,g_object,false,false);
        gta::Call<void>(gta::SET_ENTITY_AS_MISSION_ENTITY,g_object,true,false);gta::Call<void>(gta::FREEZE_ENTITY_POSITION,g_object,true);
        gta::Call<void>(gta::SET_ENTITY_HAS_GRAVITY,g_object,false);gta::Call<void>(gta::SET_ENTITY_DYNAMIC,g_object,false);
        gta::Call<void>(gta::SET_ENTITY_COLLISION,g_object,false,false);
        // Despite the modern name, this Legacy native uses collision-enable
        // polarity, matching the measured ped collisionDisabled=1 gate.
        gta::Call<void>(gta::SET_ENTITY_COMPLETELY_DISABLE_COLLISION,g_object,false,false);
        Log("native board: presentation object created; physics and collision disabled");g_room=RoomCache{};
    }
    if(!gta::Call<bool>(gta::DOES_ENTITY_EXIST,g_object)){g_object=0;return false;}
    Place(g_object,pose,quaternion.x,quaternion.y,quaternion.z,quaternion.w);
    const bool posed=Pose(pose);
    LogRoom(g_object,g_room,"native board",g_log);ShareAmbient(g_object,g_log);
    if(!posed&&!g_cache.reported){Log("native board: invalid bone tags, parents, or matrices; object hidden");g_cache.reported=true;}
    gta::Call<void>(gta::SET_ENTITY_VISIBLE,g_object,posed,false);return posed;
}
void Stop(){Remove();g_started=false;}
int Entity(){return g_object;}
}
