#include "ai_followers.h"
#include "runtime_bridge.h"
#include "game_probe.h"
#include "host_util.h"
#include "natives.h"
#include "pose_hook.h"
#include "board_native.h"
#include <main.h>
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <deque>
#include <filesystem>
#include <share.h>
#include <cstring>
#include <unordered_map>
#include <chrono>

namespace aifollowers {
namespace {
FILE* diagnostics=nullptr;
DWORD summaryAt=0;
std::unordered_map<int,std::string> decisions;
std::string managerStatus;
template<class... A> void Trace(const char* format,A... args) {
    if(!diagnostics)return;
    SYSTEMTIME t{};GetLocalTime(&t);
    std::fprintf(diagnostics,"%02u:%02u:%02u.%03u ",t.wHour,t.wMinute,t.wSecond,t.wMilliseconds);
    std::fprintf(diagnostics,format,args...);std::fputc('\n',diagnostics);std::fflush(diagnostics);
}
struct Rider {
    int ped=0, player=0, group=0;
    unsigned model=0,id=0;
    bool held=false, preparing=false, collision=true, ragdoll=true, groupSuspended=false;
    DWORD started=0, goalAt=0, outputAt=0;
    std::uint64_t tick=0;
    SvVec3 placed{};
    DWORD traceAt=0;
    bool boardReady=false;
};
Rider riders[4];
struct HostTiming {
    using Clock=std::chrono::steady_clock;
    Clock::time_point start=Clock::now();
    ~HostTiming() {
        static auto reportAt=Clock::now();
        static double total=0.,peak=0.;static unsigned frames=0;
        const auto now=Clock::now();
        const double ms=std::chrono::duration<double,std::milli>(now-start).count();
        total+=ms;peak=(std::max)(peak,ms);++frames;
        if(now-reportAt>=std::chrono::seconds(5)) {
            unsigned active=0;for(const auto& r:riders)if(r.held)++active;
            Trace("AI host performance active=%u frames=%u mean_ms=%.3f peak_ms=%.3f",active,frames,total/frames,peak);
            total=peak=0.;frames=0;reportAt=now;
        }
    }
};
DWORD scanAt=0, cooldown[4]{};
unsigned nextId=1;
int leader=0;
std::deque<SvVec3> trail;
float Dist(const SvVec3& a,const SvVec3& b) {return std::hypot(a.x-b.x,a.y-b.y);}
bool Exists(const Rider& r) {
    return r.ped&&gta::Call<BOOL>(gta::DOES_ENTITY_EXIST,r.ped)&&gta::Call<unsigned>(gta::GET_ENTITY_MODEL,r.ped)==r.model;
}
const char* BusyReason(int ped) {
    if(gta::Call<int>(gta::GET_SCRIPT_TASK_STATUS,ped,util::Joaat("SCRIPT_TASK_ENTER_VEHICLE"))==1)return "entering_vehicle";
    if(gta::Call<BOOL>(gta::IS_ENTITY_DEAD,ped,false))return "dead";
    if(gta::Call<BOOL>(gta::IS_PED_IN_ANY_VEHICLE,ped,false))return "in_vehicle";
    if(gta::Call<BOOL>(0x4859F1FC66A6278EULL,ped,0))return "combat";
    if(gta::Call<BOOL>(0x9DE327631295B4C2ULL,ped))return "swimming";
    return nullptr;
}
bool GroupFollower(int ped,int group) {return gta::Call<BOOL>(0x9BB01E3834671191ULL,ped,group)!=0;}
bool ScriptFollower(int ped,int player) {
    // Mission escorts need not belong to the player's group. Require an
    // active entity-follow task and friendly relationship, not mission flag alone.
    if(!gta::Call<BOOL>(0x0A7B270912999B3CULL,ped)||gta::Call<int>(0xEBA5AD3A0EAF7121ULL,ped,player)>2)return false;
    for(unsigned task:{util::Joaat("SCRIPT_TASK_FOLLOW_TO_OFFSET_OF_ENTITY"),util::Joaat("SCRIPT_TASK_GO_TO_ENTITY")})
        if(gta::Call<int>(gta::GET_SCRIPT_TASK_STATUS,ped,task)==1)return true;
    return false;
}
void Release(unsigned slot,RuntimeBridge& runtime,const char* reason="unspecified") {
    Rider& r=riders[slot];
    Trace("release slot=%u ped=%d model=%08x id=%u held=%d tick=%llu reason=%s",slot,r.ped,r.model,r.id,r.held,static_cast<unsigned long long>(r.tick),reason);
    posehook::RemoveFollower(slot);
    boardnative::StopFollower(slot);
    if(r.ped)runtime.FollowerStart(slot,nullptr);
    if((r.held||r.preparing)&&Exists(r)) {
        gta::Call<void>(gta::FREEZE_ENTITY_POSITION,r.ped,false);
        gta::Call<void>(gta::SET_ENTITY_COLLISION,r.ped,r.collision,false);
        gta::Call<void>(gta::SET_PED_CAN_RAGDOLL,r.ped,r.ragdoll);
        if(r.groupSuspended&&gta::Call<BOOL>(gta::DOES_ENTITY_EXIST,r.player)&&
            gta::Call<int>(0xF162E133B4E7A675ULL,r.player)==r.group&&
            !gta::Call<BOOL>(0x5891CAC5D4ACFF74ULL,r.ped)) { // IS_PED_IN_GROUP
            gta::Call<void>(0x9F3480FE65DB31B5ULL,r.ped,r.group); // SET_PED_AS_GROUP_MEMBER
            Trace("group_restored ped=%d group=%d",r.ped,r.group);
        }
        if(GroupFollower(r.ped,r.group))gta::Call<void>(0x2E2F4240B3F24647ULL,r.ped,r.group,true);
        // Explicit mission tasks are never cleared. Automatic group following
        // resumes after restoring the same player's group membership.
    }
    r={};cooldown[slot]=GetTickCount()+5000;
}
bool Start(unsigned slot,int ped,int player,int group,RuntimeBridge& runtime,const std::string& cache) {
    Trace("setup_begin slot=%u ped=%d model=%08x player=%d group=%d",slot,ped,gta::Call<unsigned>(gta::GET_ENTITY_MODEL,ped),player,group);
    probe::SkeletonInfo sk;const char* why=nullptr;std::string json;
    if(!probe::ResolvePedSkeleton(ped,sk,why)){Trace("setup_failed slot=%u ped=%d stage=skeleton reason=%s",slot,ped,why?why:"unavailable");return false;}
    if(!probe::SkeletonJson(sk,json)){Trace("setup_failed slot=%u ped=%d stage=skeleton_json bones=%d",slot,ped,sk.count);return false;}
    Trace("setup_skeleton slot=%u ped=%d bones=%d bytes=%zu",slot,ped,sk.count,json.size());
    SvFollowerStart request{};request.size=sizeof(request);request.id=nextId++;
    if(!request.id)request.id=nextId++;
    request.entity=ped;
    auto p=gta::Call<Vector3>(gta::GET_ENTITY_COORDS,ped,true);
    float z=p.z;
    // Native slots carry raw bits, not numeric conversions. GTA reads a
    // 32-bit float here, so passing a double corrupts the query height.
    const float queryZ=p.z+2.0f;
    static_assert(std::is_same_v<decltype(queryZ),const float>);
    float nativeZ=0.;std::memcpy(&nativeZ,&queryZ,sizeof(nativeZ));
    const bool grounded=gta::Call<BOOL>(gta::GET_GROUND_Z_FOR_3D_COORD,p.x,p.y,queryZ,&z,false,false)!=0;
    Trace("ground_query slot=%u ped=%d at=(%.3f,%.3f,%.3f) requested_z=%.3f argument_type=float native_float_z=%.9g success=%d ground_z=%.3f",slot,ped,p.x,p.y,p.z,queryZ,nativeZ,grounded,z);
    if(!grounded){Trace("setup_failed slot=%u ped=%d stage=ground_query",slot,ped);return false;}
    request.spawn={p.x,p.y,z};request.heading=gta::Call<float>(gta::GET_ENTITY_HEADING,player);
    auto& character=request.character;character.size=sizeof(character);
    character.model_hash=gta::Call<unsigned>(gta::GET_ENTITY_MODEL,ped);character.cache_root_utf8=cache.c_str();
    for(int i=0;i<12;++i) {
        character.drawable[i]=static_cast<std::uint16_t>((std::max)(0,gta::Call<int>(gta::GET_PED_DRAWABLE_VARIATION,ped,i)));
        character.texture[i]=static_cast<std::uint8_t>((std::max)(0,gta::Call<int>(gta::GET_PED_TEXTURE_VARIATION,ped,i)));
    }
    const auto folder=std::filesystem::u8path(cache)/L".live"/L"ai-followers"/std::to_wstring(character.model_hash)/std::to_wstring(slot);
    std::error_code error;std::filesystem::create_directories(folder,error);
    if(error){Trace("setup_failed slot=%u ped=%d stage=cache_directory error=%d",slot,ped,error.value());return false;}
    FILE* file=_wfsopen((folder/L"skeleton.json").c_str(),L"wb",_SH_DENYWR);
    if(!file){Trace("setup_failed slot=%u ped=%d stage=skeleton_file_open errno=%d",slot,ped,errno);return false;}
    const bool written=std::fwrite(json.data(),1,json.size(),file)==json.size();
    if(std::fclose(file)!=0||!written){Trace("setup_failed slot=%u ped=%d stage=skeleton_file_write errno=%d",slot,ped,errno);return false;}
    const auto live=folder.u8string();std::string path(reinterpret_cast<const char*>(live.data()),live.size());
    character.live_skeleton_utf8=path.c_str();
    if(!runtime.FollowerStart(slot,&request)){Trace("setup_failed slot=%u ped=%d stage=runtime_request id=%u runtime_loaded=%d",slot,ped,request.id,runtime.IsLoaded());return false;}
    Trace("setup_queued slot=%u ped=%d id=%u live_skeleton=%s; background errors are in SkateVRuntime.log",slot,ped,request.id,path.c_str());
    auto& r=riders[slot];r.ped=ped;r.player=player;r.group=group;r.model=character.model_hash;r.id=request.id;
    r.started=GetTickCount();r.collision=!gta::Call<BOOL>(gta::GET_ENTITY_COLLISION_DISABLED,ped);
    r.ragdoll=gta::Call<BOOL>(gta::CAN_PED_RAGDOLL,ped)!=0;
    // Reserve the follower immediately, before slow asset/skeleton loading.
    // Waiting happens at its current position, without GTA group catch-up.
    r.preparing=true;
    if(GroupFollower(ped,group)) {
        gta::Call<void>(0x2E2F4240B3F24647ULL,ped,group,false);
        gta::Call<void>(0xED74007FFB146BC2ULL,ped);
        r.groupSuspended=true;
        Trace("group_suspended_for_loading ped=%d group=%d",ped,group);
    }
    gta::Call<void>(gta::FREEZE_ENTITY_POSITION,ped,true);
    return true;
}
SvVec3 GoalAt(const SvVec3& at,float spacing,float* catchup=nullptr) {
    if(trail.empty())return at;
    // Find the point behind the leader by route distance, then advance along
    // that same route from the follower. This preserves corners and detours.
    size_t limit=trail.size()-1;float walked=0.;
    while(limit>0&&walked<spacing) {walked+=Dist(trail[limit],trail[limit-1]);--limit;}
    size_t nearest=0;float best=1e9f;
    for(size_t i=0;i<=limit;++i) {const float d=Dist(at,trail[i]);if(d<best){best=d;nearest=i;}}
    size_t target=nearest;float look=0.;
    while(target<limit&&look<4.) {look+=Dist(trail[target],trail[target+1]);++target;}
    // Keep moving through intermediate points even if the leader has stopped
    // farther along the route. Arrival braking belongs at the final point.
    float remaining=0.;
    for(size_t i=target;i<limit;++i)remaining+=Dist(trail[i],trail[i+1]);
    if(catchup)*catchup=remaining*0.4f;
    return trail[target];
}
}
void InitDiagnostics(const std::wstring& directory) {
    if(diagnostics){std::fclose(diagnostics);diagnostics=nullptr;}
    const auto path=directory+L"SkateV-AI-Followers.log";
    MoveFileExW(path.c_str(),(directory+L"SkateV-AI-Followers.prev.log").c_str(),MOVEFILE_REPLACE_EXISTING);
    diagnostics=_wfsopen(path.c_str(),L"w",_SH_DENYNO);
    decisions.clear();summaryAt=0;managerStatus.clear();
    Trace("diagnostics_start build=1.2.6-ai-performance pid=%lu; candidate decisions log on change; scan/active heartbeat every 10 seconds",GetCurrentProcessId());
}
bool OwnsPed(int entity) {for(const auto& r:riders)if((r.held||r.preparing)&&r.ped==entity)return true;return false;}
void Stop(RuntimeBridge& runtime,const char* reason) {
    if(managerStatus!=reason){Trace("manager_inactive reason=%s",reason);managerStatus=reason;}
    if(leader)Trace("manager_stop player=%d reason=%s",leader,reason);
    for(unsigned i=0;i<4;++i)if(riders[i].ped)Release(i,runtime,reason);
    if(leader){decisions.clear();summaryAt=0;}
    trail.clear();leader=0;scanAt=0;
}
void Tick(int player,const Settings& cfg,RuntimeBridge& runtime,const std::string& cache,void (*log)(const char*)) {
    if(!cfg.enabled||!player||!runtime.IsLoaded()) {Stop(runtime,!cfg.enabled?"menu_disabled":!player?"no_player":"runtime_unloaded");return;}
    HostTiming timing;
    if(!leader)Trace("manager_start player=%d groups=%d missions=%d tricks=%d maximum=%d distance=%.1f runtime_loaded=%d",player,cfg.groups,cfg.missions,cfg.tricks,cfg.maximum,cfg.distance,runtime.IsLoaded());
    managerStatus="active";
    if(leader&&leader!=player)Stop(runtime);
    leader=player;
    const DWORD now=GetTickCount();
    const int group=gta::Call<int>(0xF162E133B4E7A675ULL,player);
    const auto position=gta::Call<Vector3>(gta::GET_ENTITY_COORDS,player,true);
    const SvVec3 here{position.x,position.y,position.z};
    if(!trail.empty()&&Dist(trail.back(),here)>35.) {
        trail.clear();Trace("leader_route_jump player=%d; followers keep ownership and catch up without teleport",player);
    }
    if(trail.empty()||Dist(trail.back(),here)>1.0f)trail.push_back(here);
    while(trail.size()>1024)trail.pop_front();
    SvOutput playerOutput{};
    const float speed=runtime.Output(playerOutput)?std::hypot(playerOutput.velocity.x,playerOutput.velocity.y):0.0f;
    const unsigned maximum=static_cast<unsigned>((std::clamp)(cfg.maximum,1,4));
    for(unsigned slot=0;slot<4;++slot) {
        auto& r=riders[slot];if(!r.ped)continue;
        const char* releaseReason=slot>=maximum?"maximum_reduced":!Exists(r)?"ped_deleted_or_model_changed":BusyReason(r.ped);
        // A carried group follower remains ours while its original group is
        // temporarily suspended; testing native membership here would release
        // it on the very next frame after the intended removal.
        const bool groupRole=cfg.groups&&((r.groupSuspended&&group==r.group)||GroupFollower(r.ped,group));
        if(!releaseReason&&!groupRole&&!(cfg.missions&&ScriptFollower(r.ped,player)))releaseReason="follower_role_lost";
        if(releaseReason){Release(slot,runtime,releaseReason);continue;}
        if(r.held) {
            const auto live=gta::Call<Vector3>(gta::GET_ENTITY_COORDS,r.ped,true);
            if(Dist({live.x,live.y,live.z},r.placed)>4.||std::abs(live.z-r.placed.z)>4.) {
                Trace("external_move ped=%d from=(%.2f,%.2f,%.2f) to=(%.2f,%.2f,%.2f) group_suspended=%d currently_member=%d",r.ped,r.placed.x,r.placed.y,r.placed.z,live.x,live.y,live.z,r.groupSuspended,GroupFollower(r.ped,group));
                Release(slot,runtime,"external_teleport");continue;
            }
        }
        static SvFollowerOutput output{};
        // The frozen ped and retained pose hook keep the last published pose.
        // Reapplying an identical snapshot every GTA frame repeats skeleton
        // probes, bone conversion and native placement without new motion.
        // Retry until the board is ready even if no new snapshot arrives.
        if(runtime.FollowerOutput(slot,output)&&output.id==r.id&&
            (!r.held||r.tick!=output.tick||!r.boardReady)) {
            if(!std::isfinite(output.root.x)||!std::isfinite(output.root.y)||!std::isfinite(output.root.z)) {Trace("invalid_output_wait slot=%u ped=%d",slot,r.ped);continue;}
            probe::SkeletonInfo sk;const char* why=nullptr;
            if(!probe::ResolvePedSkeleton(r.ped,sk,why)){Trace("pose_failed ped=%d reason=%s",r.ped,why?why:"unavailable");Release(slot,runtime,"live_skeleton_unavailable");continue;}
            if(output.bone_count!=static_cast<unsigned>(sk.count)){Trace("pose_failed ped=%d runtime_bones=%u gta_bones=%d",r.ped,output.bone_count,sk.count);Release(slot,runtime,"bone_count_mismatch");continue;}
            if(!posehook::InstallFollower(slot,reinterpret_cast<std::uintptr_t>(getScriptHandleBaseAddress(r.ped)),sk)){Release(slot,runtime,"pose_hook_rejected");continue;}
            if(!r.held) {
                r.held=true;
                r.preparing=false;
                if(GroupFollower(r.ped,group)) {
                    gta::Call<void>(0x2E2F4240B3F24647ULL,r.ped,group,false);
                    // The teleport flag alone did not stop GTA's automatic
                    // group catch-up from moving the simulation-owned ped.
                    // Suspend only this human's group role until release.
                    gta::Call<void>(0xED74007FFB146BC2ULL,r.ped); // REMOVE_PED_FROM_GROUP
                    r.groupSuspended=true;
                    Trace("group_suspended ped=%d group=%d remaining_member=%d",r.ped,group,GroupFollower(r.ped,group));
                }
                gta::Call<void>(gta::SET_ENTITY_COLLISION,r.ped,false,false);
                gta::Call<void>(gta::SET_PED_CAN_RAGDOLL,r.ped,false);
                gta::Call<void>(gta::FREEZE_ENTITY_POSITION,r.ped,true);
                if(log)log("AI skater follower acquired; group catch-up suspended, explicit tasks preserved");
                Trace("acquired slot=%u ped=%d id=%u tick=%llu bones=%u",slot,r.ped,r.id,static_cast<unsigned long long>(output.tick),output.bone_count);
            }
            if(r.tick!=output.tick) {r.tick=output.tick;r.outputAt=now;}
            gta::Call<void>(gta::SET_ENTITY_COORDS_NO_OFFSET,r.ped,output.root.x,output.root.y,output.root.z,false,false,false);
            r.placed=output.root;
            gta::Call<void>(gta::SET_ENTITY_HEADING,r.ped,output.heading);
            posehook::PublishFollower(slot,output.bones,output.bone_count);
            const bool boardReady=boardnative::TickFollower(slot,output.board);
            if(boardReady!=r.boardReady){Trace("board slot=%u ped=%d visible=%d",slot,r.ped,boardReady);r.boardReady=boardReady;}
        }
        // An absent/stalled snapshot never hands the ped back to teleporting
        // group AI. Preserve its last position until the session resumes or
        // the user leaves skating/disables the feature (Release restores it).
        if(!r.traceAt||now-r.traceAt>=10000){r.traceAt=now;Trace("session slot=%u ped=%d id=%u held=%d tick=%llu age_ms=%lu board=%d",slot,r.ped,r.id,r.held,static_cast<unsigned long long>(r.tick),now-r.started,r.boardReady);}
        if(now-r.goalAt>150) {
            r.goalAt=now;
            const auto p=gta::Call<Vector3>(gta::GET_ENTITY_COORDS,r.ped,true);
            SvFollowerGoal goal{};goal.size=sizeof(goal);goal.id=r.id;
            float catchup=0.;
            goal.point=GoalAt({p.x,p.y,p.z},cfg.distance+slot*2.5f,&catchup);
            goal.speed=(std::min)(11.0f,speed+catchup);goal.tricks=cfg.tricks;
            goal.current={p.x,p.y,p.z};
            if(!runtime.FollowerGoal(slot,goal))Trace("goal_rejected slot=%u ped=%d id=%u",slot,r.ped,r.id);
        }
    }
    if(static_cast<int>(now-scanAt)<0)return;
    scanAt=now+1000;
    int pool[1024]{};const int count=worldGetAllPeds(pool,1024);
    const bool summary=!summaryAt||now-summaryAt>=10000;
    if(summary){
        summaryAt=now;
        unsigned held=0,loading=0;
        for(const auto& r:riders){if(r.held)++held;else if(r.ped)++loading;}
        Trace("scan player=%d group=%d pool=%d held=%u loading=%u enabled=%d groups=%d missions=%d tricks=%d max=%u distance=%.1f",player,group,count,held,loading,cfg.enabled,cfg.groups,cfg.missions,cfg.tricks,maximum,cfg.distance);
    }
    for(unsigned slot=0;slot<maximum;++slot) {
        if(riders[slot].ped||static_cast<int>(now-cooldown[slot])<0)continue;
        int chosen=0;float best=30.0f;
        for(int i=0;i<count;++i) {
            const int ped=pool[i];
            // Human-only is unconditional: cats and other animals cannot enter
            // this manager, irrespective of group/mission/menu settings.
            if(ped==player||!gta::Call<BOOL>(gta::DOES_ENTITY_EXIST,ped)||!gta::Call<BOOL>(0xB980061DA992779DULL,ped))continue;
            const auto p=gta::Call<Vector3>(gta::GET_ENTITY_COORDS,ped,true);
            const float d=Dist({p.x,p.y,p.z},here);
            const bool groupMember=GroupFollower(ped,group);
            const bool scriptFollower=ScriptFollower(ped,player);
            bool already=false;for(const auto& r:riders)if(r.ped==ped)already=true;
            const char* reason=gta::Call<BOOL>(0x12534C348C6CB68BULL,ped)?"is_player":BusyReason(ped);
            if(!reason&&gta::Call<BOOL>(gta::IS_ENTITY_ATTACHED,ped))reason="attached";
            if(!reason&&gta::Call<BOOL>(0x1218E6886D3D8327ULL,ped))reason="static_or_frozen";
            if(!reason&&already)reason="already_managed";
            if(!reason&&!(cfg.groups&&groupMember)&&!(cfg.missions&&scriptFollower))reason="no_enabled_follower_role";
            if(!reason&&(d>=30.0f||std::abs(p.z-here.z)>=4.))reason="outside_acquisition_range";
            if(d<40.||groupMember){
                const std::string decision=std::to_string(gta::Call<unsigned>(gta::GET_ENTITY_MODEL,ped))+":"+(reason?reason:"eligible")+":"+std::to_string(groupMember)+":"+std::to_string(scriptFollower);
                if(summary||decisions[ped]!=decision){
                    decisions[ped]=decision;
                    Trace("candidate ped=%d model=%08x distance=%.1f height_delta=%.1f group_member=%d mission_follow=%d decision=%s task_follow=%d task_goto=%d task_enter_vehicle=%d",ped,gta::Call<unsigned>(gta::GET_ENTITY_MODEL,ped),d,p.z-here.z,groupMember,scriptFollower,reason?reason:"eligible",gta::Call<int>(gta::GET_SCRIPT_TASK_STATUS,ped,util::Joaat("SCRIPT_TASK_FOLLOW_TO_OFFSET_OF_ENTITY")),gta::Call<int>(gta::GET_SCRIPT_TASK_STATUS,ped,util::Joaat("SCRIPT_TASK_GO_TO_ENTITY")),gta::Call<int>(gta::GET_SCRIPT_TASK_STATUS,ped,util::Joaat("SCRIPT_TASK_ENTER_VEHICLE")));
                }
            }
            if(reason)continue;
            if(d<best&&std::abs(p.z-here.z)<4.) {chosen=ped;best=d;}
        }
        if(chosen) {
            Trace("selected slot=%u ped=%d distance=%.1f",slot,chosen,best);
            if(!Start(slot,chosen,player,group,runtime,cache)){cooldown[slot]=now+5000;Trace("retry_delayed slot=%u ped=%d delay_ms=5000",slot,chosen);}
            break; // One session setup per scan, so player input remains responsive.
        }
    }
}
}
