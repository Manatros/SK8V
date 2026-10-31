# GTAxSkate integration notes for SK8V

This fork extends [SK8V](https://github.com/becaring/SK8V) 1.1.10. It holds the
source, tests and build tools needed to inspect and integrate our additions.
Player installation belongs in [GTAxSkate](https://github.com/Manatros/GTAxSkate).
This is a combined integration tree; the areas below are logical boundaries,
not a claim that independently tested feature pull requests already exist.

## Preserve the upstream core

SK8V owns skating, animation, audio, transitions and state. Entry uses its weapon
wheel; hold Y (E on keyboard) to leave. Our additions fill demonstrated gaps and
provide optional host services. Skate engine changes are recorded in
`patches/skate/` and applied by `tools/prepare-skate-overlay.ps1`; do not substitute
a second skating engine. Required host/runtime ABI checks still apply: distribute
the tested pair rather than mixing arbitrary versions.

## Integration boundaries

| Area | Source and purpose |
|---|---|
| World collision/grinds | Runtime `world.rs`, `live.rs`, `tiles.rs`, `rails.rs`, `visrails.rs`; `rust/world-cache`. Preserve active GTA states, fill demonstrated gaps and derive rails from authored geometry. Contract below. |
| Vehicles/props | Runtime `dynamic.rs`, `worker.rs`; host entity gathering. Prefer per-instance bounds. A fixed prop leaves both dynamic triangles and rails only after its matching static version is installed. Detached car parts must not inherit whole-car collision. |
| Object placement | Host `object_dropper.cpp`, `object_dropper_logic.h`. Resolve the selected model and height after this frame's movement/lift. Manual lift is −4 to +4 m because GTA's ground query can select overhead surfaces. |
| Keyboard | `rust/skate-input`, runtime keyboard exports and host input/menu. Emulate controller input while preserving upstream entry/leave; an idle controller must not suppress active keyboard input. |
| Board shop/parts | Runtime `shop.rs`, `board_shop.rs`, host board parts and preparation tools. Extract the player's own disc assets; preserve the animated board skeleton and skin flags. Static replacements can break trucks/wheels. |
| Camera/tuning/radio | Host camera/menu/radio and runtime tuning exports. Keep additions optional and preserve upstream behavior unless a preset explicitly selects alternatives. |

## Collision data contract and rationale

The former city bake combined mutually exclusive map states, including intact
and destroyed buildings, and treated every present tile as authoritative. Its
rail exporter also scanned Online-only packs. Solid boxes derived from absent
models can therefore obstruct Story Mode. Valid file checksums alone do not
establish correct geometry.

1. Share the Story archive order in `world-cache/src/vfs.rs`; honor enabled patch
   startup archives through `packs.rs`. Reject unreadable required metadata.
2. Validate manifests before classifying groups. Script, time and weather
   dependent groups need GTA's active state. Place local MLO bounds through
   their instances, never at world origin.
3. Bake invariant cells. The mandatory `live-cells.txt` policy binds rules,
   inventory, size, count and body digest; tiles also bind its digest.
4. Read active geometry for live cells before expensive cleanup/rail finding.
   Keep 13 m context for closed-solid cleanup, then clip to cell ownership.
   Never overlay baked and live collision for the same cell.
5. Do not refill those cells from the old raw fallback: it also contains alternate
   floors. Watch for later loads even when an active state currently has no
   bounds. Invalid policy on an existing enabled dataset must retain the prior
   area rather than silently enabling unrestricted cached floors.
6. Regenerate visible rails under the same Story/state policy. The adjacent
   `.visrails.policy` receipt binds exporter policy and exact bytes. Old all-DLC
   exports are rejected. Live patches retain active physics tubes and omit
   static visual/track sidecars; sidecar-only grind lines may be absent there.

`tiles::rules_hash` covers archive selection, extraction, placement and final
surface rules, with canonical line endings. Semantic changes require matching
data regeneration; relabeling old output with a new hash is unsafe. Disabling
`BakedWorld` explicitly selects the legacy live/fallback path.

Collision view uses closest surface/segment distance. Corner/centroid selection
hid large nearby walls while they still collided; drawing remains capped.

## Build, validation and remaining limits

Follow [BUILDING.md](BUILDING.md) for the paired build, pinned dependencies and
overlay preparation. Focused checks from the repository root:

```text
cargo test --offline --manifest-path rust/Cargo.toml -p skatev-runtime --lib
cargo test --offline --manifest-path rust/Cargo.toml -p skatev-world-cache --all-targets
```

Host fixtures are under `host/tests`, covering placement, menu, camera and board
parts. Data-dependent tests are ignored by default and take explicit local paths
through environment variables. Never commit archives, keys or extracted assets.

Export verified rails, bake a representative small region, verify source/policy
identities and coverage, then rebuild the city. Complete coverage means baked
cells **plus intentional live cells**; do not endlessly retry live cells as
failed tiles. Command forms are in [WORLD-BAKE.md](docs/WORLD-BAKE.md).

Tests establish invariants, not gameplay acceptance. Compare upstream and the
candidate at the same location/state: open streets, changing states, bridges,
metro contact performance, grind transitions and placed props. The current
collision correction still requires gameplay validation. Moving-object grinds
and repeated-pattern stair recognition remain unresolved. Propose broader work
separately with a specific reproduction and meaningful tests.

## Repository and license responsibilities

Keep implementation, meaningful tests, integration rationale and required build
tools here. Keep player launchers, installation payloads and user help in
GTAxSkate. Exclude internal task logs, AI instructions, experiments and generated
game data from both public trees. Preserve attribution, component licenses and
source availability for distributed binaries; this fork supplies no proprietary
game assets or keys.
