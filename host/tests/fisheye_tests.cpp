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

    // The replay's status line under the fisheye (replay::StatusY 0.68): GTA's
    // y 0.68 in the middle column shows near the bottom (output y 0.90-0.96).
    fisheye::SourcePoint(960, 1080 * 0.93f, 1920, 1080, 120, sx, sy);
    assert(std::fabs(sy / 1080 - 0.68f) < 0.02f);
    // A short line (source x 0.35-0.65 on that row) stays inside the picture.
    fisheye::SourcePoint(0, 1080 * 0.93f, 1920, 1080, 120, sx, sy);
    assert(sx / 1920 < 0.35f);

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
