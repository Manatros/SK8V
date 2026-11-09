#include "fisheye.h"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <vector>

#include "d3d11_saved_state.h"
#include "host_util.h"

namespace fisheye {
namespace {
using d3dstate::SafeRelease;

constexpr float kSourceFov = 120.0f;  // the replay camera's vertical FOV for the fisheye lens
// The text band (fractions of the frame height), below the timeline at 0.905.
constexpr float kBandTop = 0.915f, kBandBottom = 0.985f;
constexpr float kTimelineY = 0.905f;

std::atomic<bool> g_enabled{false};
std::atomic<bool> g_failed{false};
std::atomic<std::uint64_t> g_drawnAt{0};  // GetTickCount64 of the last bent frame
std::atomic<float> g_progress{0.0f};      // the timeline (every frame, no repaint)
LogFn g_log = nullptr;

// The HUD as the script thread last set it.
struct Hud {
    std::string text;
    float progress = 0.0f;
    bool show = false;
    std::uint64_t serial = 0;
};
std::mutex g_hudLock;
Hud g_hud;

struct Gpu {
    ID3D11Device* device = nullptr;
    ID3D11DeviceContext* ctx = nullptr;
    ID3D11VertexShader* vs = nullptr;
    ID3D11VertexShader* vsRect = nullptr;
    ID3D11PixelShader* ps = nullptr;
    ID3D11PixelShader* psTex = nullptr;
    ID3D11PixelShader* psColor = nullptr;
    ID3D11Buffer* cb = nullptr;
    ID3D11SamplerState* linear = nullptr;
    ID3D11BlendState* opaque = nullptr;
    ID3D11BlendState* premul = nullptr;
    ID3D11RasterizerState* raster = nullptr;
    ID3D11DepthStencilState* noDepth = nullptr;
    ID3D11Texture2D* copy = nullptr;
    ID3D11ShaderResourceView* copySrv = nullptr;
    UINT copyW = 0, copyH = 0;
    DXGI_FORMAT copyFormat = DXGI_FORMAT_UNKNOWN;
    ID3D11Texture2D* hud = nullptr;
    ID3D11ShaderResourceView* hudSrv = nullptr;
    UINT hudW = 0, hudH = 0;
    std::uint64_t hudSerial = ~0ull;
} gpu;

// Full-screen triangle; the pixel shader maps each output pixel to the point
// of the 120-degree frame it shows (full-frame equidistant fisheye), samples
// it with Catmull-Rom (9 bilinear taps) and darkens the corners a little.
// vs_rect/ps_tex draw the HUD texture into a rectangle (premultiplied alpha).
constexpr char kShader[] = R"(
cbuffer C : register(b0) {
    float aspect; float halfDiag; float cornerAngle; float tanHalf;
    float2 texSize; float vignette; float pad0;
    float4 rect;   // x0 y0 x1 y1 in clip space (HUD)
    float4 color;  // premultiplied (timeline)
};
Texture2D src : register(t0);
SamplerState smp : register(s0);
struct V { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
V vs_main(uint id : SV_VertexID) {
    V o;
    o.uv = float2((id << 1) & 2, id & 2);
    o.pos = float4(o.uv * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
}
V vs_rect(uint id : SV_VertexID) {
    V o;
    o.uv = float2(id & 1, id >> 1);
    o.pos = float4(lerp(rect.x, rect.z, o.uv.x), lerp(rect.y, rect.w, o.uv.y), 0, 1);
    return o;
}
float3 CatmullRom(float2 uv) {
    float2 p = uv * texSize;
    float2 t1 = floor(p - 0.5) + 0.5;
    float2 f = p - t1;
    float2 w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    float2 w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    float2 w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    float2 w3 = f * f * (-0.5 + 0.5 * f);
    float2 w12 = w1 + w2;
    float2 t0 = (t1 - 1) / texSize, t3 = (t1 + 2) / texSize, t12 = (t1 + w2 / w12) / texSize;
    float3 c = 0;
    c += src.SampleLevel(smp, float2(t0.x, t0.y), 0).rgb * w0.x * w0.y;
    c += src.SampleLevel(smp, float2(t12.x, t0.y), 0).rgb * w12.x * w0.y;
    c += src.SampleLevel(smp, float2(t3.x, t0.y), 0).rgb * w3.x * w0.y;
    c += src.SampleLevel(smp, float2(t0.x, t12.y), 0).rgb * w0.x * w12.y;
    c += src.SampleLevel(smp, float2(t12.x, t12.y), 0).rgb * w12.x * w12.y;
    c += src.SampleLevel(smp, float2(t3.x, t12.y), 0).rgb * w3.x * w12.y;
    c += src.SampleLevel(smp, float2(t0.x, t3.y), 0).rgb * w0.x * w3.y;
    c += src.SampleLevel(smp, float2(t12.x, t3.y), 0).rgb * w12.x * w3.y;
    c += src.SampleLevel(smp, float2(t3.x, t3.y), 0).rgb * w3.x * w3.y;
    return saturate(c);
}
float4 ps_main(V i) : SV_Target {
    float2 p = (i.uv * 2 - 1) * float2(aspect, 1);
    float r = length(p);
    float2 q = p;
    if (r > 1e-5) {
        float theta = r / halfDiag * cornerAngle;
        q = p * (tan(theta) / tanHalf / r);
    }
    float2 uv = (q / float2(aspect, 1)) * 0.5 + 0.5;
    float rho = r / halfDiag;
    float shade = 1.0 - vignette * smoothstep(0.55, 1.0, rho);
    return float4(CatmullRom(uv) * shade, 1);
}
float4 ps_tex(V i) : SV_Target { return src.Load(int3(i.uv * texSize, 0)); }
float4 ps_color(V i) : SV_Target { return color; }
)";

struct Constants {
    float aspect, halfDiag, cornerAngle, tanHalf;
    float texW, texH, vignette, pad0;
    float rect[4];
    float color[4];
};
static_assert(sizeof(Constants) % 16 == 0);

Constants Make(float w, float h, float vfov) {
    Constants c{};
    c.aspect = w / h;
    c.halfDiag = std::sqrt(c.aspect * c.aspect + 1.0f);
    c.tanHalf = std::tan(vfov * 0.5f * 0.01745329f);
    c.cornerAngle = std::atan(c.halfDiag * c.tanHalf);
    c.texW = w, c.texH = h;
    c.vignette = 0.35f;
    return c;
}

void Fail(const char* what) {
    if (!g_failed.exchange(true)) util::Logf(g_log, "fisheye: %s; the fisheye lens stays a plain 120-degree view", what);
}

void ReleaseCopy() {
    SafeRelease(gpu.copySrv);
    SafeRelease(gpu.copy);
    gpu.copyW = gpu.copyH = 0;
    gpu.copyFormat = DXGI_FORMAT_UNKNOWN;
}

void ReleaseHud() {
    SafeRelease(gpu.hudSrv);
    SafeRelease(gpu.hud);
    gpu.hudW = gpu.hudH = 0;
    gpu.hudSerial = ~0ull;
}

bool Typed(DXGI_FORMAT f, DXGI_FORMAT& typeless, DXGI_FORMAT& unorm) {
    switch (f) {
    case DXGI_FORMAT_R8G8B8A8_TYPELESS:
    case DXGI_FORMAT_R8G8B8A8_UNORM:
    case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB:
        typeless = DXGI_FORMAT_R8G8B8A8_TYPELESS, unorm = DXGI_FORMAT_R8G8B8A8_UNORM;
        return true;
    case DXGI_FORMAT_B8G8R8A8_TYPELESS:
    case DXGI_FORMAT_B8G8R8A8_UNORM:
    case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB:
        typeless = DXGI_FORMAT_B8G8R8A8_TYPELESS, unorm = DXGI_FORMAT_B8G8R8A8_UNORM;
        return true;
    case DXGI_FORMAT_R10G10B10A2_TYPELESS:
    case DXGI_FORMAT_R10G10B10A2_UNORM:
        typeless = DXGI_FORMAT_R10G10B10A2_TYPELESS, unorm = DXGI_FORMAT_R10G10B10A2_UNORM;
        return true;
    default:
        return false;
    }
}

bool Init(ID3D11Device* device) {
    if (gpu.device == device) return gpu.ps != nullptr;
    Release();
    g_failed = false;  // a new device: try again
    gpu.device = device;
    device->AddRef();
    device->GetImmediateContext(&gpu.ctx);
    auto compile = [](const char* entry, const char* profile) {
        return d3dstate::CompileShader(kShader, sizeof(kShader) - 1, "fisheye", entry, profile, g_log, "fisheye");
    };
    ID3DBlob* vsCode = compile("vs_main", "vs_4_0");
    ID3DBlob* vsRectCode = compile("vs_rect", "vs_4_0");
    ID3DBlob* psCode = compile("ps_main", "ps_4_0");
    ID3DBlob* psTexCode = compile("ps_tex", "ps_4_0");
    ID3DBlob* psColorCode = compile("ps_color", "ps_4_0");
    bool ok = vsCode && vsRectCode && psCode && psTexCode && psColorCode &&
              SUCCEEDED(device->CreatePixelShader(psColorCode->GetBufferPointer(), psColorCode->GetBufferSize(), nullptr, &gpu.psColor)) &&
              SUCCEEDED(device->CreateVertexShader(vsCode->GetBufferPointer(), vsCode->GetBufferSize(), nullptr, &gpu.vs)) &&
              SUCCEEDED(device->CreateVertexShader(vsRectCode->GetBufferPointer(), vsRectCode->GetBufferSize(), nullptr, &gpu.vsRect)) &&
              SUCCEEDED(device->CreatePixelShader(psCode->GetBufferPointer(), psCode->GetBufferSize(), nullptr, &gpu.ps)) &&
              SUCCEEDED(device->CreatePixelShader(psTexCode->GetBufferPointer(), psTexCode->GetBufferSize(), nullptr, &gpu.psTex));
    SafeRelease(vsCode);
    SafeRelease(vsRectCode);
    SafeRelease(psCode);
    SafeRelease(psTexCode);
    SafeRelease(psColorCode);
    if (ok) {
        D3D11_BUFFER_DESC d{};
        d.ByteWidth = sizeof(Constants);
        d.Usage = D3D11_USAGE_DYNAMIC;
        d.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
        d.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
        D3D11_SAMPLER_DESC s{};
        s.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;
        s.AddressU = s.AddressV = s.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
        s.MaxLOD = D3D11_FLOAT32_MAX;
        D3D11_BLEND_DESC bd{};
        bd.RenderTarget[0].RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
        D3D11_BLEND_DESC pm{};
        auto& rt = pm.RenderTarget[0];
        rt.BlendEnable = TRUE;
        rt.SrcBlend = D3D11_BLEND_ONE;
        rt.DestBlend = D3D11_BLEND_INV_SRC_ALPHA;
        rt.BlendOp = D3D11_BLEND_OP_ADD;
        rt.SrcBlendAlpha = D3D11_BLEND_ZERO;  // GTA's backbuffer alpha is left alone
        rt.DestBlendAlpha = D3D11_BLEND_ONE;
        rt.BlendOpAlpha = D3D11_BLEND_OP_ADD;
        rt.RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
        D3D11_RASTERIZER_DESC rd{};
        rd.FillMode = D3D11_FILL_SOLID;
        rd.CullMode = D3D11_CULL_NONE;
        rd.DepthClipEnable = TRUE;
        D3D11_DEPTH_STENCIL_DESC dd{};
        ok = SUCCEEDED(device->CreateBuffer(&d, nullptr, &gpu.cb)) && SUCCEEDED(device->CreateSamplerState(&s, &gpu.linear)) &&
             SUCCEEDED(device->CreateBlendState(&bd, &gpu.opaque)) && SUCCEEDED(device->CreateBlendState(&pm, &gpu.premul)) &&
             SUCCEEDED(device->CreateRasterizerState(&rd, &gpu.raster)) &&
             SUCCEEDED(device->CreateDepthStencilState(&dd, &gpu.noDepth));
    }
    if (!ok) {
        SafeRelease(gpu.ps);  // marks the device as failed for Init
        Fail("could not create the shaders or states");
    }
    return ok;
}

bool EnsureCopy(UINT w, UINT h, DXGI_FORMAT typeless, DXGI_FORMAT unorm) {
    if (gpu.copy && gpu.copyW == w && gpu.copyH == h && gpu.copyFormat == typeless) return true;
    ReleaseCopy();
    D3D11_TEXTURE2D_DESC d{};
    d.Width = w;
    d.Height = h;
    d.MipLevels = 1;
    d.ArraySize = 1;
    d.Format = typeless;
    d.SampleDesc.Count = 1;
    d.Usage = D3D11_USAGE_DEFAULT;
    d.BindFlags = D3D11_BIND_SHADER_RESOURCE;
    D3D11_SHADER_RESOURCE_VIEW_DESC sv{};
    sv.Format = unorm;
    sv.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
    sv.Texture2D.MipLevels = 1;
    if (FAILED(gpu.device->CreateTexture2D(&d, nullptr, &gpu.copy)) ||
        FAILED(gpu.device->CreateShaderResourceView(gpu.copy, &sv, &gpu.copySrv))) {
        ReleaseCopy();
        return false;
    }
    gpu.copyW = w, gpu.copyH = h, gpu.copyFormat = typeless;
    return true;
}

// The text band as premultiplied BGRA: white lines with a drop shadow (GDI),
// each line shrunk to fit the width.
void PaintHud(const Hud& h, UINT w, UINT bandH, std::vector<std::uint32_t>& out) {
    out.assign(static_cast<std::size_t>(w) * bandH, 0u);
    BITMAPINFO bi{};
    bi.bmiHeader.biSize = sizeof(bi.bmiHeader);
    bi.bmiHeader.biWidth = static_cast<LONG>(w);
    bi.bmiHeader.biHeight = -static_cast<LONG>(bandH);  // top-down
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB;
    void* bits = nullptr;
    HDC dc = CreateCompatibleDC(nullptr);
    HBITMAP bmp = dc ? CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &bits, nullptr, 0) : nullptr;
    const float frameH = static_cast<float>(bandH) / (kBandBottom - kBandTop);
    if (bmp && bits) {
        HGDIOBJ oldBmp = SelectObject(dc, bmp);
        std::memset(bits, 0, static_cast<std::size_t>(w) * bandH * 4);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, RGB(255, 255, 255));
        // Lines split at GTA's "~n~"; each centred in its half of the band, shrunk to 96 % of the width.
        std::vector<std::wstring> lines;
        for (std::size_t at = 0;;) {
            const std::size_t next = h.text.find("~n~", at);
            const std::string part = h.text.substr(at, next == std::string::npos ? std::string::npos : next - at);
            lines.emplace_back(part.begin(), part.end());  // ASCII
            if (next == std::string::npos) break;
            at = next + 3;
        }
        const int rows = static_cast<int>(lines.size());
        for (int i = 0; i < rows; ++i) {
            int height = std::max(10, static_cast<int>(frameH * (i == 0 ? 0.024f : 0.019f)));
            HFONT font = nullptr;
            for (int tries = 0; tries < 6; ++tries) {
                if (font) DeleteObject(font);
                font = CreateFontW(-height, 0, 0, 0, FW_SEMIBOLD, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                   ANTIALIASED_QUALITY, DEFAULT_PITCH, L"Segoe UI");
                SelectObject(dc, font);
                SIZE size{};
                GetTextExtentPoint32W(dc, lines[i].c_str(), static_cast<int>(lines[i].size()), &size);
                if (size.cx <= static_cast<LONG>(w * 0.96f) || height <= 10) break;
                height = std::max(10, static_cast<int>(height * w * 0.96f / size.cx));
            }
            RECT r{0, static_cast<LONG>(bandH * i / rows), static_cast<LONG>(w), static_cast<LONG>(bandH * (i + 1) / rows)};
            DrawTextW(dc, lines[i].c_str(), static_cast<int>(lines[i].size()), &r, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(dc, GetStockObject(SYSTEM_FONT));
            DeleteObject(font);
        }
        GdiFlush();
        const auto* src = static_cast<const std::uint32_t*>(bits);
        auto cover = [&](int x, int y) -> float {
            if (x < 0 || y < 0 || x >= static_cast<int>(w) || y >= static_cast<int>(bandH)) return 0.0f;
            const std::uint32_t p = src[static_cast<std::size_t>(y) * w + x];
            return static_cast<float>(std::max({p & 0xFFu, (p >> 8) & 0xFFu, (p >> 16) & 0xFFu})) / 255.0f;
        };
        for (int y = 0; y < static_cast<int>(bandH); ++y)
            for (int x = 0; x < static_cast<int>(w); ++x) {
                const float t = cover(x, y), sh = cover(x - 2, y - 2) * 0.75f;
                const float a = t + sh * (1.0f - t);
                if (a <= 0.0f) continue;
                const auto c = static_cast<std::uint32_t>(t * 255.0f + 0.5f);  // white, premultiplied
                out[static_cast<std::size_t>(y) * w + x] = (static_cast<std::uint32_t>(a * 255.0f + 0.5f) << 24) | (c << 16) | (c << 8) | c;
            }
        SelectObject(dc, oldBmp);
    }
    if (bmp) DeleteObject(bmp);
    if (dc) DeleteDC(dc);
}

// Uploads the HUD band when it changed; false: nothing to draw.
bool UpdateHud(UINT w, UINT h) {
    Hud hud;
    {
        std::lock_guard<std::mutex> lock(g_hudLock);
        hud = g_hud;
    }
    if (!hud.show) return false;
    const UINT bandH = std::max(8u, static_cast<UINT>(std::lround(kBandBottom * h) - std::lround(kBandTop * h)));
    if (!gpu.hud || gpu.hudW != w || gpu.hudH != bandH) {
        ReleaseHud();
        D3D11_TEXTURE2D_DESC d{};
        d.Width = w;
        d.Height = bandH;
        d.MipLevels = 1;
        d.ArraySize = 1;
        d.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
        d.SampleDesc.Count = 1;
        d.Usage = D3D11_USAGE_DYNAMIC;
        d.BindFlags = D3D11_BIND_SHADER_RESOURCE;
        d.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;
        if (FAILED(gpu.device->CreateTexture2D(&d, nullptr, &gpu.hud)) ||
            FAILED(gpu.device->CreateShaderResourceView(gpu.hud, nullptr, &gpu.hudSrv))) {
            ReleaseHud();
            return false;
        }
        gpu.hudW = w, gpu.hudH = bandH;
    }
    if (gpu.hudSerial != hud.serial) {
        static std::vector<std::uint32_t> pixels;
        PaintHud(hud, w, bandH, pixels);
        D3D11_MAPPED_SUBRESOURCE m{};
        if (FAILED(gpu.ctx->Map(gpu.hud, 0, D3D11_MAP_WRITE_DISCARD, 0, &m))) return false;
        for (UINT y = 0; y < bandH; ++y)
            std::memcpy(static_cast<std::uint8_t*>(m.pData) + static_cast<std::size_t>(y) * m.RowPitch, &pixels[static_cast<std::size_t>(y) * w], w * 4);
        gpu.ctx->Unmap(gpu.hud, 0);
        gpu.hudSerial = hud.serial;
    }
    return true;
}

}  // namespace

void SetLog(LogFn log) { g_log = log; }
void SetEnabled(bool on) { g_enabled = on; }
bool Enabled() { return g_enabled.load(); }

void SetHud(const std::string& text, float progress, bool show) {
    g_progress = progress;
    std::lock_guard<std::mutex> lock(g_hudLock);
    if (g_hud.text == text && g_hud.show == show) return;  // the text is painted again only when it changes
    g_hud.text = text, g_hud.show = show;
    ++g_hud.serial;
}

bool Drawing() {
    return g_enabled.load() && !g_failed.load() && GetTickCount64() - g_drawnAt.load() < 500;
}

void SourcePoint(float x, float y, float w, float h, float vfov, float& sx, float& sy) {
    const Constants c = Make(w, h, vfov);
    const float px = (x / w * 2.0f - 1.0f) * c.aspect, py = y / h * 2.0f - 1.0f;
    const float r = std::sqrt(px * px + py * py);
    float qx = px, qy = py;
    if (r > 1e-5f) {
        const float theta = r / c.halfDiag * c.cornerAngle;
        const float k = std::tan(theta) / c.tanHalf / r;
        qx = px * k, qy = py * k;
    }
    sx = (qx / c.aspect * 0.5f + 0.5f) * w;
    sy = (qy * 0.5f + 0.5f) * h;
}

void Present(IDXGISwapChain* sc) {
    if (!g_enabled.load() || g_failed.load() || !sc) return;
    ID3D11Device* device = nullptr;
    if (FAILED(sc->GetDevice(__uuidof(ID3D11Device), reinterpret_cast<void**>(&device)))) return;
    const bool ready = Init(device);
    device->Release();
    if (!ready) return;
    ID3D11Texture2D* back = nullptr;
    if (FAILED(sc->GetBuffer(0, __uuidof(ID3D11Texture2D), reinterpret_cast<void**>(&back)))) return;
    D3D11_TEXTURE2D_DESC bd{};
    back->GetDesc(&bd);
    DXGI_FORMAT typeless, unorm;
    if (bd.SampleDesc.Count != 1 || !Typed(bd.Format, typeless, unorm)) {
        back->Release();
        Fail("the backbuffer format is not supported");
        return;
    }
    ID3D11RenderTargetView* rtv = nullptr;
    D3D11_RENDER_TARGET_VIEW_DESC rv{};
    rv.Format = unorm;
    rv.ViewDimension = D3D11_RTV_DIMENSION_TEXTURE2D;
    if (!EnsureCopy(bd.Width, bd.Height, typeless, unorm) || FAILED(gpu.device->CreateRenderTargetView(back, &rv, &rtv))) {
        back->Release();  // a passing failure (resize, device reset): this frame stays unbent
        static bool logged = false;
        if (!logged) logged = true, util::Logf(g_log, "fisheye: could not create the frame copy this frame (retrying)");
        return;
    }
    ID3D11DeviceContext* c = gpu.ctx;
    d3dstate::SavedState saved;
    saved.Save(c);
    c->CopyResource(gpu.copy, back);
    Constants k = Make(static_cast<float>(bd.Width), static_cast<float>(bd.Height), kSourceFov);
    auto upload = [&] {
        D3D11_MAPPED_SUBRESOURCE m{};
        if (FAILED(c->Map(gpu.cb, 0, D3D11_MAP_WRITE_DISCARD, 0, &m))) return false;
        std::memcpy(m.pData, &k, sizeof(k));
        c->Unmap(gpu.cb, 0);
        return true;
    };
    upload();
    c->OMSetRenderTargets(1, &rtv, nullptr);
    const float factor[4] = {};
    c->OMSetBlendState(gpu.opaque, factor, 0xFFFFFFFFu);
    c->OMSetDepthStencilState(gpu.noDepth, 0);
    c->RSSetState(gpu.raster);
    D3D11_VIEWPORT vp{0.0f, 0.0f, static_cast<float>(bd.Width), static_cast<float>(bd.Height), 0.0f, 1.0f};
    c->RSSetViewports(1, &vp);
    c->IASetInputLayout(nullptr);
    c->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    c->VSSetShader(gpu.vs, nullptr, 0);
    c->GSSetShader(nullptr, nullptr, 0);
    c->HSSetShader(nullptr, nullptr, 0);
    c->DSSetShader(nullptr, nullptr, 0);
    c->PSSetShader(gpu.ps, nullptr, 0);
    c->VSSetConstantBuffers(0, 1, &gpu.cb);
    c->PSSetConstantBuffers(0, 1, &gpu.cb);
    c->PSSetShaderResources(0, 1, &gpu.copySrv);
    c->PSSetSamplers(0, 1, &gpu.linear);
    c->Draw(3, 0);
    g_drawnAt = GetTickCount64();
    // The replay HUD, unbent, over the picture: the timeline as solid
    // rectangles (as replay.cpp DrawTimeline: 0.2-0.8 wide, 0.8 % high), then the text band.
    if (UpdateHud(bd.Width, bd.Height)) {
        const float H = static_cast<float>(bd.Height);
        auto clipX = [](float x) { return x * 2.0f - 1.0f; };
        auto clipY = [](float y) { return 1.0f - y * 2.0f; };
        c->OMSetBlendState(gpu.premul, factor, 0xFFFFFFFFu);
        c->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        c->VSSetShader(gpu.vsRect, nullptr, 0);
        c->PSSetShader(gpu.psColor, nullptr, 0);
        const float f = std::clamp(g_progress.load(), 0.0f, 1.0f);
        auto bar = [&](float x0, float x1, float hh, float r, float g, float b, float a) {
            k.rect[0] = clipX(x0), k.rect[2] = clipX(x1);
            k.rect[1] = clipY(std::round((kTimelineY - hh / 2) * H) / H), k.rect[3] = clipY(std::round((kTimelineY + hh / 2) * H) / H);
            k.color[0] = r * a, k.color[1] = g * a, k.color[2] = b * a, k.color[3] = a;
            if (upload()) c->Draw(4, 0);
        };
        bar(0.2f, 0.8f, 0.008f, 0.08f, 0.08f, 0.08f, 0.7f);
        bar(0.2f, 0.2f + 0.6f * f, 0.008f, 1.0f, 0.78f, 0.16f, 0.9f);
        bar(0.2f + 0.6f * f - 0.0015f, 0.2f + 0.6f * f + 0.0015f, 0.02f, 1.0f, 1.0f, 1.0f, 1.0f);
        // The text band on whole pixels, texel for pixel (no blur).
        const float top = static_cast<float>(std::lround(kBandTop * H));
        k.rect[0] = -1.0f, k.rect[1] = clipY(top / H);
        k.rect[2] = 1.0f, k.rect[3] = clipY((top + gpu.hudH) / H);
        k.texW = static_cast<float>(gpu.hudW), k.texH = static_cast<float>(gpu.hudH);
        if (upload()) {
            c->PSSetShader(gpu.psTex, nullptr, 0);
            c->PSSetShaderResources(0, 1, &gpu.hudSrv);
            c->Draw(4, 0);
        }
    }
    ID3D11ShaderResourceView* none = nullptr;
    c->PSSetShaderResources(0, 1, &none);
    saved.Restore(c);
    rtv->Release();
    back->Release();
}

void Release() {
    ReleaseCopy();
    ReleaseHud();
    SafeRelease(gpu.vs);
    SafeRelease(gpu.vsRect);
    SafeRelease(gpu.ps);
    SafeRelease(gpu.psTex);
    SafeRelease(gpu.psColor);
    SafeRelease(gpu.cb);
    SafeRelease(gpu.linear);
    SafeRelease(gpu.opaque);
    SafeRelease(gpu.premul);
    SafeRelease(gpu.raster);
    SafeRelease(gpu.noDepth);
    SafeRelease(gpu.ctx);
    SafeRelease(gpu.device);
}

}  // namespace fisheye
