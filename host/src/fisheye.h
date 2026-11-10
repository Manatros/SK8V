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
#include <vector>

namespace fisheye {
using LogFn = void (*)(const char*);
void SetLog(LogFn log);
// Any thread: draw the pass on the following presents.
void SetEnabled(bool on);
bool Enabled();
// Enabled, not failed, and a bent frame was drawn in the last half second:
// only then is the replay HUD drawn by this overlay instead of by GTA.
bool Drawing();
// The replay HUD as items (screen 0-1, as GTA's DRAW_RECT and text): solid
// rectangles (straight colour, 0-1) and white texts (`y` their top; `scale`
// GTA's text scale). Texts are drawn in two bands, the top fifth and the
// bottom fifth of the screen (the timeline strip and the status/keys).
struct HudRect {
    float x0, y0, x1, y1;
    float r, g, b, a;
};
struct HudText {
    std::string text;
    float x, y, scale;
    bool centre;
    bool operator==(const HudText& o) const { return text == o.text && x == o.x && y == o.y && scale == o.scale && centre == o.centre; }
};
// Script thread, every replay frame: drawn over the bent picture while the
// pass runs (`show` false: none). Each band's text is painted again only when
// its texts change; rectangles are drawn every frame.
void SetHudItems(const std::vector<HudRect>& rects, const std::vector<HudText>& texts, bool show);
// Render thread: bends the backbuffer and draws the HUD when enabled. After a
// lasting GPU failure it logs once and stays off (the plain wide view remains).
void Present(IDXGISwapChain* swapChain);
// Releases every GPU object (device change, shutdown).
void Release();

// Skate's "Take Photo" (design 2026-11-09-skate-replay-editor P2c). Script
// thread: Request asks for the frame presented a few frames from now (the
// caller hides its HUD meanwhile) to be written as a PNG to `path`; State is
// 0 idle, 1 waiting or writing, 2 saved, 3 failed; Reset makes it idle again.
// The picture is taken after the fisheye bend and before our HUD.
void RequestPhoto(const std::wstring& path);
int PhotoState();
void ResetPhoto();
// A request not taken yet is dropped (leaving the replay, a timeout).
void CancelPhoto();
// The PNG writer (WIC), 24-bit BGR rows top-down; also used by the tests.
bool WritePng(const std::wstring& path, const unsigned char* bgr, unsigned width, unsigned height);
// The source point (pixels of a w x h frame) the output pixel (x, y) shows,
// for a source rendered at `vfovDegrees` vertical field of view. Exposed for
// the tests; the shader does the same.
void SourcePoint(float x, float y, float w, float h, float vfovDegrees, float& sx, float& sy);
}  // namespace fisheye
