#pragma once
#include <string>
class RuntimeBridge;
namespace aifollowers {
void InitDiagnostics(const std::wstring& logDirectory);
struct Settings {
    bool enabled=false, groups=true, missions=true, tricks=true;
    int maximum=2;
    float distance=6.0f;
};
void Tick(int player, const Settings& settings, RuntimeBridge& runtime, const std::string& pedCache, void (*log)(const char*));
void Stop(RuntimeBridge& runtime, const char* reason="left active Skate mode");
bool OwnsPed(int entity);
}
