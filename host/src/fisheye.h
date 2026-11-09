#pragma once
// The replay's fisheye lens and its HUD (design 2026-11-09-replay-look-sound).
// GTA renders the replay camera at 120 degrees and this pass bends the
// finished frame into a full-frame equidistant fisheye (the frame's corners
// keep the source's corners). Catmull-Rom sampling keeps the enlarged middle
// sharp; a light vignette darkens the corners as a real lens does.
// Afterwards the replay's status line and timeline are drawn over it unbent
// (GDI text in a texture), so the HUD never bends with the picture.
// Render thread, ScriptHookV's present callback, before the HUD overlay;
// every D3D11 state it touches is restored.
#include <windows.h>
#include <d3d11.h>
#include <dxgi.h>
#include <string>

namespace fisheye {
using LogFn = void (*)(const char*);
void SetLog(LogFn log);
// Any thread: draw the pass on the following presents.
void SetEnabled(bool on);
bool Enabled();
// Enabled, not failed, and a bent frame was drawn in the last half second:
// only then is the replay HUD drawn by this overlay instead of by GTA.
bool Drawing();
// Script thread: the replay HUD drawn over the bent picture (only while
// enabled): the status text (lines split by "~n~", GTA's newline) and the
// timeline's filled fraction (0-1). `show` false: none (hidden HUD). The
// text is painted again only when it changes; the timeline is drawn as
// solid rectangles every frame.
void SetHud(const std::string& text, float progress, bool show);
// Render thread: bends the backbuffer and draws the HUD when enabled. After a
// lasting GPU failure it logs once and stays off (the plain wide view remains).
void Present(IDXGISwapChain* swapChain);
// Releases every GPU object (device change, shutdown).
void Release();
// The source point (pixels of a w x h frame) the output pixel (x, y) shows,
// for a source rendered at `vfovDegrees` vertical field of view. Exposed for
// the tests; the shader does the same.
void SourcePoint(float x, float y, float w, float h, float vfovDegrees, float& sx, float& sy);
}  // namespace fisheye
