#pragma once
// The replay's fisheye lens (design 2026-11-09-replay-look-sound): GTA
// renders the replay camera at 120 degrees and this pass bends the finished
// frame into a full-frame fisheye (equidistant mapping, the corners keep the
// whole picture). Render thread, ScriptHookV's present callback, before the
// HUD overlay; every D3D11 state it touches is restored.
#include <windows.h>
#include <d3d11.h>
#include <dxgi.h>

namespace fisheye {
using LogFn = void (*)(const char*);
void SetLog(LogFn log);
// Any thread: draw the pass on the following presents.
void SetEnabled(bool on);
bool Enabled();
// Render thread: bends the backbuffer when enabled. After a GPU failure it
// logs once and stays off (the plain 120-degree view remains).
void Present(IDXGISwapChain* swapChain);
// Releases every GPU object (device change, shutdown).
void Release();
// The source point (pixels of a w x h frame) the output pixel (x, y) shows,
// for a source rendered at `vfovDegrees` vertical field of view. Exposed for
// the tests; the shader does the same.
void SourcePoint(float x, float y, float w, float h, float vfovDegrees, float& sx, float& sy);
}  // namespace fisheye
