#include "fisheye.h"

#include <atomic>
#include <cmath>
#include <cstring>

#include "d3d11_saved_state.h"
#include "host_util.h"

namespace fisheye {
namespace {
using d3dstate::SafeRelease;

constexpr float kSourceFov = 120.0f;  // the replay camera's vertical FOV for the fisheye lens

std::atomic<bool> g_enabled{false};
std::atomic<bool> g_failed{false};
LogFn g_log = nullptr;

struct Gpu {
    ID3D11Device* device = nullptr;
    ID3D11DeviceContext* ctx = nullptr;
    ID3D11VertexShader* vs = nullptr;
    ID3D11PixelShader* ps = nullptr;
    ID3D11Buffer* cb = nullptr;
    ID3D11SamplerState* linear = nullptr;
    ID3D11BlendState* opaque = nullptr;
    ID3D11RasterizerState* raster = nullptr;
    ID3D11DepthStencilState* noDepth = nullptr;
    ID3D11Texture2D* copy = nullptr;
    ID3D11ShaderResourceView* copySrv = nullptr;
    UINT copyW = 0, copyH = 0;
    DXGI_FORMAT copyFormat = DXGI_FORMAT_UNKNOWN;
} gpu;

// Full-screen triangle from the vertex id; the pixel shader maps each output
// pixel to the point of the 120-degree frame it shows (equidistant fisheye,
// the frame's corners map to the source's corners).
constexpr char kShader[] = R"(
cbuffer C : register(b0) { float aspect; float halfDiag; float cornerAngle; float tanHalf; };
Texture2D src : register(t0);
SamplerState smp : register(s0);
struct V { float4 pos : SV_Position; float2 uv : TEXCOORD0; };
V vs_main(uint id : SV_VertexID) {
    V o;
    o.uv = float2((id << 1) & 2, id & 2);
    o.pos = float4(o.uv * float2(2, -2) + float2(-1, 1), 0, 1);
    return o;
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
    return float4(src.SampleLevel(smp, uv, 0).rgb, 1);
}
)";

struct Constants {
    float aspect, halfDiag, cornerAngle, tanHalf;
};

Constants Make(float w, float h, float vfov) {
    Constants c{};
    c.aspect = w / h;
    c.halfDiag = std::sqrt(c.aspect * c.aspect + 1.0f);
    c.tanHalf = std::tan(vfov * 0.5f * 0.01745329f);
    c.cornerAngle = std::atan(c.halfDiag * c.tanHalf);
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
    ID3DBlob* vsCode = d3dstate::CompileShader(kShader, sizeof(kShader) - 1, "fisheye", "vs_main", "vs_4_0", g_log, "fisheye");
    ID3DBlob* psCode = d3dstate::CompileShader(kShader, sizeof(kShader) - 1, "fisheye", "ps_main", "ps_4_0", g_log, "fisheye");
    bool ok = vsCode && psCode &&
              SUCCEEDED(device->CreateVertexShader(vsCode->GetBufferPointer(), vsCode->GetBufferSize(), nullptr, &gpu.vs)) &&
              SUCCEEDED(device->CreatePixelShader(psCode->GetBufferPointer(), psCode->GetBufferSize(), nullptr, &gpu.ps));
    SafeRelease(vsCode);
    SafeRelease(psCode);
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
        D3D11_RASTERIZER_DESC rd{};
        rd.FillMode = D3D11_FILL_SOLID;
        rd.CullMode = D3D11_CULL_NONE;
        rd.DepthClipEnable = TRUE;
        D3D11_DEPTH_STENCIL_DESC dd{};
        ok = SUCCEEDED(device->CreateBuffer(&d, nullptr, &gpu.cb)) && SUCCEEDED(device->CreateSamplerState(&s, &gpu.linear)) &&
             SUCCEEDED(device->CreateBlendState(&bd, &gpu.opaque)) && SUCCEEDED(device->CreateRasterizerState(&rd, &gpu.raster)) &&
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

}  // namespace

void SetLog(LogFn log) { g_log = log; }
void SetEnabled(bool on) { g_enabled = on; }
bool Enabled() { return g_enabled.load(); }

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
    D3D11_MAPPED_SUBRESOURCE m{};
    if (SUCCEEDED(c->Map(gpu.cb, 0, D3D11_MAP_WRITE_DISCARD, 0, &m))) {
        const Constants k = Make(static_cast<float>(bd.Width), static_cast<float>(bd.Height), kSourceFov);
        std::memcpy(m.pData, &k, sizeof(k));
        c->Unmap(gpu.cb, 0);
    }
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
    c->PSSetConstantBuffers(0, 1, &gpu.cb);
    c->PSSetShaderResources(0, 1, &gpu.copySrv);
    c->PSSetSamplers(0, 1, &gpu.linear);
    c->Draw(3, 0);
    ID3D11ShaderResourceView* none = nullptr;
    c->PSSetShaderResources(0, 1, &none);
    saved.Restore(c);
    rtv->Release();
    back->Release();
}

void Release() {
    ReleaseCopy();
    SafeRelease(gpu.vs);
    SafeRelease(gpu.ps);
    SafeRelease(gpu.cb);
    SafeRelease(gpu.linear);
    SafeRelease(gpu.opaque);
    SafeRelease(gpu.raster);
    SafeRelease(gpu.noDepth);
    SafeRelease(gpu.ctx);
    SafeRelease(gpu.device);
}

}  // namespace fisheye
