#pragma once
// Saved replays, Skate's "Save Replay" (design 2026-11-09-skate-replay-editor
// P3): the frames from In to Out (with a little margin), the edit and the
// sound, in Documents\GTAxSkate\Replays\*.svreplay. Every field is written
// explicitly (little-endian, no struct images); filters and the lens by name.
// No GTA calls here, so it can be unit-tested.
#include <cstdint>
#include <string>
#include <vector>

#include "replay_edit.h"
#include "replay_logic.h"

namespace replay {

struct SavedReplay {
    std::uint32_t modelHash = 0;      // the ped model it was recorded with (information)
    std::uint64_t savedUnix = 0;
    std::vector<Frame> frames;        // times from 0
    // The edit with times from 0 (the first saved frame).
    double in = 0.0, out = 0.0;
    std::vector<double> markers;
    std::vector<Clip> clips;
    std::uint32_t soundStartMs = 0;   // the game time of pcm[0] in the frames' gameMs
    std::vector<std::int16_t> pcm;    // 48 kHz mono
};

struct SavedInfo {
    std::wstring path;
    std::uint64_t savedUnix = 0;
    double length = 0.0;              // the edited video's length (s)
    std::uint32_t frames = 0;
};

// The bytes of a saved replay (built on the script thread, written by a worker).
std::vector<std::uint8_t> Serialize(const SavedReplay& r);
// Reads one; false with `error` set when it is not a replay of this version.
bool Deserialize(const std::vector<std::uint8_t>& bytes, SavedReplay& r, std::string& error);
// The header only (the list): false when unreadable.
bool ReadInfo(const std::vector<std::uint8_t>& head, SavedInfo& info);
// Bytes a header needs at most for ReadInfo.
constexpr std::size_t kInfoBytes = 64;

}  // namespace replay
