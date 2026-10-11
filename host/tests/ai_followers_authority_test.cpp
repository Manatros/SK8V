// Exercise the actual host manager with simulated GTA natives: real physics
// already has a separate asset simulation. This covers the group handover
// that failed in gameplay, including restoration and mission task ownership.
#include <windows.h>
#include <string>
#include <cassert>
#include <cstring>
#define private public
#include "../src/runtime_bridge.h"
#undef private
#include "../src/ai_followers.cpp"

namespace {
struct Game {
    bool member=true,frozen=false,collision=true,ragdoll=true,mission=false,output=true;
    int group=2,removed=0,restored=0,cleared=0,stopped=0,placements=0,published=0;
    bool fixedTick=false;
    SvVec3 position{},outputPosition{},leaderPosition{};
} game;
UINT64 hash=0,args[16]{},result[4]{};
unsigned argc=0;
std::uint64_t tick=6;
}
void nativeInit(UINT64 h){hash=h;argc=0;std::memset(args,0,sizeof(args));}
void nativePush64(UINT64 a){assert(argc<16);args[argc++]=a;}
PUINT64 nativeCall(){
    std::memset(result,0,sizeof(result));
    switch(hash){
    case gta::DOES_ENTITY_EXIST: result[0]=1;break;
    case gta::GET_ENTITY_MODEL: result[0]=0x9b22dbaf;break;
    case gta::GET_ENTITY_COORDS: {
        Vector3 p{};
        if(args[0]==42){p.x=game.position.x;p.y=game.position.y;p.z=game.position.z;}
        if(args[0]==1){p.x=game.leaderPosition.x;p.y=game.leaderPosition.y;p.z=game.leaderPosition.z;}
        std::memcpy(result,&p,sizeof(p));break;
    }
    case gta::GET_SCRIPT_TASK_STATUS: result[0]=game.mission&&args[1]==util::Joaat("SCRIPT_TASK_FOLLOW_TO_OFFSET_OF_ENTITY")?1:7;break;
    case gta::GET_GROUND_Z_FOR_3D_COORD: *reinterpret_cast<float*>(args[3])=0.f;result[0]=1;break;
    case gta::CAN_PED_RAGDOLL: result[0]=1;break;
    case 0xF162E133B4E7A675ULL: result[0]=game.group;break;
    case 0x9BB01E3834671191ULL: result[0]=game.member;break;
    case 0x5891CAC5D4ACFF74ULL: result[0]=game.member;break;
    case 0xED74007FFB146BC2ULL: game.member=false;++game.removed;break;
    case 0x9F3480FE65DB31B5ULL: game.member=true;++game.restored;break;
    case 0x0A7B270912999B3CULL: result[0]=game.mission;break;
    case gta::FREEZE_ENTITY_POSITION: game.frozen=args[1]!=0;break;
    case gta::SET_ENTITY_COLLISION: game.collision=args[1]!=0;break;
    case gta::SET_PED_CAN_RAGDOLL: game.ragdoll=args[1]!=0;break;
    case gta::SET_ENTITY_COORDS_NO_OFFSET:
        ++game.placements;
        std::memcpy(&game.position.x,&args[1],4);std::memcpy(&game.position.y,&args[2],4);std::memcpy(&game.position.z,&args[3],4);break;
    case gta::CLEAR_PED_TASKS_IMMEDIATELY: ++game.cleared;break;
    }
    return result;
}
int worldGetAllPeds(int*,int){return 0;}
BYTE* getScriptHandleBaseAddress(int){return reinterpret_cast<BYTE*>(1);}
bool probe::ResolvePedSkeleton(int,probe::SkeletonInfo& sk,const char*&){sk.count=317;return true;}
bool probe::SkeletonJson(const probe::SkeletonInfo&,std::string& json){json="{}";return true;}
bool posehook::InstallFollower(unsigned,std::uintptr_t,const probe::SkeletonInfo&){return true;}
void posehook::RemoveFollower(unsigned){}
void posehook::PublishFollower(unsigned,const float*,int){++game.published;}
bool boardnative::TickFollower(unsigned,const SvBoardPose&){return true;}
void boardnative::StopFollower(unsigned){}
bool RuntimeBridge::FollowerStart(unsigned,const SvFollowerStart* request){if(!request)++game.stopped;return true;}
bool RuntimeBridge::FollowerGoal(unsigned,const SvFollowerGoal&){return true;}
bool RuntimeBridge::FollowerOutput(unsigned,SvFollowerOutput& out)const{out={};out.size=sizeof(out);out.id=7;out.bone_count=317;out.tick=game.fixedTick?tick:tick++;out.root=game.outputPosition;return game.output;}
bool RuntimeBridge::Output(SvOutput& out)const{out={};return true;}

void prepare(RuntimeBridge& runtime,bool member=true){
    game={};game.member=member;
    runtime.runtime_=reinterpret_cast<void*>(1);
    for(auto& r:aifollowers::riders)r={};
    aifollowers::leader=0;aifollowers::trail.clear();
    auto& r=aifollowers::riders[0];r.ped=42;r.player=1;r.group=2;r.model=0x9b22dbaf;r.id=7;
    r.started=GetTickCount();r.collision=true;r.ragdoll=true;
}
int main(){
    // Intermediate route points must retain catch-up speed, including when
    // the player has stopped farther ahead. The final point must allow a stop.
    for(int i=0;i<=40;++i)aifollowers::trail.push_back({float(i),0.f,0.f});
    float catchup=0.f;
    const auto intermediate=aifollowers::GoalAt({0.f,0.f,0.f},6.f,&catchup);
    assert(intermediate.x==4.f&&catchup>10.f);
    const auto arrival=aifollowers::GoalAt({32.f,0.f,0.f},6.f,&catchup);
    assert(arrival.x==34.f&&catchup==0.f);
    RuntimeBridge runtime;
    const aifollowers::Settings enabled{true,true,true,true,2,6.f};
    prepare(runtime);game.fixedTick=true;
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(game.placements==1&&game.published==1);
    for(int i=0;i<20;++i)aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(game.placements==1&&game.published==1&&aifollowers::OwnsPed(42));
    ++tick;aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(game.placements==2&&game.published==2);
    aifollowers::Stop(runtime);
    prepare(runtime);aifollowers::riders[0]={};game.output=false;
    assert(aifollowers::Start(0,42,1,2,runtime,"ai-test-cache"));
    assert(game.removed==1&&!game.member&&game.frozen&&aifollowers::OwnsPed(42));
    aifollowers::riders[0].started=GetTickCount()-30000;
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(game.stopped==0&&!game.member&&aifollowers::OwnsPed(42));
    aifollowers::Stop(runtime);
    assert(game.restored==1&&game.member&&!game.frozen);
    prepare(runtime);
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(game.removed==1&&!game.member&&game.frozen&&!game.collision);
    assert(aifollowers::OwnsPed(42));
    // Before the fix, suspended native membership made a retained follower
    // fail eligibility here and resume teleport following on the next frame.
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(aifollowers::OwnsPed(42)&&game.stopped==0&&game.removed==1);
    game.outputPosition={100.f,0.f,0.f};
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(aifollowers::OwnsPed(42)&&game.restored==0); // no distant catch-up teleport
    game.output=false;
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(aifollowers::OwnsPed(42)&&game.restored==0); // no missing-output teleport
    game.output=true;
    game.leaderPosition={200.f,0.f,0.f};
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(aifollowers::OwnsPed(42)&&game.restored==0); // no route-jump catch-up teleport
    auto disabled=enabled;disabled.groups=false;disabled.missions=false;
    aifollowers::Tick(1,disabled,runtime,"",nullptr);
    assert(!aifollowers::OwnsPed(42)&&game.restored==1&&game.member);
    assert(!game.frozen&&game.collision&&game.ragdoll&&game.cleared==0);

    prepare(runtime);aifollowers::Tick(1,enabled,runtime,"",nullptr);
    aifollowers::Stop(runtime);
    assert(game.restored==1&&game.member&&!game.frozen);

    prepare(runtime);aifollowers::Tick(1,enabled,runtime,"",nullptr);
    game.group=3;aifollowers::Stop(runtime);
    assert(game.restored==0); // Do not move a mission actor into a new group.

    prepare(runtime,false);game.mission=true;
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(aifollowers::OwnsPed(42)&&game.removed==0);
    game.position.x=10.f;
    aifollowers::Tick(1,enabled,runtime,"",nullptr);
    assert(!aifollowers::OwnsPed(42)&&game.cleared==0&&game.restored==0);
    std::puts("PASS: loading reservation/restoration, distant/stalled/route-jump ownership, group handover, option/exit restoration, changed-group guard, and mission teleport release");
}
