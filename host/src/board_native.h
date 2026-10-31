#pragma once
#include <cstdint>
#include <string>
#include "skatev_runtime.h"
#include "board_pose_diagnostics.h"
namespace boardnative {
// Call on the ScriptHookV script thread, after the Legacy Story Mode guard.
bool Start(const std::wstring& iniPath,const std::wstring& dataRoot,void (*log)(const char*));
// True only when the native custom object exists and all seven bones were posed.
bool Tick(bool skating,const SvBoardPose& pose);
// Put away only: delete presentation on the script thread, retain the
// registered resources and loaded archetype/model for the next invocation.
void Stop();
int Entity(); // Exclude this presentation-only entity from ExternalQueries.
// Writes the seven board bones of `object` (a drawable with the native board's
// skeleton) from `pose`. `cache` keeps its bone indices; one per object.
// failed: why the last pose was not written (logged once per object, rc9:
// "the wheels do not roll" while only one object's diagnostic was logged).
struct PoseCache { std::uintptr_t skeleton = 0; int indices[7] = {}; bool reported = false; diagnostics::State diagnostic;
                   const char* failed = nullptr; unsigned long long logAt = 0; };
bool PoseObject(int object, const SvBoardPose& pose, PoseCache& cache, void (*log)(const char*));
// The room GTA itself keeps the board object in, logged when it changes (at
// most 40 lines per object). Read only: GTA places and lights the board.
struct Room { int interior = 0; std::uint32_t room = 0; };
struct RoomCache { Room last{-1, 0}; int logs = 0; };
void LogRoom(int object, RoomCache& cache, const char* who, void (*log)(const char*));
// Gives a board object next to the player (same room, within 4 m) GTA's ambient
// light scales of that spot, which GTA leaves at full for our objects.
void ShareAmbient(int object, void (*log)(const char*));
// Glowing wheels (the maintainer's wish 2026-10-17): `strength` 0 (off) .. 1, colour 0..255;
// r < 0: the ridden wheels' own colour (SetWheelColour).
void SetWheelGlow(float strength, int r, int g, int b);
// The ridden wheels' own colour (shop; white without one), used when the
// glow colour is "Wheel colour".
void SetWheelColour(int r, int g, int b);
const int* WheelColour();
// A small GTA light just outside each wheel of `pose` this frame, so the wheel
// faces and the ground under them glow. Same for every board state, since the
// composed single drawable has no separate wheel object to light.
void GlowWheels(const SvBoardPose& pose);
// Puts a board object at the pose (frozen, as SK8V does); PoseObject then writes the bones.
void Place(int object, const SvBoardPose& pose, float qx, float qy, float qz, float qw);
}
