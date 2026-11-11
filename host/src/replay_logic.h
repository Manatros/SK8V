#pragma once
// Replay (design 2026-11-02-replay-editor): the rolling recording of what
// skating showed (skater root, bone matrices, board, camera) and the
// playhead over it. No GTA calls here, so it can be unit-tested.
#include <cstdint>
#include <vector>

namespace replay {

// One presented frame. Bone and board matrices: 16 floats each, GTA world
// space, RAGE row layout (as posehook::PublishPose takes them).
struct FrameData {
    double time = 0.0;           // seconds since the recording began
    bool cut = false;            // a teleport or respawn led into this frame
    float root[3] = {};          // skater root (ped placement)
    float heading = 0.0f;        // skater heading, degrees
    float velocity[3] = {};
    float camera[7] = {};        // position, rotation (pitch, roll, yaw), fov
    std::uint32_t gameMs = 0;    // GET_GAME_TIMER at this frame (replay sound)
    bool board = false;          // board matrices valid
    float boardEntity[16] = {};
    float boardWorld[7][16] = {};
};
struct Frame : FrameData {
    std::vector<float> bones;    // bones x 16
};

// The last `seconds` of frames. Slots are reused (no allocation once full).
class Recording {
public:
    explicit Recording(double seconds = 30.0) : window_(seconds) {}
    void SetWindow(double seconds);
    // Makes the slots once: `seconds` at `rate` frames a second (plus one),
    // each with room for `bones`; when full, the oldest frame is overwritten.
    void Reserve(double seconds, double rate, int bones);
    std::size_t Capacity() const { return slots_.size(); }
    // Appends a frame `dt` seconds after the previous one. A different bone
    // count than the frames held clears them first (a new skeleton).
    void Add(const Frame& f, double dt);
    void Clear();
    std::size_t Size() const { return count_; }
    double Start() const;
    double End() const;
    double Length() const { return Size() ? End() - Start() : 0.0; }
    // The frame at or just before `time` and the one after, with the fraction
    // between them (0 at `a`); `b` is `a` across a cut or at the ends.
    struct Sample {
        const Frame* a = nullptr;
        const Frame* b = nullptr;
        float t = 0.0f;
    };
    Sample At(double time) const;
    const Frame& Last() const { return At(count_ - 1); }
    const Frame& Get(std::size_t i) const { return At(i); }  // 0 = the oldest

private:
    const Frame& At(std::size_t i) const { return slots_[(head_ + i) % slots_.size()]; }
    std::vector<Frame> slots_;
    std::size_t head_ = 0, count_ = 0;
    double window_;
    double clock_ = 0.0;
};

// Playback speeds, slowest first (Skate's editor steps).
inline constexpr float kSpeeds[] = {0.25f, 0.5f, 1.0f, 2.0f};
inline constexpr int kNormalSpeed = 2;

// The replay's playhead: time within [start, end], speed step, direction.
struct Playhead {
    double time = 0.0;
    int speed = kNormalSpeed;
    bool playing = true;
    bool reverse = false;
    // Advances by `dt` real seconds; stops (paused) at either end.
    void Advance(double dt, double start, double end);
    void Faster() { if (speed + 1 < static_cast<int>(std::size(kSpeeds))) ++speed; }
    void Slower() { if (speed > 0) --speed; }
};

// The replay's controls from either device, read the same way (design
// 2026-11-09-replay-input). A controller and the keyboard and mouse work
// side by side; the keyboard keys are fixed and named in the help line.
struct PadIn {                     // XInput layout
    std::uint16_t buttons = 0;
    std::uint8_t leftTrigger = 0, rightTrigger = 0;
    std::int16_t leftX = 0, leftY = 0, rightX = 0, rightY = 0;
};
struct KeysIn {                    // held this frame
    bool play = false;             // Space
    bool read = true;              // false: no focus or a menu has the keys (held state kept, no edges)
    bool faster = false, slower = false;  // Up / Down arrow (held)
    bool wheelUp = false, wheelDown = false;  // a mouse wheel notch this frame (one step each, no edge test)
    bool camera = false;           // C
    bool leave = false;            // Tab or Backspace
    bool reverse = false;          // R
    bool left = false, right = false;     // A / D
    bool arrowLeft = false, arrowRight = false;
    bool photo = false;                   // P
    bool forward = false, back = false;   // W / S
    bool up = false, down = false;        // E / Q
    bool plus = false, minus = false;     // + / - (main row and numpad): speed
    bool filter = false, lens = false;    // F / L
    bool shift = false;                   // Shift held: F and L step back
    bool hide = false;                    // H: hide or show the replay HUD
    bool stepBack = false, stepForward = false;  // , and . : one frame (pauses)
    bool zoomIn = false, zoomOut = false; // ] and [ held: zoom
    // The editor (design 2026-11-09-skate-replay-editor); `leave` above is Tab or Backspace.
    bool tab = false, backspace = false, enter = false, del = false;
    bool pageUp = false, pageDown = false;
    bool keyT = false, keyN = false, keyI = false, keyO = false, keyU = false;
    bool keyK = false, keyJ = false, keyZ = false, keyX = false;
    float mouseX = 0.0f, mouseY = 0.0f;   // GTA's look normals this frame (0: mouse not in use)
};
struct Command {
    bool playPause = false, slower = false, faster = false, nextCamera = false, leave = false;
    bool openEditor = false;           // L3 or T: Skate's "Edit timeline"
    int filterStep = 0, lensStep = 0;  // -1, 0, +1 this frame
    bool hideHud = false;              // H or the right stick's click
    int frameStep = 0;                 // -1, 0, +1: one frame back or on (pauses)
    float zoom = 0.0f;                 // -1..1 held: zoom out / in
    bool reverse = false;          // held
    float scrub = 0.0f;            // -1..1, while paused
    float move[3] = {};            // free camera: right, forward, up, each -1..1
    float lookYaw = 0.0f, lookPitch = 0.0f;  // degrees to add this frame
};
// Edges come from the previous frame; the first frame after Reset only
// takes the held state, so keys and buttons held on entry do nothing.
// The editor modes read presses only (each mode gives them its meaning).
struct Edges {
    std::uint16_t pad = 0;             // buttons pressed this frame
    bool lt = false, rt = false;       // a trigger pressed past half this frame
    KeysIn key;                        // keys pressed this frame (true on the press)
};
class Input {
public:
    void Reset() { fresh_ = true; }
    // Presses since the last frame (either Update or Edges keeps the history).
    Edges Poll(const PadIn& pad, const KeysIn& keys);
    // `arrowsMove`: the Free and Orbit cameras, where the Up/Down arrows move
    // the camera up and down instead of changing the speed.
    // `paused`: LB/RB step one frame instead of changing the speed (as in Skate's editor).
    Command Update(const PadIn& pad, const KeysIn& keys, float dt, bool arrowsMove = false, bool paused = false);
private:
    bool fresh_ = true;
    std::uint16_t prevPad_ = 0;
    bool prevLt_ = false, prevRt_ = false;
    KeysIn prevKeys_{};
};
// Buttons and triggers held when the replay closes (keyboard: a held Q is Y,
// C is RT, Space is A) stay out of skating until each is let go, so leaving
// never steps off the board, grabs or pushes.
class ExitMask {
public:
    void Start(const PadIn& held) { buttons_ = held.buttons, lt_ = held.leftTrigger > 30, rt_ = held.rightTrigger > 30; }
    // Clears what was let go, then removes what is still masked from `pad`.
    void Apply(PadIn& pad);
    bool Active() const { return buttons_ || lt_ || rt_; }
private:
    std::uint16_t buttons_ = 0;
    bool lt_ = false, rt_ = false;
};
// A stick axis with the 20 % dead zone, -1..1.
float StickAxis(std::int16_t v);

// Lenses (design 2026-11-09-replay-look-sound): Auto keeps each camera's own
// field of view; the others are 35 mm focal lengths; Fisheye is 120 degrees
// plus the barrel pass.
enum Lens { kLensAuto, kLensFisheye, kLens14, kLens24, kLens35, kLens50, kLens85, kLens135, kLensCount };
const char* LensName(int lens);
// GTA's (vertical) field of view in degrees for `lens`; `autoFov` for Auto.
float LensFov(int lens, float autoFov);

// Film filters: Off, black and white, the Rockstar Editor's 25 film looks.
inline constexpr int kFilterCount = 27;
const char* FilterName(int filter);      // "Off", "Black and white", "Film 1".."Film 25"
const char* FilterModifier(int filter);  // GTA timecycle modifier; null for Off
// Steps an index by `step` with wrap-around.
int Cycle(int index, int step, int count);

// Replay sound: the GTA game time (ms) at the sample, -1 across a cut (silence).
double GameMsAt(const Recording::Sample& s);
// The sound's playback rate: 0 (silent) while paused, reversed or scrubbing,
// else the playhead speed.
double SoundRate(const Playhead& head, bool scrubbing);

// Back: a tap (pressed and released within `tapMs`, nothing else pressed
// meanwhile) opens the replay; held longer it stays the gun button.
class Tap {
public:
    explicit Tap(unsigned tapMs = 250) : tapMs_(tapMs) {}
    // Returns true on the release that completes a tap.
    bool Update(bool down, bool otherDown, unsigned nowMs);
    // The press now held (or the next release) is no tap: leaving the replay.
    void Block() { down_ = spoiled_ = true; }
private:
    unsigned tapMs_;
    bool down_ = false, spoiled_ = false;
    unsigned since_ = 0;
};

}  // namespace replay
