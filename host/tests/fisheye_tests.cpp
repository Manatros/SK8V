// The replay's fisheye mapping (design 2026-11-09-replay-look-sound), and the
// pass itself on a WARP device: the centre stays, the frame's corners show
// the source's corners, and GTA's pipeline state is left as it was.
#include "../src/fisheye.cpp"
#include <cassert>
#include <cmath>
#include <cstdint>
#include <iostream>
#include <vector>

int main() {
    // Mapping: centre fixed, corners onto corners, the middle magnified (barrel).
    float sx, sy;
    fisheye::SourcePoint(960, 540, 1920, 1080, 120, sx, sy);
    assert(std::fabs(sx - 960) < 0.01f && std::fabs(sy - 540) < 0.01f);
    fisheye::SourcePoint(0, 0, 1920, 1080, 120, sx, sy);
    assert(std::fabs(sx) < 0.5f && std::fabs(sy) < 0.5f);
    fisheye::SourcePoint(1920, 1080, 1920, 1080, 120, sx, sy);
    assert(std::fabs(sx - 1920) < 0.5f && std::fabs(sy - 1080) < 0.5f);
    fisheye::SourcePoint(1060, 540, 1920, 1080, 120, sx, sy);
    assert(sx > 960 && sx < 1060);  // near the centre the picture is enlarged
    for (int x = 0; x <= 1920; x += 64)  // every source point stays inside the frame
        for (int y = 0; y <= 1080; y += 60) {
            fisheye::SourcePoint(static_cast<float>(x), static_cast<float>(y), 1920, 1080, 120, sx, sy);
            assert(sx >= -0.5f && sx <= 1920.5f && sy >= -0.5f && sy <= 1080.5f);
        }

    // The pass on WARP with a swap chain on a hidden window.
    WNDCLASSW wc{};
    wc.lpfnWndProc = DefWindowProcW;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = L"fisheye_test";
    RegisterClassW(&wc);
    HWND hwnd = CreateWindowW(L"fisheye_test", L"", WS_OVERLAPPEDWINDOW, 0, 0, 256, 144, nullptr, nullptr, wc.hInstance, nullptr);
    DXGI_SWAP_CHAIN_DESC sd{};
    sd.BufferCount = 1;
    sd.BufferDesc.Width = 256;
    sd.BufferDesc.Height = 144;
    sd.BufferDesc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    sd.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
    sd.OutputWindow = hwnd;
    sd.SampleDesc.Count = 1;
    sd.Windowed = TRUE;
    IDXGISwapChain* sc = nullptr;
    ID3D11Device* dev = nullptr;
    ID3D11DeviceContext* ctx = nullptr;
    HRESULT hr = D3D11CreateDeviceAndSwapChain(nullptr, D3D_DRIVER_TYPE_WARP, nullptr, 0, nullptr, 0, D3D11_SDK_VERSION, &sd, &sc,
                                               &dev, nullptr, &ctx);
    if (FAILED(hr)) {
        std::cout << "fisheye tests passed (mapping only: no WARP device)\n";
        return 0;
    }
    ID3D11Texture2D* back = nullptr;
    sc->GetBuffer(0, __uuidof(ID3D11Texture2D), reinterpret_cast<void**>(&back));
    // A checkerboard of 16 px squares, white on black.
    std::vector<std::uint32_t> px(256 * 144);
    for (int y = 0; y < 144; ++y)
        for (int x = 0; x < 256; ++x) px[y * 256 + x] = ((x / 16 + y / 16) & 1) ? 0xFFFFFFFFu : 0xFF000000u;
    ctx->UpdateSubresource(back, 0, nullptr, px.data(), 256 * 4, 0);
    // GTA's state the pass must leave alone: a viewport and a render target.
    ID3D11RenderTargetView* rtv = nullptr;
    dev->CreateRenderTargetView(back, nullptr, &rtv);
    ctx->OMSetRenderTargets(1, &rtv, nullptr);
    D3D11_VIEWPORT vp{3, 4, 50, 60, 0, 1};
    ctx->RSSetViewports(1, &vp);

    fisheye::SetEnabled(true);
    // The replay timeline as the replay sends it: dark bar, yellow fill to 0.5, a line of text.
    fisheye::SetHudItems({{0.2f, 0.901f, 0.8f, 0.909f, 0.08f, 0.08f, 0.08f, 0.7f}, {0.2f, 0.901f, 0.5f, 0.909f, 1.0f, 0.78f, 0.16f, 0.9f}},
                         {{"INSTANT REPLAY  1x  Follow", 0.5f, 0.918f, 0.34f, true}}, true);
    fisheye::Present(sc);
    assert(!fisheye::g_failed.load());

    UINT n = 1;
    D3D11_VIEWPORT got{};
    ctx->RSGetViewports(&n, &got);
    assert(n == 1 && got.TopLeftX == 3 && got.Width == 50);
    ID3D11RenderTargetView* bound = nullptr;
    ctx->OMGetRenderTargets(1, &bound, nullptr);
    assert(bound == rtv);
    bound->Release();

    // Read back: the centre square keeps its colour (x 136: inside the square right of the centre line), the corner pixel too.
    D3D11_TEXTURE2D_DESC d{};
    back->GetDesc(&d);
    d.Usage = D3D11_USAGE_STAGING;
    d.BindFlags = 0;
    d.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    ID3D11Texture2D* staging = nullptr;
    dev->CreateTexture2D(&d, nullptr, &staging);
    ctx->CopyResource(staging, back);
    D3D11_MAPPED_SUBRESOURCE m{};
    ctx->Map(staging, 0, D3D11_MAP_READ, 0, &m);
    auto at = [&](int x, int y) { return static_cast<const std::uint32_t*>(m.pData)[y * (m.RowPitch / 4) + x] & 0xFFFFFFu; };
    assert(at(136, 72) == (px[72 * 256 + 136] & 0xFFFFFFu));
    assert(at(1, 1) == (px[1 * 256 + 1] & 0xFFFFFFu));
    // Barrel: the square left of centre (x 112..127) is wider after the pass,
    // so x 108 (black before: square 6 at row 4) is now still inside it.
    const std::uint32_t mid = px[72 * 256 + 120] & 0xFFFFFFu;
    assert((px[72 * 256 + 108] & 0xFFFFFFu) != mid && at(108, 72) == mid);
    // The HUD is drawn after the bend, unbent: the timeline's filled (yellow)
    // part at y 0.901-0.909 (row 130), x 0.2-0.5, the empty (dark) part right of it.
    const std::uint32_t bar = static_cast<const std::uint32_t*>(m.pData)[130 * (m.RowPitch / 4) + 60];
    const std::uint32_t r = bar & 0xFF, g = (bar >> 8) & 0xFF, b = (bar >> 16) & 0xFF;  // R8G8B8A8
    assert(r > 150 && g > 100 && b < 120);
    const std::uint32_t empty = static_cast<const std::uint32_t*>(m.pData)[130 * (m.RowPitch / 4) + 180];
    assert((empty & 0xFF) < 120 && ((empty >> 8) & 0xFF) < 120);  // 70 % dark over at most white
    ctx->Unmap(staging, 0);

    fisheye::SetEnabled(false);
    fisheye::Release();
    staging->Release();
    rtv->Release();
    back->Release();
    ctx->Release();
    sc->Release();
    dev->Release();
    std::cout << "fisheye tests passed\n";
    return 0;
}
