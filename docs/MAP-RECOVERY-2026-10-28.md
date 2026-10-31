# Map recovery investigation — 28 October 2026

Status: investigation and isolated test candidate; no gameplay acceptance or installation.

## Request and preserved decisions

Recovered Claude's last human prompt (27 October, 18:19 UTC): remove unseen and
extruding collision, restore the dropped rail's centred grind path, clean faces,
vertices and edges against visible geometry, reconstruct map history; replace
the skate shop's shelf boards and add a skater-style seller. The continuation
prioritises the map. The user confirms the rail shifts on fresh placement in
open space.

The user explicitly amended A50: GTA collision need not remain the base when a
measured alternative is better. This authorises evaluation, not an unmeasured
whole-world replacement. A49 still requires two edge paths per steel track rail.
SK8V retains animation, physics and transition ownership. Preserve the one-owner
surface policy, state-safe Story inputs, no invented fallback surfaces, and one
rules generation across runtime, baker and installed world. No weekly public
release before 2 November; local candidates are separate from releases.

## History recovered

Baseline: fork `b27eec69cf7e0f8f8ae8654bd6deaf2565938940`, the released v0.7.0
build. World receipt: rules `4217eedcd28b7deb`, source `fd794827c362d262`, 25,912
baked cells and 200 state-dependent live cells.

- The 19 October wholesale reset lost motion, prop adjacency, adaptive gathering
  and floor-swap safeguards. Reuse individual proven fixes; do not reset again.
- The favourable 20 October park test used live collision, while later tests
  used archive-baked inputs. It does not establish one universally good bake.
- Primary reassessment: the 25 October v0.6.0/`742dfda`/`84fc009` replays had
  world corrections disabled; the assistant acknowledged the invalid comparison.
  Their negative outcomes cannot reject the original scoped positive fixes.
  The Gemini combined replay does not independently reject each constituent fix.
  See MAP-HISTORY-REASSESSMENT-2026-10-28.md for original evidence and true k data.
- Duplicate walking/vehicle collision, mixed bake generations, state-variant
  contamination, generated rail boxes and duplicated park datasets are recorded
  failed approaches. They must not be reintroduced as cleanup shortcuts.
- High-resolution park replacement followed by cut/snap cleanup improved measured
  layering, but did not eliminate it. Neither the older nor current park is a
  universally accepted gameplay reference.

Sources: shared CLAUDE/BACKLOG, SOLUTIONS, FAILED-APPROACHES, SK8V-BUGS,
AI-AUDIT-2026-10-24, ARCHITECTURE, TOOLS, RAIL-CONTINUITY-2026-10-25, and the
25–26 October designs and recorded test results. Newest explicit decisions win
over historical descriptions in those files.

## Current data, not assumptions

Sample: six installed tiles intersecting a 90 m square around Vespucci
(-1370, -1405), recommended active variants. A bounded diagnostic found 21,623
geometrically unique active faces, no exact within-tile duplicates, no
nonfinite/degenerate faces, 11 faces below 2 cm² and 26 with area/longest-edge
below 3 mm. These counts use face-box intersection with the sample.

The existing overlap algorithm, with its same seed, normal agreement and
0.5–30 cm separation test, found 131.7 m² layered out of 11,093 m² (1.19%).
Its centroid-based sample contains 21,435 faces, explaining the different face
count. About 90.5 m² is GTA over GTA; the remainder includes cut/visible park
surfaces. This detects nearby parallel surfaces, not automatically unwanted
collision: each candidate still needs visible and ownership evidence.

Boundary edges and edges with more than two incidents were counted only as
diagnostics. Open terrain, detail overlaps and triangle-soup representation mean
that these alone do not prove a hole or justify global welding. The apparent
older k box carries the same source/rules and geometry as this sample; it is not
an independent historical control.

Four bounded visible exports completed with the existing release exporter:
actual shop (36,713 triangles / 28 models), bad railing controls (2,045 / 5),
good railing control (1,288 / 3), and the Vinewood road step (11 / 2).
The exporter reported six unsupported YTYP resources; retain this coverage limit.
Its `--surface` mode deliberately excludes glass and cutout shaders, among
others. Absence from these exports therefore MUST NOT be used as blanket
permission to delete collision. Glass, fences and incompletely exported assets
require an appropriate complete reference.

## Verified first repair packet

See DROPPER-ALIGNMENT-2026-10-27.md for the checked design, real-template
regressions and peer-review record.

1. Automatic approach geometry: the halfpipe source has 558 faces; the old
   loader installed 570 and extended about 1.29 m past each entrance. Remove
   the synthetic extension, preserving the source outline. This restores the
   original 0.217 m entrance step; a model-aligned entrance correction is still
   needed. Do not describe the whole halfpipe as fixed.
2. Narrow dropped rail: the existing height-by-centroid filter retains 22 of
   72 source faces, opens the bar underside, and retains angled support
   fragments. Origin-only extraction appears centred, but translated/yawed
   fixed-prop extraction reproduces a ±1.15 cm lateral knee and a low tail.
   This also occurs before world filters, with no ground present. Retaining
   the complete measured top-bar component is the proposed scoped correction;
   do not change the generic crown or centre arbitrary objects.

## Map strategy

Use a measured per-surface hybrid: keep reliable collision where it agrees with
the model, replace the specific mismatching component from a complete visible
reference, and remove superseded collision atomically. Do not add a second
surface under or over the replacement. This is a direction for checked designs,
not a claim that the whole-world pass is implemented.

Next pilot: material-64 railing slabs at the existing M2/M3 positions, with M4,
copings, stairs, decks, glass and cutout fences as unchanged controls. Reuse the
visible-bar design's component ownership and state safeguards, but do not
activate its broad candidate classifier without a measured census. Missing
objects require a source-completeness fix, not a fabricated box.

Optimisation order: remove proved duplicate/hidden ownership, repair locally
inconsistent boundaries, preserve material/step/grind discontinuities, then
benchmark any simplification. Global decimation and triangle-count targets
alone have already been rejected: they can create T-junctions and increase
collision-query candidate counts.

Before any whole-map change: box bake, visible overlay, floor-hole/overlap/rail
diff, positive controls, track coverage, deterministic rerun, segment budget
below 52,000, then one complete verified generation. The user's gameplay test
decides acceptance.

## Connected park support & adjacent partner candidate (`0eb21e5711ea49fb`)

See `docs/designs/2026-10-28-connected-park-surfaces.md` and
`docs/designs/2026-10-29-park-partner-ownership.md` for the full design,
Goldfish reviews, peer-review record, and progression across snapshots `v1`–`v5`:

- **Mechanism**:
  1. Small upward GTA bowl facets (`area < 0.25 m2`, `source_n.z > 0.5`, `visible_n.z > 0.5`) qualify when their connected manifold neighbourhood within the canonical `1 m` cube has clipped area `>= 0.25 m2` and minimum caliper breadth `>= 0.30 m` (`SurfaceSupport::qualifies`).
  2. When a visible triangle's nearest GTA face is a small upward chord tilted slightly past `CURVE_FACING` by chord displacement (`dot < 0.766`), `SurfaceSupport::adjacent_partner` resolves ownership across a shared manifold edge within `MATCH` (`0.25 m`) and `d + PARTNER_SLACK` (`+0.05 m`) to a qualified upward neighbour, while keeping the displaced chord's plane deviation (`dev(i).max(dev(partner))`) and keying `deviates` by the original chord so flat neighbours are not dragged into `used`.
- **Measured diagnostic box outcome (`0eb21e5711ea49fb` vs control `31c00a7`)**:
  - Restores both M2 bowl transition witnesses (`M2a = 3.311486 m`, `M2b = 3.423688 m`) to exact visible heights.
  - Closes the `0.19 m` M2 residual chord seam at `(-1379.55, -1382.95)` (`new_uncovered_cells: 7 -> 6`, all 6 remaining cells `<= 8.7 mm` from a shown triangle edge) and eliminates the `1.008 m` unwanted grind line on that residual chord (`raildiff.py` `lost = 0.0 m`, `gained: 1.4 m -> 0.4 m`; `segments.log`: `2523` total, worst 5x5 `2523 < 52000`).
  - Preserves `current_better` (`4.629261 m` single surface) and `M4_coping` (`3.314798 m`), passes all 320 `skatev-runtime` unit tests and all 9 safeguard mutations (`mutation_checks.py`), and produces 11/11 byte-identical `.svwt` tiles on deterministic repeat bake (`partner-determinism.json`).
  - Remains staged offline in `work/reassessment-20261028/box`; whole-map generation across all parks and user in-game testing are still required before any installation or acceptance claim.

## Shop evidence and limit

The current anchor is (-1224.45, -1442.68); the older surf shop is a different
location. Current presentation uses generic wall placement, not measured shelf
slots. The new bounded export contains a large `vb_29_stall_02` drawable and
other market/detail meshes, but establishes no separate placeholder-board
entity IDs. Do not hide the whole stall or replace a shared model globally.
First identify the board geometry/instances and slot transforms, then use one
owned reversible presentation and seller lifecycle. Preserve daily stock,
the eight display slots and the ridden-board registration reserve.
