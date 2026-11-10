#include "fisheye.h"

#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <thread>
#include <vector>

#include <objbase.h>
#include <wincodec.h>

#include "d3d11_saved_state.h"
#include "host_util.h"

namespace fisheye {
namespace {
using d3dstate::SafeRelease;

constexpr float kSourceFov = 120.0f;  // the replay camera's vertical FOV for the fisheye lens
// The two text bands (fractions of the frame height).
constexpr float kBands[2][2] = {{0.0f, 0.2f}, {0.8f, 1.0f}};

std::atomic<bool> g_enabled{false};
std::atomic<bool> g_failed{false};
std::atomic<std::uint64_t> g_drawnAt{0};  // GetTickCount64 of the last bent frame
std::atomic<std::uint64_t> g_presents{0};
// The detail render: 0 off, 1 capturing (screen held), 2 in use, 3 dropped (held until the 60-degree frames are gone).
std::atomic<int> g_detailPhase{0};
std::atomic<std::uint64_t> g_detailAt{0}, g_liveAt{0};
std::atomic<bool> g_centerCaptured{false};
std::atomic<bool> g_filtered{false};
// The photo: requested by the script thread, taken by the render thread, written by a worker.
std::atomic<int> g_photoState{0};
std::atomic<std::uint64_t> g_photoAt{0};
std::mutex g_photoLock;
std::wstring g_photoPath;
LogFn g_log = nullptr;

// The HUD as the script thread last set it.
struct Hud {
    std::vector<HudRect> rects;
    std::vector<HudText> texts[2];  // per band
    bool show = false;
    std::uint64_t serial[2] = {};   // per band: its texts changed
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
    ID3D11Texture2D* center = nullptr;        // the detail render of the middle
    ID3D11ShaderResourceView* centerSrv = nullptr;
    bool baseValid = false;                   // `copy` holds a 120-degree frame
    UINT copyW = 0, copyH = 0;
    DXGI_FORMAT copyFormat = DXGI_FORMAT_UNKNOWN;
    ID3D11Texture2D* hud[2] = {};
    ID3D11ShaderResourceView* hudSrv[2] = {};
    UINT hudW = 0, hudH[2] = {};
    std::uint64_t hudSerial[2] = {~0ull, ~0ull};
} gpu;

// Full-screen triangle; the pixel shader maps each output pixel to the point
// of the 120-degree frame it shows (full-frame equidistant fisheye), samples
// it with Catmull-Rom (9 bilinear taps) and darkens the corners a little.
// vs_rect/ps_tex draw the HUD texture into a rectangle (premultiplied alpha).
constexpr char kShader[] = R"(
cbuffer C : register(b0) {
    float aspect; float halfDiag; float cornerAngle; float tanHalf;
    float2 texSize; float vignette; float pad0;
    float centerScale; float detail; float2 pad1;  // the detail view: tan(60)/tan(30); 1 when it is valid
    float4 rect;   // x0 y0 x1 y1 in clip space (HUD)
    float4 color;  // premultiplied (timeline)
};
Texture2D src : register(t0);
Texture2D centerTex : register(t1);
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
float3 CatmullRomOf(Texture2D tex, float2 uv) {
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
    c += tex.SampleLevel(smp, float2(t0.x, t0.y), 0).rgb * w0.x * w0.y;
    c += tex.SampleLevel(smp, float2(t12.x, t0.y), 0).rgb * w12.x * w0.y;
    c += tex.SampleLevel(smp, float2(t3.x, t0.y), 0).rgb * w3.x * w0.y;
    c += tex.SampleLevel(smp, float2(t0.x, t12.y), 0).rgb * w0.x * w12.y;
    c += tex.SampleLevel(smp, float2(t12.x, t12.y), 0).rgb * w12.x * w12.y;
    c += tex.SampleLevel(smp, float2(t3.x, t12.y), 0).rgb * w3.x * w12.y;
    c += tex.SampleLevel(smp, float2(t0.x, t3.y), 0).rgb * w0.x * w3.y;
    c += tex.SampleLevel(smp, float2(t12.x, t3.y), 0).rgb * w12.x * w3.y;
    c += tex.SampleLevel(smp, float2(t3.x, t3.y), 0).rgb * w3.x * w3.y;
    return saturate(c);
}
float3 CatmullRom(float2 uv) { return CatmullRomOf(src, uv); }
float Luma(float3 c) { return dot(c, float3(0.299, 0.587, 0.114)); }
// FXAA (the classic short form) at a point of the 120-degree frame: along an
// edge the colour is averaged, so the enlarged middle shows no stair-steps.
float3 Fxaa(float2 uv) {
    float2 px = 1.0 / texSize;
    float3 rgbM = src.SampleLevel(smp, uv, 0).rgb;
    float lNW = Luma(src.SampleLevel(smp, uv + float2(-1, -1) * px, 0).rgb);
    float lNE = Luma(src.SampleLevel(smp, uv + float2(1, -1) * px, 0).rgb);
    float lSW = Luma(src.SampleLevel(smp, uv + float2(-1, 1) * px, 0).rgb);
    float lSE = Luma(src.SampleLevel(smp, uv + float2(1, 1) * px, 0).rgb);
    float lM = Luma(rgbM);
    float lMin = min(lM, min(min(lNW, lNE), min(lSW, lSE)));
    float lMax = max(lM, max(max(lNW, lNE), max(lSW, lSE)));
    if (lMax - lMin < max(0.0312, lMax * 0.125)) return CatmullRom(uv);  // no edge: the sharp sampling
    float2 dir = float2(-((lNW + lNE) - (lSW + lSE)), (lNW + lSW) - (lNE + lSE));
    float reduce = max((lNW + lNE + lSW + lSE) * 0.03125, 1.0 / 128.0);
    float rcpMin = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * rcpMin, -8.0, 8.0) * px;
    float3 a = 0.5 * (src.SampleLevel(smp, uv + dir * (1.0 / 3.0 - 0.5), 0).rgb + src.SampleLevel(smp, uv + dir * (2.0 / 3.0 - 0.5), 0).rgb);
    float3 b = a * 0.5 + 0.25 * (src.SampleLevel(smp, uv - dir * 0.5, 0).rgb + src.SampleLevel(smp, uv + dir * 0.5, 0).rgb);
    float lB = Luma(b);
    return (lB < lMin || lB > lMax) ? a : b;
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
    float3 colour = Fxaa(uv);
    if (detail > 0.5) {
        // The same direction in the 60-degree detail view (same camera, narrower lens).
        float2 qc = q * centerScale;
        float edge = max(abs(qc.x) / aspect, abs(qc.y));
        float w = 1.0 - smoothstep(0.82, 0.97, edge);
        if (w > 0.0) {
            float2 uvc = (qc / float2(aspect, 1)) * 0.5 + 0.5;
            float3 sharp = CatmullRomOf(centerTex, uvc);
            if (detail > 1.5) {
                // Filtered view: keep the 120-degree frame's low-frequency colour and
                // vignette, and add only the 60-degree view's fine detail.
                float2 d = centerScale * 0.5 / texSize;
                float3 low = 0.25 * centerTex.SampleLevel(smp, uvc, 0).rgb
                    + 0.1875 * (centerTex.SampleLevel(smp, uvc + float2(-d.x, -d.y), 0).rgb
                              + centerTex.SampleLevel(smp, uvc + float2( d.x, -d.y), 0).rgb
                              + centerTex.SampleLevel(smp, uvc + float2(-d.x,  d.y), 0).rgb
                              + centerTex.SampleLevel(smp, uvc + float2( d.x,  d.y), 0).rgb);
                colour = saturate(colour + (sharp - low) * w);
            } else {
                colour = lerp(colour, sharp, w);
            }
        }
    }
    return float4(colour * shade, 1);
}
float4 ps_tex(V i) : SV_Target { return src.Load(int3(i.uv * texSize, 0)); }
float4 ps_color(V i) : SV_Target { return color; }
)";

struct Constants {
    float aspect, halfDiag, cornerAngle, tanHalf;
    float texW, texH, vignette, pad0;
    float centerScale, detail, pad1, pad2;
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
    c.centerScale = c.tanHalf / std::tan(kDetailFov * 0.5f * 0.01745329f);
    return c;
}

void Fail(const char* what) {
    if (!g_failed.exchange(true)) util::Logf(g_log, "fisheye: %s; the fisheye lens stays a plain 120-degree view", what);
}

void ReleaseCopy() {
    SafeRelease(gpu.copySrv);
    SafeRelease(gpu.copy);
    SafeRelease(gpu.centerSrv);
    SafeRelease(gpu.center);
    gpu.baseValid = false;
    g_centerCaptured = false;  // a new size: the detail is gone
    g_detailPhase = 0;
    gpu.copyW = gpu.copyH = 0;
    gpu.copyFormat = DXGI_FORMAT_UNKNOWN;
}

void ReleaseHud() {
    for (int i = 0; i < 2; ++i) {
        SafeRelease(gpu.hudSrv[i]);
        SafeRelease(gpu.hud[i]);
        gpu.hudH[i] = 0;
        gpu.hudSerial[i] = ~0ull;
    }
    gpu.hudW = 0;
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
        FAILED(gpu.device->CreateShaderResourceView(gpu.copy, &sv, &gpu.copySrv)) ||
        FAILED(gpu.device->CreateTexture2D(&d, nullptr, &gpu.center)) ||
        FAILED(gpu.device->CreateShaderResourceView(gpu.center, &sv, &gpu.centerSrv))) {
        ReleaseCopy();
        return false;
    }
    gpu.copyW = w, gpu.copyH = h, gpu.copyFormat = typeless;
    return true;
}

// One band as premultiplied BGRA: its white texts with a drop shadow (GDI),
// each at its place; a line wider than 96 % of the screen is shrunk to fit.
void PaintHud(const std::vector<HudText>& texts, float bandTop, UINT w, UINT bandH, float frameH, std::vector<std::uint32_t>& out) {
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
    if (bmp && bits) {
        HGDIOBJ oldBmp = SelectObject(dc, bmp);
        std::memset(bits, 0, static_cast<std::size_t>(w) * bandH * 4);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, RGB(255, 255, 255));
        int top = static_cast<int>(bandH), bottom = 0;
        for (const HudText& t : texts) {
            const std::wstring line(t.text.begin(), t.text.end());  // ASCII
            // GTA's text scale 1.0 is about 7 % of the screen height.
            int height = std::max(10, static_cast<int>(frameH * 0.07f * t.scale));
            HFONT font = nullptr;
            SIZE size{};
            for (int tries = 0; tries < 6; ++tries) {
                if (font) SelectObject(dc, GetStockObject(SYSTEM_FONT)), DeleteObject(font);
                font = CreateFontW(-height, 0, 0, 0, FW_SEMIBOLD, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                   ANTIALIASED_QUALITY, DEFAULT_PITCH, L"Segoe UI");
                SelectObject(dc, font);
                GetTextExtentPoint32W(dc, line.c_str(), static_cast<int>(line.size()), &size);
                if (size.cx <= static_cast<LONG>(w * 0.96f) || height <= 10) break;
                height = std::max(10, static_cast<int>(height * w * 0.96f / size.cx));
            }
            const int y = static_cast<int>((t.y - bandTop) * frameH);
            const int x = t.centre ? static_cast<int>(t.x * w) - size.cx / 2 : static_cast<int>(t.x * w);
            TextOutW(dc, x, y, line.c_str(), static_cast<int>(line.size()));
            top = std::min(top, y), bottom = std::max(bottom, y + static_cast<int>(size.cy) + 3);
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
        for (int y = std::max(0, top); y < std::min(static_cast<int>(bandH), bottom); ++y)
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

// Uploads a band's texture when its texts changed; false: nothing to draw.
bool UpdateBand(int band, const Hud& hud, UINT w, UINT h) {
    if (hud.texts[band].empty()) return false;
    const UINT top = static_cast<UINT>(std::lround(kBands[band][0] * h));
    const UINT bandH = std::max(8u, static_cast<UINT>(std::lround(kBands[band][1] * h)) - top);
    if (!gpu.hud[band] || gpu.hudW != w || gpu.hudH[band] != bandH) {
        SafeRelease(gpu.hudSrv[band]);
        SafeRelease(gpu.hud[band]);
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
        if (FAILED(gpu.device->CreateTexture2D(&d, nullptr, &gpu.hud[band])) ||
            FAILED(gpu.device->CreateShaderResourceView(gpu.hud[band], nullptr, &gpu.hudSrv[band]))) {
            SafeRelease(gpu.hudSrv[band]);
            SafeRelease(gpu.hud[band]);
            return false;
        }
        gpu.hudW = w, gpu.hudH[band] = bandH;
        gpu.hudSerial[band] = ~0ull;
    }
    if (gpu.hudSerial[band] != hud.serial[band]) {
        static std::vector<std::uint32_t> pixels;
        PaintHud(hud.texts[band], kBands[band][0], w, bandH, static_cast<float>(h), pixels);
        D3D11_MAPPED_SUBRESOURCE m{};
        if (FAILED(gpu.ctx->Map(gpu.hud[band], 0, D3D11_MAP_WRITE_DISCARD, 0, &m))) return false;
        for (UINT y = 0; y < bandH; ++y)
            std::memcpy(static_cast<std::uint8_t*>(m.pData) + static_cast<std::size_t>(y) * m.RowPitch, &pixels[static_cast<std::size_t>(y) * w], w * 4);
        gpu.ctx->Unmap(gpu.hud[band], 0);
        gpu.hudSerial[band] = hud.serial[band];
    }
    return true;
}

}  // namespace

void SetLog(LogFn log) { g_log = log; }
void SetEnabled(bool on) {
    if (g_enabled.exchange(on) != on) {
        if (on) g_liveAt = std::max(g_liveAt.load(), g_presents.load() + 1);
        else g_drawnAt = 0, gpu.baseValid = false;
    }
}
bool Enabled() { return g_enabled.load(); }

void SetHudItems(const std::vector<HudRect>& rects, const std::vector<HudText>& texts, bool show) {
    std::vector<HudText> band[2];
    for (const HudText& t : texts) band[t.y < 0.5f ? 0 : 1].push_back(t);
    std::lock_guard<std::mutex> lock(g_hudLock);
    g_hud.rects = rects;
    g_hud.show = show;
    for (int i = 0; i < 2; ++i)
        if (!(g_hud.texts[i] == band[i])) g_hud.texts[i] = std::move(band[i]), ++g_hud.serial[i];  // painted again only on change
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

namespace {
void Bend(IDXGISwapChain* sc);
void Capture(IDXGISwapChain* sc);
}  // namespace

void Present(IDXGISwapChain* sc) {
    if (!sc) return;
    const std::uint64_t n = ++g_presents;
    if (g_enabled.load() && !g_failed.load()) Bend(sc);
    if (g_photoState.load() == 1 && g_photoAt.load() && n >= g_photoAt.load()) Capture(sc);
}

namespace {

void Bend(IDXGISwapChain* sc) {
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
    // The detail render: while capturing, the backbuffer holds the 60-degree view
    // (kept as `center`) and the screen shows the last 120-degree frame.
    const std::uint64_t n = g_presents.load();
    const int phase = g_detailPhase.load();
    bool hold = false;
    if (phase == 1) {
        hold = true;
        if (!g_centerCaptured.load() && n >= g_detailAt.load()) {
            c->CopyResource(gpu.center, back);
            g_centerCaptured = true;
        }
    } else if ((phase == 2 || phase == 3) && n < g_liveAt.load()) {
        hold = true;  // the camera turns back to 120 degrees a frame or two late
    } else if (phase == 3) {
        int expected = 3;
        g_detailPhase.compare_exchange_strong(expected, 0);
    }
    if (!hold || !gpu.baseValid) c->CopyResource(gpu.copy, back), gpu.baseValid = true;
    Constants k = Make(static_cast<float>(bd.Width), static_cast<float>(bd.Height), kSourceFov);
    k.detail = phase == 2 && g_centerCaptured.load() ? (g_filtered.load() ? 2.0f : 1.0f) : 0.0f;  // phase 3 holds without it
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
    ID3D11ShaderResourceView* views[2] = {gpu.copySrv, gpu.centerSrv};
    c->PSSetShaderResources(0, 2, views);
    c->PSSetSamplers(0, 1, &gpu.linear);
    c->Draw(3, 0);
    g_drawnAt = GetTickCount64();
    // The replay HUD, unbent, over the picture: rectangles, then the two text bands.
    Hud hud;
    {
        std::lock_guard<std::mutex> lock(g_hudLock);
        hud = g_hud;
    }
    if (hud.show) {
        const float H = static_cast<float>(bd.Height);
        auto clipX = [](float x) { return x * 2.0f - 1.0f; };
        auto clipY = [](float y) { return 1.0f - y * 2.0f; };
        c->OMSetBlendState(gpu.premul, factor, 0xFFFFFFFFu);
        c->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        c->VSSetShader(gpu.vsRect, nullptr, 0);
        c->PSSetShader(gpu.psColor, nullptr, 0);
        for (const HudRect& r : hud.rects) {
            k.rect[0] = clipX(r.x0), k.rect[2] = clipX(r.x1);
            k.rect[1] = clipY(std::round(r.y0 * H) / H), k.rect[3] = clipY(std::max(std::round(r.y1 * H), std::round(r.y0 * H) + 1.0f) / H);
            k.color[0] = r.r * r.a, k.color[1] = r.g * r.a, k.color[2] = r.b * r.a, k.color[3] = r.a;
            if (upload()) c->Draw(4, 0);
        }
        for (int band = 0; band < 2; ++band) {
            if (!UpdateBand(band, hud, bd.Width, bd.Height)) continue;
            // On whole pixels, texel for pixel (no blur).
            const float top = static_cast<float>(std::lround(kBands[band][0] * H));
            k.rect[0] = -1.0f, k.rect[1] = clipY(top / H);
            k.rect[2] = 1.0f, k.rect[3] = clipY((top + gpu.hudH[band]) / H);
            k.texW = static_cast<float>(gpu.hudW), k.texH = static_cast<float>(gpu.hudH[band]);
            if (upload()) {
                c->PSSetShader(gpu.psTex, nullptr, 0);
                c->PSSetShaderResources(0, 1, &gpu.hudSrv[band]);
                c->Draw(4, 0);
            }
        }
    }
    ID3D11ShaderResourceView* none[2] = {};
    c->PSSetShaderResources(0, 2, none);
    saved.Restore(c);
    rtv->Release();
    back->Release();
}

// The presented frame into a PNG on a worker thread (the render thread only copies it).
void Capture(IDXGISwapChain* sc) {
    g_photoAt = 0;
    auto fail = [](const char* why) {
        util::Logf(g_log, "photo: %s", why);
        g_photoState = 3;
    };
    ID3D11Device* device = nullptr;
    if (FAILED(sc->GetDevice(__uuidof(ID3D11Device), reinterpret_cast<void**>(&device)))) return fail("no device");
    const bool ready = Init(device);
    device->Release();
    if (!ready) return fail("no GPU objects");
    ID3D11Texture2D* back = nullptr;
    if (FAILED(sc->GetBuffer(0, __uuidof(ID3D11Texture2D), reinterpret_cast<void**>(&back)))) return fail("no backbuffer");
    D3D11_TEXTURE2D_DESC d{};
    back->GetDesc(&d);
    DXGI_FORMAT typeless, unorm;
    if (d.SampleDesc.Count != 1 || !Typed(d.Format, typeless, unorm)) {
        back->Release();
        return fail("the backbuffer format is not supported");
    }
    d.Usage = D3D11_USAGE_STAGING;
    d.BindFlags = 0;
    d.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
    d.MiscFlags = 0;
    d.Format = unorm;
    ID3D11Texture2D* staging = nullptr;
    if (FAILED(gpu.device->CreateTexture2D(&d, nullptr, &staging))) {
        back->Release();
        return fail("no staging texture");
    }
    gpu.ctx->CopyResource(staging, back);
    back->Release();
    D3D11_MAPPED_SUBRESOURCE m{};
    if (FAILED(gpu.ctx->Map(staging, 0, D3D11_MAP_READ, 0, &m))) {
        staging->Release();
        return fail("could not read the frame");
    }
    // Only the raw rows here (render thread); the worker converts them.
    std::vector<std::uint32_t> raw(static_cast<std::size_t>(d.Width) * d.Height);
    for (UINT y = 0; y < d.Height; ++y)
        std::memcpy(&raw[static_cast<std::size_t>(y) * d.Width], static_cast<const std::uint8_t*>(m.pData) + static_cast<std::size_t>(y) * m.RowPitch,
                    static_cast<std::size_t>(d.Width) * 4);
    gpu.ctx->Unmap(staging, 0);
    staging->Release();
    std::wstring path;
    {
        std::lock_guard<std::mutex> lock(g_photoLock);
        path = g_photoPath;
    }
    const UINT w = d.Width, h = d.Height;
    std::thread([raw = std::move(raw), w, h, path, unorm] {
        std::vector<unsigned char> pixels(static_cast<std::size_t>(w) * h * 3);
        unsigned char* o = pixels.data();
        for (std::uint32_t p : raw) {
            if (unorm == DXGI_FORMAT_B8G8R8A8_UNORM) o[0] = p & 0xFF, o[1] = (p >> 8) & 0xFF, o[2] = (p >> 16) & 0xFF;
            else if (unorm == DXGI_FORMAT_R8G8B8A8_UNORM) o[0] = (p >> 16) & 0xFF, o[1] = (p >> 8) & 0xFF, o[2] = p & 0xFF;
            else o[0] = static_cast<unsigned char>(((p >> 20) & 0x3FF) >> 2), o[1] = static_cast<unsigned char>(((p >> 10) & 0x3FF) >> 2),
                 o[2] = static_cast<unsigned char>((p & 0x3FF) >> 2);  // 10-bit
            o += 3;
        }
        const bool ok = WritePng(path, pixels.data(), w, h);
        util::Logf(g_log, ok ? "photo: saved %ls" : "photo: could not write %ls", path.c_str());
        g_photoState = ok ? 2 : 3;
    }).detach();
}

}  // namespace

void RequestPhoto(const std::wstring& path) {
    {
        std::lock_guard<std::mutex> lock(g_photoLock);
        g_photoPath = path;
    }
    g_photoState = 1;
    g_photoAt = g_presents.load() + 3;  // GTA presents a frame or two after the script drew it
}

int PhotoState() { return g_photoState.load(); }

void SetFiltered(bool on) {
    g_filtered = on;
    g_liveAt = std::max(g_liveAt.load(), g_presents.load() + 1);
}
bool DetailIdle() { return g_detailPhase.load() == 0 && g_presents.load() >= g_liveAt.load(); }
void BeginDetail() {
    g_centerCaptured = false;
    g_detailAt = ~0ull;  // armed once GTA is seen rendering the detail view (ArmDetail)
    g_detailPhase = 1;
}
void ArmDetail() {
    if (g_detailPhase.load() == 1 && g_detailAt.load() == ~0ull) g_detailAt = g_presents.load() + 3;
}
bool DetailCaptured() { return g_detailPhase.load() == 1 && g_centerCaptured.load(); }
void UseDetail() {
    g_liveAt = g_presents.load() + 3;
    g_detailPhase = 2;
}
void DropDetail() {
    // While capturing, frames at the detail FOV are still on their way: hold until they are gone.
    if (g_detailPhase.load() == 1) g_liveAt = g_presents.load() + 4, g_detailPhase = 3;
    else if (g_detailPhase.load() != 3) g_detailPhase = 0;
    g_centerCaptured = false;
}
bool DetailReady() { return g_detailPhase.load() == 2 && g_centerCaptured.load() && g_presents.load() >= g_liveAt.load(); }
void CancelPhoto() {
    if (g_photoState.load() == 1 && g_photoAt.exchange(0)) g_photoState = 0;  // not taken yet (taken: the writer finishes it)
}
void ResetPhoto() {
    if (g_photoState.load() != 1) g_photoState = 0;
}

bool WritePng(const std::wstring& path, const unsigned char* bgr, unsigned width, unsigned height) {
    const HRESULT init = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    bool ok = false;
    IWICImagingFactory* factory = nullptr;
    IWICStream* stream = nullptr;
    IWICBitmapEncoder* encoder = nullptr;
    IWICBitmapFrameEncode* frame = nullptr;
    WICPixelFormatGUID format = GUID_WICPixelFormat24bppBGR;
    if (SUCCEEDED(CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&factory))) &&
        SUCCEEDED(factory->CreateStream(&stream)) && SUCCEEDED(stream->InitializeFromFilename(path.c_str(), GENERIC_WRITE)) &&
        SUCCEEDED(factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, &encoder)) &&
        SUCCEEDED(encoder->Initialize(stream, WICBitmapEncoderNoCache)) && SUCCEEDED(encoder->CreateNewFrame(&frame, nullptr)) &&
        SUCCEEDED(frame->Initialize(nullptr)) && SUCCEEDED(frame->SetSize(width, height)) &&
        SUCCEEDED(frame->SetPixelFormat(&format)) && format == GUID_WICPixelFormat24bppBGR &&
        SUCCEEDED(frame->WritePixels(height, width * 3, width * 3 * height, const_cast<BYTE*>(bgr))) && SUCCEEDED(frame->Commit()) &&
        SUCCEEDED(encoder->Commit()))
        ok = true;
    if (frame) frame->Release();
    if (encoder) encoder->Release();
    if (stream) stream->Release();
    if (factory) factory->Release();
    if (SUCCEEDED(init)) CoUninitialize();
    return ok;
}

void Release() {
    g_filtered = false;
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
