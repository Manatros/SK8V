# Resolve an adjacent bowl partner without weakening facing rules

Status: implemented and verified in isolated diagnostic box (`0eb21e5711ea49fb`).
Builds remain offline; baseline installed `31c00a7` untouched; prior connected-support
candidate `61fd2eaaf37f9dd9` preserved under `work/reassessment-20261028/v4-*` and
`box/payload/tiles-support-v4`.

## Problem and evidence

User: continue map cleanup, reassess working earlier solutions and align faces,
vertices and edges with visual models. The connected-support candidate restores
two M2 heights and protects rails/sidewalls, but leaves a small seam beside an
old chord approximately 0.19 m above the visible surface. It also exposes a new
1.008 m grind line on that residual chord. Do not promote it as a finished map.

`replace_park` comments describe nearest same-facing ownership, but the code
chooses the nearest face of any orientation and then rejects it on facing.
At the visible triangle covering (-1379.55,-1382.95), the nearest GTA triangle
is 0.067747 m away, dot 0.762689 (<0.766), area 0.163895 m2, normal.z 0.925463.
The next triangle is 0.105375 m away, dot 0.999506, area 0.154616 m2, z-normal
0.932977. They share a complete geometric edge with endpoints
(-1379.2859,-1382.8644,3.50256), (-1379.3136,-1383.9689,3.39997).
This second partner has local support 0.46837 m2 and breadth 0.52720 m.
It is a directly joined bank, not an unrelated background layer. The distance
difference 0.037628 m is below the existing 0.05 m PARTNER_SLACK.

Raw measurements: work/reassessment-20261028/M2-alternative-partners.json,
final-uncovered-gates.json, final-new-rails.json, delta-edge-distances.json.
These reconstruct surviving original variants, not a full pre-bake input;
fresh native geometry remains acceptance evidence.

## Decisions preserved

A49 two track edge lines, accepted SK8V physics/transitions and dynamic repair
31c00a7; amended A50 permits measured visible geometry. One complete matching
generation, Story-only inputs and active-state policy. No relabelled tiles,
old `.parks` reinjection, global rollback or invented collision primitives.
Do not simply lower CURVE_FACING: 45-degree ledge chamfers remain protected.
Do not search arbitrary same-facing background geometry behind a closer wall.
Keep all existing stair, deviation, cut/snap, ownership and material rules.

## Exact design

Change only visparks.rs candidate partner resolution. Keep the existing nearest
partner whenever it passes the existing facing gate. If it fails facing, an
adjacent partner may be considered ONLY when:

1. Original nearest distance <=MATCH (0.25 m); original nearest face area is
   smaller than SURFACE (0.25 m2). Both its normal and the visible normal have
   z >0.5. Keep old large-face rejection behavior.
2. Build/reuse the existing immutable SurfaceSupport topology. Consider ONLY
   direct full-edge neighbours with exactly two incident active GTA faces;
   never corner contacts, disconnected sheets or non-manifold edges.
3. Neighbour normal.z >0.5 and visible-to-neighbour dot >=CURVE_FACING (0.766).
   Its centroid-query distance must be <=MATCH and <=original nearest distance
   +PARTNER_SLACK (0.05 m). The actual shared edge must also lie within MATCH
   of the visible centroid (point-to-segment distance). Do not traverse to
   neighbours of neighbours.
4. Choose the nearest such compatible neighbour, breaking distance ties by
   lowest global GTA triangle index (`dj.total_cmp(&best_d).then(j.cmp(&best_j))`);
   require its existing local connected support (area >=0.25 m2 AND minimum
   breadth >=0.30 m). If it fails, reject; do not keep searching farther away.
5. Use this neighbour as the candidate's actual partner for `material` and
   `surface_from` ownership, while retaining the original nearest GTA face `i`
   as the candidate's displaced chord (`chord: i`):
   - At the M2 witness `(-1379.55, -1382.95)`, the visible triangle is nearly
     coplanar with the compatible neighbour `47360` (`deviation = 0.01036 m <
     CURVE = 0.03 m`), while its actual displaced nearest chord `47359` rises
     `0.20212 m` above it (which is also what tilted `47359` to `dot = 0.7627`).
   - Therefore candidate `deviation` is `dev(t, i).max(dev(t, partner))`, and
     each candidate records its deviation in `deviates` under its own nearest
     face `chord` (`i`) rather than contaminating a flat neighbour `j` (which
     would otherwise drag `47360`'s flat partner `vi=21516`, `dev = 0.67 mm`,
     into `used`). The candidate's whole-face deviation in the `used` filter is
     `deviates[&u.chord].max(deviates.get(&u.partner).copied().unwrap_or(0.0))`.
   - Because step 4 already verified `support.qualifies(partner, c)` when
     `u.partner != u.chord`, the `used` filter skips re-running `qualifies` for
     adjacent-partner candidates so `support_work` counts each query once.
   - Existing visible stair tests and all later admission/cutting/snapping
     remain. The `support` Option moves before candidate construction so both
     phases reuse one graph.

No change to SurfaceSupport geometry, large-face shortcut, generic rails,
dropper, tracks, source archives, whole-map policy or physics. A neighbour can
be a large face, but this path still explicitly validates its local breadth
and upward orientation before candidate admission.

## Tests and native controls

Production replacement regression with a closer incompatible small upward bank
and a shared-edge compatible bank: it must take the correct source's material
and surface key and use the actual visible mesh. Include a flat visible face on
the compatible neighbour to confirm it is not falsely admitted via `deviates`.
Existing code must fail first.
Negative controls: a disconnected compatible layer, corner contact/non-manifold
edge, vertical nearest source, large nearest source, neighbour outside MATCH,
distance >nearest+0.05 m, and distant shared edge. Existing stairs, ledge-over-
ground, rails, thin trims, upward breadth and topology controls remain.

Native: the missing M2 visible triangle must appear with correct parent ownership;
both restored heights remain; the 0.19 m chord seam and new 1 m grind artifact
must disappear or be held offline. No new deformation at previous wall witnesses;
preserve all old grind lines and current-better/M4 controls. Recount uncovered
samples, layer/error metrics, exact repeats, segment budget and Release work.
Only then stage a whole-map generation; inspect all park regions and verify it
before any matching local test installation. Gameplay acceptance remains open.

## Risks and rollback

Two adjacent surfaces can meet at a legitimate sharp feature. Upward-only,
small-original-face, direct manifold ownership, short distance slack, shared-
edge locality, strong facing and broad-support gates restrict the ambiguity;
they do not prove every neighbour correct. Native controls decide. Preserve the
existing 61fd2ea candidate and all earlier negative/positive evidence; installed
31c00a7 stays unchanged if the experiment regresses protected geometry.

## Goldfish questions and answers

Fresh-context Goldfish (`goldfish_partner`) checked source `31c00a7` and the
raw M2 evidence, catching that `vi=21514` deviates by `0.20212 m` from its
displaced nearest chord `47359` (`i`) but only `0.01036 m` (`< CURVE`) from its
compatible neighbour `47360` (`j`), and that keying `deviates` by `j` would
falsely admit `47360`'s flat partner `vi=21516` (`dev = 0.00067 m`). Step 4 and
Step 5 above incorporate the resolution (`dev(i).max(dev(partner))`, `deviates`
keyed by `chord: i`, deterministic `(distance, global_index)` tie-break, and no
duplicate `qualifies` call).

## Measured outcome (`0eb21e5711ea49fb`)

- **Unit & mutation tests**: 320 `skatev-runtime` unit tests pass (`adjacent_upward_partner_rescues_chord_seam_and_keeps_flat_neighbour_untouched` failed RED before the production edit and passes GREEN after). All 9 safeguard mutations in `work/reassessment-20261028/mutation_checks.py` (`external-edge`, `seed-cone`, `breadth`, `width-continuation`, `partner-slack`, `partner-edge-match`, `partner-qualifies`, `partner-chord-dev`, `partner-no-contaminate`) fail as expected (`mutation-results.json`).
- **Reconstructed park benchmark (Release)**:
  - 60,675-face expanded Vespucci input: `removed = 2740` (`+1` vs `v4`'s `2739`: the displaced M2 chord `47359`), `added = 13791` (`+2` vs `v4`'s `13789`), `support_work = (877, 2340, 7)` (`+3` queries, `+6` visited faces, worst-case still `7`), elapsed `233.3–241.9 ms` (`partner-work.log`).
  - 6,415-face one-tile input: `removed = 1309`, `added = 6868`, `support_work = (335, 856, 7)`, elapsed `104.3–107.5 ms`.
- **Native diagnostic box (`-1472 -1280 -1472 -1344`)**:
  - Bake time `20.99 s`, peak working set `~1.95 GB`, verified `11 invariant cells + 1 live cells, 74784 triangles, 1502 rail records` (`box/bake.log`, `box/verify.log`).
  - Deterministic rerun (`box/payload/tiles-partner-repeat`, `box/partner-determinism.json`): all 11 invariant `.svwt` tiles are byte-identical (`differences = []`).
  - Segment budget (`box/segments.log`): `2523 total, worst tile 450, worst 5x5 area 2523 at (-21, -21) (limit 52000)` (`-1` segment vs `v4`'s `2524`).
  - Control comparison (`partner-box-compare.log`, `box-metrics-control.json`):
    - `M2a = [3.311486]` (matches visible `[3.311486]` and `k` `[3.311486]`).
    - `M2b = [3.423688]` (matches visible `[3.423688]` and `k` `[3.423688]`).
    - `current_better = [4.629261]` (single surface preserved; no `k` triple-layer `[5.525, 5.162, 4.630]`).
    - `M4_coping = [3.314798]` (matches control `[3.314798]`).
    - `error_gt_3_8_20cm = [5638, 4234, 3498]` vs control `[5872, 4256, 3496]`; `improved_8_to_3cm = 24` (`+1` vs `v4`), `regressed_3_to_8cm = 1` (same 2.1 mm edge offset at `(-1394.25, -1425.65, z=8.902)`).
    - `new_uncovered_cells = 6`, `new_covered_cells = 6` (`7 -> 6`: the M2 chord seam at `(-1379.55, -1382.95)` is completely closed, and all 6 remaining uncovered cells are sub-centimetre triangulation seams `0.20–8.70 mm`, `partner-delta-edge-distances.log`).
    - `raildiff.py` (`partner-new-rails.json`): `lost = 0.0 m`, `gained = 0.4 m` (`1.4 m -> 0.4 m`: the `1.008 m` residual M2 chord grind line at `(-1379.2859, -1382.8644)`–`(-1379.5618, -1383.8357)` is completely eliminated; only the `0.360 m` line at `z = 8.923` remains).
    - `overlaps.py`: `131.9 m2` (`1.19%`), unchanged from `v4`.
