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

    {
        // Take Photo's PNG writer: a 4 x 2 picture becomes a PNG file.
        wchar_t tmp[MAX_PATH];
        GetTempPathW(MAX_PATH, tmp);
        const std::wstring path = std::wstring(tmp) + L"gtaxskate-photo-test.png";
        const unsigned char bgr[4 * 2 * 3] = {255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 0, 0, 0, 10, 20, 30, 40, 50, 60, 70, 80, 90};
        assert(fisheye::WritePng(path, bgr, 4, 2));
        FILE* f = _wfopen(path.c_str(), L"rb");
        assert(f);
        unsigned char sig[8] = {};
        assert(std::fread(sig, 1, 8, f) == 8);
        std::fclose(f);
        assert(sig[0] == 0x89 && sig[1] == 'P' && sig[2] == 'N' && sig[3] == 'G');
        DeleteFileW(path.c_str());
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
    fisheye::SetHudItems({{0.2f, 0.901f, 0.8f, 0.909f, 0.08f, 0.08f, 0.08f, 0.7f}, {0.2f, 0.901f, 0.5f, 0.909f, 1.0f, 0.78f, 0.16f, 0.9f},
                          {0.12f, 0.28f, 0.84f, 0.45f, 0.0f, 0.0f, 0.0f, 1.0f}},
                         {{"Clip 1", 0.5f, 0.07f, 0.34f, true}, {"Look Settings", 0.5f, 0.31f, 0.48f, true},
                          {"INSTANT REPLAY  1x  Follow", 0.5f, 0.918f, 0.34f, true}}, true);
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
    bool menuTextVisible = false;
    for (int y = 45; y < 62; ++y)
        for (int x = 78; x < 178; ++x)
            menuTextVisible = menuTextVisible || (at(x, y) & 0xFF) > 160;
    assert(menuTextVisible);  // white glyphs over an opaque black panel
    ctx->Unmap(staging, 0);

    {
        // A width-only resize must replace all three textures independently.
        for (int band = 0; band < fisheye::kBandCount; ++band) {
            assert(fisheye::UpdateBand(band, fisheye::g_hud, 320, 144));
            D3D11_TEXTURE2D_DESC bandDesc{};
            fisheye::gpu.hud[band]->GetDesc(&bandDesc);
            assert(bandDesc.Width == 320);
        }
        // A capture frame still bends the picture but omits even a stale visible HUD.
        ctx->UpdateSubresource(back, 0, nullptr, px.data(), 256 * 4, 0);
        fisheye::Bend(sc, false);
        ctx->CopyResource(staging, back);
        assert(SUCCEEDED(ctx->Map(staging, 0, D3D11_MAP_READ, 0, &m)));
        assert(at(108, 72) == mid);
        assert(at(60, 130) != (bar & 0xFFFFFFu));
        ctx->Unmap(staging, 0);
    }

    {
        // The detail render: the 60-degree view (here all red) is kept, the screen holds the
        // 120-degree frame meanwhile, and afterwards the middle comes from the detail view
        // while the edges still come from the 120-degree frame.
        fisheye::SetHudItems({}, {}, false);
        std::vector<std::uint32_t> red(256 * 144, 0xFF0000FFu);  // R8G8B8A8: red
        fisheye::BeginDetail();
        fisheye::ArmDetail();  // the script saw GTA render at the detail FOV
        for (int i = 0; i < 6 && !fisheye::DetailCaptured(); ++i) {
            ctx->UpdateSubresource(back, 0, nullptr, red.data(), 256 * 4, 0);
            fisheye::Present(sc);
        }
        assert(fisheye::DetailCaptured());
        fisheye::UseDetail();
        for (int i = 0; i < 5; ++i) {
            ctx->UpdateSubresource(back, 0, nullptr, px.data(), 256 * 4, 0);  // the 120-degree view again
            fisheye::Present(sc);
        }
        assert(fisheye::DetailReady());
        ctx->CopyResource(staging, back);
        D3D11_MAPPED_SUBRESOURCE d2{};
        ctx->Map(staging, 0, D3D11_MAP_READ, 0, &d2);
        const auto* q = static_cast<const std::uint32_t*>(d2.pData);
        const std::uint32_t mid = q[72 * (d2.RowPitch / 4) + 128] & 0xFFFFFFu, corner = q[2 * (d2.RowPitch / 4) + 2] & 0xFFFFFFu;
        ctx->Unmap(staging, 0);
        assert((mid & 0xFF) > 200 && ((mid >> 8) & 0xFF) < 40);   // red: the detail view
        assert(!((corner & 0xFF) > 200 && ((corner >> 8) & 0xFF) < 40));  // the corner is not
        fisheye::DropDetail();
        assert(!fisheye::DetailReady());
    }

    {
        // Filtered detail render: the 120-degree view supplies the low-frequency colour
        // and vignette (here uniform mid-grey 128), while the 60-degree view has a dark
        // vignette offset (base 60) plus a sharp bright stripe at the centre (x = 128).
        // With SetFiltered(true), a smooth region inside the detail window keeps the
        // 120-degree grey (~128, not 60), while the centre stripe transfers its contrast.
        fisheye::SetFiltered(true);
        assert(!fisheye::DetailIdle());
        std::vector<std::uint32_t> wideGrey(256 * 144, 0xFF808080u);
        ctx->UpdateSubresource(back, 0, nullptr, wideGrey.data(), 256 * 4, 0);
        fisheye::Present(sc);
        assert(fisheye::DetailIdle());

        std::vector<std::uint32_t> detailFiltered(256 * 144, 0xFF3C3C3Cu);  // darker base (60)
        for (int y = 0; y < 144; ++y) detailFiltered[y * 256 + 128] = 0xFFDCDCDCu;  // bright 1 px stripe (220)
        fisheye::BeginDetail();
        fisheye::ArmDetail();
        for (int i = 0; i < 6 && !fisheye::DetailCaptured(); ++i) {
            ctx->UpdateSubresource(back, 0, nullptr, detailFiltered.data(), 256 * 4, 0);
            fisheye::Present(sc);
        }
        assert(fisheye::DetailCaptured());
        fisheye::UseDetail();
        for (int i = 0; i < 5; ++i) {
            ctx->UpdateSubresource(back, 0, nullptr, wideGrey.data(), 256 * 4, 0);
            fisheye::Present(sc);
        }
        assert(fisheye::DetailReady());
        ctx->CopyResource(staging, back);
        D3D11_MAPPED_SUBRESOURCE d3{};
        ctx->Map(staging, 0, D3D11_MAP_READ, 0, &d3);
        const auto* q = static_cast<const std::uint32_t*>(d3.pData);
        const int smooth = static_cast<int>(q[72 * (d3.RowPitch / 4) + 110] & 0xFFu);
        const int stripe = static_cast<int>(q[72 * (d3.RowPitch / 4) + 128] & 0xFFu);
        ctx->Unmap(staging, 0);
        assert(smooth >= 118 && smooth <= 138);  // keeps the 120-degree frame's 128 level, not 60
        assert(stripe > smooth + 40);            // transfers the sharp stripe from the 60-degree view
        fisheye::DropDetail();
        fisheye::SetFiltered(false);
    }

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
