#pragma once
// Builds the C ABI SvReplayHudState frame snapshot sent to the ported Skate 3
// replay_editor2 APT HUD without ScriptHookV dependencies so it is unit-testable.
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>

#include "replay_edit.h"
#include "replay_logic.h"
#include "skatev_runtime.h"

namespace replay {

enum class ReplayHudScreen : std::uint8_t {
    Instant,
    Timeline,
    CameraEdit,
    Panel,         // EditMenu, Effects, Look, CameraMenu, Finished, Reel
    HiddenOverlay  // Loading, Help, Prompt
};

struct ReplayHudInput {
    bool activeStyle = false;  // HudStyle::Skate && !g_photoTaking
    ReplayHudScreen screen = ReplayHudScreen::Instant;
    bool playing = false;
    bool hudHidden = false;
    double recStart = 0.0;
    double recLength = 0.0;
    double playheadTime = 0.0;
    int clipIndex = 0;
    float shownZoomMm = 0.0f;
    std::string title;
    const Edit* edit = nullptr;
};

namespace detail {
inline void CopyUtf8(char* dst, std::size_t cap, const std::string& src) {
    if (!cap) return;
    std::size_t n = (std::min)(src.size(), cap - 1);
    if (n < src.size() && (static_cast<unsigned char>(src[n]) & 0xC0u) == 0x80u) {
        while (n > 0 && (static_cast<unsigned char>(src[n - 1]) & 0xC0u) == 0x80u) --n;
        if (n > 0) --n;
    }
    if (n) std::memcpy(dst, src.data(), n);
    dst[n] = '\0';
}

inline std::string FormatClock(double s) {
    char t[32];
    const double clamped = std::clamp(std::isfinite(s) ? s : 0.0, 0.0, 359999.9);
    const int m = static_cast<int>(clamped / 60.0);
    std::snprintf(t, sizeof(t), "%d:%04.1f", m, clamped - m * 60.0);
    return t;
}
}  // namespace detail

inline SvReplayHudState BuildReplayHudState(const ReplayHudInput& in) {
    SvReplayHudState st{};
    st.size = sizeof(st);
    st.selected_clip = -1;
    st.selected_key = -1;
    st.speed_pct = 100;
    if (!in.activeStyle || in.screen == ReplayHudScreen::HiddenOverlay || !in.edit) return st;

    st.active = 1;
    const bool preview = in.screen == ReplayHudScreen::Instant;
    const double len = (std::isfinite(in.recLength) && in.recLength > 1e-3) ? in.recLength : 1e-3;
    auto Norm = [&](double t) -> float {
        if (!std::isfinite(t) || !std::isfinite(in.recStart)) return 0.0f;
        return static_cast<float>(std::clamp((t - in.recStart) / len, 0.0, 1.0));
    };

    const auto& clips = in.edit->Clips();
    const int safeClipIdx = clips.empty() ? 0 : std::clamp(in.clipIndex, 0, static_cast<int>(clips.size()) - 1);
    const Clip emptyClip{};
    const Clip& clip = clips.empty() ? emptyClip : clips[static_cast<std::size_t>(safeClipIdx)];

    switch (in.screen) {
    case ReplayHudScreen::Instant:
        st.mode = 0;
        break;
    case ReplayHudScreen::Timeline:
        st.mode = 1;
        break;
    case ReplayHudScreen::CameraEdit:
        switch (clip.camera) {
        case CameraKind::Follow:
        case CameraKind::AdvFollow:
            st.mode = 2;
            break;
        case CameraKind::Tripod:
        case CameraKind::AdvTripod:
            st.mode = 3;
            break;
        case CameraKind::Free:
        case CameraKind::AdvFree:
            st.mode = 4;
            break;
        default:
            st.mode = 5;
            break;
        }
        break;
    case ReplayHudScreen::Panel:
    default:
        st.mode = 6;
        break;
    }

    std::uint32_t flags = 0;
    if (in.playing) flags |= 1u;
    if (in.hudHidden) flags |= 2u;
    if (in.screen == ReplayHudScreen::Instant || in.screen == ReplayHudScreen::Timeline ||
        in.screen == ReplayHudScreen::CameraEdit) {
        flags |= 16u;  // canShowHide
    }
    if (in.screen == ReplayHudScreen::CameraEdit && IsAdvanced(clip.camera)) {
        double bestDist = 0.25 + 1e-6;
        int bestIdx = -1;
        const int lim = static_cast<int>((std::min)(clip.keys.size(), std::size_t{48}));
        for (int k = 0; k < lim; ++k) {
            const double d = std::fabs(clip.keys[static_cast<std::size_t>(k)].time - in.playheadTime);
            if (d <= bestDist) {
                bestDist = d;
                bestIdx = k;
            }
        }
        if (bestIdx >= 0) {
            flags |= 4u;  // overKey
            st.selected_key = bestIdx;
        }
    }
    if (in.screen == ReplayHudScreen::Timeline) {
        for (double m : in.edit->Markers()) {
            if (std::fabs(m - in.playheadTime) <= 0.5) {
                flags |= 8u;  // overClipMarker
                break;
            }
        }
    }
    st.flags = flags;
    st.playhead = Norm(in.playheadTime);

    if (preview) {
        st.trim_in = 0.0f;
        st.trim_out = 1.0f;
        detail::CopyUtf8(st.encoded_time_utf8, sizeof(st.encoded_time_utf8),
                         detail::FormatClock(in.playheadTime - in.recStart) + " / " + detail::FormatClock(in.recLength));
    } else {
        st.trim_in = Norm(in.edit->In());
        st.trim_out = Norm(in.edit->Out());
        const std::uint32_t clipCount = static_cast<std::uint32_t>((std::min)(clips.size(), std::size_t{32}));
        st.clip_count = clipCount;
        st.selected_clip = clips.empty() ? -1 : safeClipIdx;
        for (std::uint32_t i = 0; i < clipCount; ++i) {
            const int idx = static_cast<int>(i);
            st.clips[i] = {Norm(in.edit->ClipStart(idx)), Norm(in.edit->ClipEnd(idx)),
                           static_cast<std::uint32_t>(clips[i].camera), idx == safeClipIdx ? 1u : 0u};
        }
        const std::uint32_t keyCount = static_cast<std::uint32_t>((std::min)(clip.keys.size(), std::size_t{48}));
        st.key_count = keyCount;
        for (std::uint32_t k = 0; k < keyCount; ++k) st.keys[k] = Norm(clip.keys[k].time);

        const auto& markers = in.edit->Markers();
        const std::uint32_t markerCount = static_cast<std::uint32_t>((std::min)(markers.size(), std::size_t{24}));
        st.marker_count = markerCount;
        for (std::uint32_t m = 0; m < markerCount; ++m) st.markers[m] = Norm(markers[m]);

        st.shake_type = static_cast<std::uint32_t>(std::clamp(clip.shake, 0, 3));
        static constexpr int kPct[] = {25, 50, 100, 200};
        st.speed_pct = kPct[std::clamp(clip.speed, 0, 3)];
        detail::CopyUtf8(st.encoded_time_utf8, sizeof(st.encoded_time_utf8),
                         detail::FormatClock(in.playheadTime - in.edit->In()) + " / " +
                             detail::FormatClock(in.edit->EditedLength()));
    }

    std::string zoom;
    if (clip.lens == kLensFisheye) zoom = "Fish Eye";
    else if (!preview && std::isfinite(in.shownZoomMm) && in.shownZoomMm > 0.0f) {
        const double zoomMm = std::clamp(static_cast<double>(in.shownZoomMm), 1.0, 9999.0);
        zoom = std::to_string(static_cast<int>(std::lround(zoomMm))) + "mm";
    } else {
        zoom = LensName(clip.lens);
    }
    detail::CopyUtf8(st.zoom_utf8, sizeof(st.zoom_utf8), zoom);
    detail::CopyUtf8(st.title_utf8, sizeof(st.title_utf8), in.title);
    return st;
}

}  // namespace replay
