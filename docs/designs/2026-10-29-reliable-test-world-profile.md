# A reliable world profile for local test builds

Status: draft before production edits. Installed Argon candidate has matching
runtime/world rules 0eb21e5711ea49fb; no rollback or further terrain-rule edit.

## User request and measured problem

29 October feedback to Argon: use one best set of collision settings for every
test; change it only to isolate causes, so working fixes are not misjudged.
Current Codex request: audit Argon's intervening work, preserve it, continue and
deliver a testable build. Preserve personal controls, equipment and other settings.

Argon correctly completed the checked connected-bank/adjacent-partner design,
including original-chord deviation ownership; installed hashes and all 25,912
baked +200 live cells independently verify. Original test at 12:46:09 nevertheless
logged `custom: Grind every stair step=1, Unmarked stairs as steps (test)=1`.
The INI was reset after the run at 13:09. Do not discard the user's bowl feedback:
HighResParks was on, and no stale-generation or plain-GTA comparison was logged.
The crash file contains only memory telemetry, not a recorded fatal exception.

Current main.cpp duplicates best values in GtaxSkatePreset, BestWorldSettings,
WorldAtBest and kWorldSwitches. WorldState also omits DynamicWorld, so it can
say best while moving/placed object collision is disabled. Manual file repair
does not prevent another test inheriting old experimental switches.

## Decisions preserved

One matching static generation and Story input; A49 two steel edge paths;
SK8V animation, physics, camera and transitions; authored dropper fixes. No
old-version reset, new collision boxes, blanket prop bake or material deletion.
No unrelated control, visual, audio, board or trainer changes. The installed
source remains uncommitted and is snapshotted before further edits.

## Design

Scope host/src/main.cpp, host/CMakeLists.txt and host/tests/world_settings_tests.cpp.

1. Make existing kWorldSwitches authoritative for best values. Derive
   BestWorldSettings and WorldAtBest from that table. GtaxSkatePreset appends
   that generated vector instead of duplicating those keys/values.
2. Add DynamicWorld to that table with a named host-only runtime id sentinel.
   SendWorldOptions must skip the sentinel: never send it to the runtime ABI.
   Move its existing Nearby entities collide row into the world-switch group;
   do not create duplicate menu keys. Other runtime option IDs remain unchanged.
3. Add CMake option GTAXSKATE_TEST_WORLD_BASELINE, default OFF. Only the local
   test host is built with ON; normal/release builds preserve saved startup
   behavior. Expose it as a boolean compile definition on the host target.
4. A pure helper normalizes a Config's collision switch members to the table's
   best values only when that option is enabled. Call it on the local config
   used by EarlyRuntime and on g_config immediately after ScriptMain loads it,
   before runtime options or dynamic object gathering. EarlyRuntime must not
   touch global comparison state. ScriptMain resets plain-GTA comparison only
   for the enforced local test start/restart.
5. Log the test baseline and previous saved world state in ScriptMain; the
   existing actual `world settings: best/custom/COMPARE` log and status row
   remain authoritative during play. Manual menu comparisons remain available
   for the current session and still log changes. The next local test startup
   restores the baseline. No persisted INI, preset, binding or board file writes
   are added by normalization; preserve personal preferences and avoid startup
   write-permission/partial-save failure paths.
6. Package manifest/TESTPLAN explicitly say best-at-startup and describe manual
   diagnostic changes. Metadata is descriptive, not used as a binary-integrity
   or compatibility bypass. New host pairs with unchanged verified runtime and
   complete world rules 0eb21e5711ea49fb. No static rule hash or tiles change.

## Tests

- Enforced config starts with old StepRails/Unflagged enabled, HighResParks and
  DynamicWorld disabled; normalizes to the single best table. All unrelated
  settings remain exact. Disabled normalization preserves custom world values.
- Best vector has every table key exactly once, preset has no duplicate keys,
  status recognizes DynamicWorld off, and runtime emission never includes the
  host-only sentinel. Existing runtime IDs, baked-world hard-on and comparison
  behavior remain. Adjust menu-position assertions to derive from table size.
- INI fingerprint remains unchanged during pure normalization; stored User
  preferences survive. Existing menu best reset, presets, save-text, camera,
  interaction and dropper tests remain required where affected.
- Host Release build with ON; prove default build option is OFF. New test
  manifest hashes, runtime/world unchanged, source patch and rollback fixtures.
  Gameplay startup must log best; intentional changes must log custom/COMPARE.

## Risks and rollback

Forgetting the local test CMake option would reproduce the old problem; record
the configured option and artifact hashes. A release accidentally built with
ON would ignore saved custom world preferences at startup, so do not change the
default or reuse this build directory for publication. Reordering table rows
changes menu indices; verify existing controls and one row per key. DynamicWorld
is host-only and must never reach sv_set_world_option. Config-only normalization
does not persist baseline preferences; this is intentional and logged for local
test binaries, not a claim that the saved INI was repaired.

Rollback restores the verified previous host/runtime pair. World data stays the
same matching generation. Existing user INI/controls/board data are untouched.

## Goldfish questions and answers

Fresh Goldfish (goldfish_test_profile) restated and approved bounded implementation.
MenuPages has both raw rows and later explicit grouping: move DynamicWorld in
both places, with no duplicate key, and wire WorldChanged. Fixture compilation
must independently exercise ON and OFF macro paths. Capture the stored state
without g_plainGtaCompare obscuring it; only ScriptMain mutates comparison.
Keep FirstRunPreset unchanged: the new normalizer performs no writes, rather
than claiming the whole old startup path never writes. Explicit baseline logs
must appear on script restart even if the deduplicated state already said best.

## Implemented verification

Release host ON built successfully; a separate fresh CMake configure proves
the option defaults OFF. All nine host fixtures pass, including independently
compiled world-settings ON and OFF modes, menu/preset/save-text permissions,
camera/interaction/dropper/prop controls. The fixture checks actual INI bytes
are unchanged by normalization and personal Config fields remain exact.
Fresh peer review caught the DynamicWorld destination routing omission; it is
now resolved by canonical table membership, not another hardcoded key list.
Final peer review reports no remaining actionable defects in this scope.
Independent runtime 320 pass/21 ignored and full installed-world native verify
pass; runtime/world stay 0eb21e5711ea49fb. Gameplay startup remains untested.
