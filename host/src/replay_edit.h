#pragma once
// Skate 3's replay editor model (design 2026-11-09-skate-replay-editor): the
// In and Out points over the recording, clip markers dividing it into clips,
// one camera per clip (Advanced cameras with keys), and each clip's effects.
// No GTA calls here, so it can be unit-tested.
#include <cstdint>
#include <vector>

namespace replay {

enum class CameraKind : std::uint8_t { Game, Follow, Tripod, Free, AdvFollow, AdvTripod, AdvFree, Count };
const char* CameraKindName(CameraKind k);  // Skate's names ("Game Camera", ...)
inline bool IsAdvanced(CameraKind k) { return k == CameraKind::AdvFollow || k == CameraKind::AdvTripod || k == CameraKind::AdvFree; }

// What a camera mode lets the player set (Skate: angle, distance, height,
// zoom, where on the skater's body it points; position for Tripod/Free).
struct CameraParams {
    float yaw = 0.0f, pitch = 0.0f;  // degrees: Follow around the skater; Tripod/Free the look direction
    float distance = 4.0f;           // m, Follow
    float height = 1.2f;             // m above the root (Follow), or the eye's own z offset
    float target = 0.6f;             // 0 feet .. 1 head: where on the skater the camera points
    float zoomMm = 0.0f;             // focal length (35 mm film); 0 = the camera's own field of view
    float position[3] = {};          // Tripod / Free: the eye in the world
    bool positioned = false;         // position set (Tripod/Free start where the camera was)
};
CameraParams Lerp(const CameraParams& a, const CameraParams& b, float t);  // angles the short way
CameraParams Spline(const CameraParams& p0, const CameraParams& p1, const CameraParams& p2, const CameraParams& p3, float t);

struct Key {
    double time = 0.0;
    CameraParams p;
};

struct Clip {
    CameraKind camera = CameraKind::Game;
    CameraParams base;
    std::vector<Key> keys;  // Advanced cameras only, sorted by time
    int speed = 2;          // index into kSpeeds (1x)
    int shake = 0;          // 0 Off, 1 Low, 2 Medium, 3 High
    int filter = 0;         // FilterName index
    int lens = 0;           // Lens (Auto)
};
float ShakeAmplitude(int shake);  // GTA HAND_SHAKE amplitude: 0, 0.3, 0.7, 1.2

// The result of an editing step (Skate's prompts).
enum class EditResult { Ok, TooClose, TooMany, NotFound, NeedsConfirm };

class Edit {
public:
    static constexpr int kMaxMarkers = 20;
    static constexpr double kMinGap = 0.25;  // s between markers and In/Out

    // A fresh edit over the recording's [start, end]: In and Out at the ends, one clip.
    void Reset(double start, double end);
    double In() const { return in_; }
    double Out() const { return out_; }
    double Start() const { return start_; }
    double End() const { return end_; }
    const std::vector<double>& Markers() const { return markers_; }
    const std::vector<Clip>& Clips() const { return clips_; }
    bool Edited() const { return edited_; }

    // In/Out at `t` (kept kMinGap from each other and from the markers inside;
    // markers outside the new range are removed with their clips' settings).
    EditResult SetIn(double t);
    EditResult SetOut(double t);
    // A clip marker at `t` splits the clip there; both halves keep its settings
    // and its keys on their side.
    EditResult PlaceMarker(double t);
    // The marker nearest `t` within `within` s. Deleting it joins the two clips;
    // both lose camera and keys (Skate: "All key and camera information will be
    // reset on either side of the marker"). `confirmed` false: NeedsConfirm.
    EditResult DeleteMarker(double t, double within, bool confirmed);
    int ClipAt(double t) const;  // index of the clip holding t (clamped to In..Out)
    double ClipStart(int i) const;
    double ClipEnd(int i) const;
    // Previous / next marker or In/Out from `t`.
    double PrevStop(double t) const;
    double NextStop(double t) const;

    // A different camera for a clip with keys or a non-Game camera asks first.
    EditResult SetCamera(int clip, CameraKind k, bool confirmed);
    Clip& ClipRef(int i) { return clips_[static_cast<std::size_t>(i)]; }
    const Clip& ClipRef(int i) const { return clips_[static_cast<std::size_t>(i)]; }
    void Touch() { edited_ = true; }
    // Keys (Advanced cameras): placed in the clip holding `t`, replacing one within 0.05 s.
    EditResult AddKey(double t, const CameraParams& p);
    EditResult DeleteKey(double t, double within);
    double PrevKey(double t) const;  // or t when none
    double NextKey(double t) const;

    // What plays at `t`: the clip and its camera parameters (keys interpolated).
    struct Frame {
        int clip = 0;
        CameraKind camera = CameraKind::Game;
        CameraParams p;
        float speed = 1.0f;
        int shake = 0, filter = 0, lens = 0;
    };
    Frame Evaluate(double t) const;
    // Seconds the edited video lasts (each clip's span / its speed).
    double EditedLength() const;

private:
    double start_ = 0.0, end_ = 0.0, in_ = 0.0, out_ = 0.0;
    std::vector<double> markers_;  // sorted, strictly inside (in, out)
    std::vector<Clip> clips_;      // markers_.size() + 1
    bool edited_ = false;
};

}  // namespace replay
