# Recover continuous visible bowl surfaces

Status: first candidate tested offline, not promotable; refinement below pending
fresh-context check. No rule change or bake installed. Baseline fork 31c00a7.

## Problem and authorization

28 October: the user asks to reassess earlier map solutions, including ones
possibly rejected even though they worked, and continue toward the best map.
The earlier explicit A50 amendment permits a better collision source when
measurements support it. This design recovers a measured part of the earlier
visible-bowl solution rather than resetting the world or reinstating all of k.

## Evidence

Primary evidence distinguishes positive scoped outcomes from whole-build
rejection. The 25 October v0.6.0/742dfda/84fc009 replay had world corrections
disabled; Claude explicitly acknowledged that those comparisons were invalid.
Original feedback positively assessed the 20 October bowl, the 22 October
geometry/performance, and the 26 October high-resolution bowl, with remaining
seam/step/join problems. Old .parks, incomplete hi@ ground, and duplicated
fallback layers are different experiments and must not share one failure label.

True historical data: k is world-recovery-083de508efec4eaabf053671da853018,
rules 498ed7147e728bf3, source 93fed1e3eace23c3. It has less visible-surface
height error in the park than current l, but also locations that l improves.
The similarly named build/k box contains current data and is not a control.
Scattered uncovered raster cells mostly describe narrow details, not rider holes.

The M2 transition supplies a concrete witness. Near (-1379.47,-1384.35), l
keeps a GTA face at z 3.3693 instead of visible/k z 3.3115. Its original face
area is 0.194890 m2; visible centroid-to-partner distance 0.04938 m, normal dot
0.99139, maximum visible-vertex plane deviation 0.14923 m. All current geometric
admission gates pass except the 0.25 m2 per-triangle area cutoff. A neighbouring
0.193787 m2 partner fails the same cutoff. The cutoff was added in 24358d4 to
protect rails and trims, but also rejects facets of a continuous bowl.

Reconstruction limit: these partner measurements restore the surviving original
GTA variants in the installed tile; they are not a complete pre-bake dump.
Therefore a fresh box bake and visible comparison remain required.

Coplanar adjacency alone does not recover the witness. Its legitimate neighbours
turn about 26–34 degrees. Unbounded gentle connected area is also unsafe: thin
tubes can wrap through successive normal turns and long trims can accumulate
arbitrarily large area. A bounded support probe gives the two witnesses about
0.293 and 0.558 m2 respectively. Archive/tag restrictions incorrectly break the
real bowl at the vb_34_0 / vb_34_1 boundary; use geometric continuity in the active
input, while retaining each replaced face's own material.

Evidence scripts and numeric snapshots: work/reassessment-20261028/
historical_raster.py, partner_gates.py, coplanar_partners.py,
gentle_partners.py, gentle_patch_probe.py, local_support_controls.py and JSONs.

## Decisions preserved

- A49: two separate edge paths per steel track rail; no generic rail change.
- SK8V animation/physics/transition ownership; installed dynamic fixes 31c00a7.
- State-safe Story inputs and one complete generation with matching rules.
- No collision fallbacks or invented boxes; one owner per replaced surface.
- High-resolution curves, current stairs exclusion, 3 cm deviation requirement,
  40 degree facing requirement, existing cut/snap and hidden-layer handling.
- FAILED-APPROACHES: no whole-version reset, generic decimation, material-based
  invisible filtering, injected old .parks layer, or mixed bake hashes.

## Design

Change only the small-face admission in visparks.rs::replace_park. Existing
faces at least SURFACE=0.25 m2 retain their current admission path. A smaller
partner may pass only if it belongs locally to enough continuous surface:

1. Build a deterministic edge adjacency graph for finite, nondegenerate GTA
   triangles already in this park's active input. Use the existing 0.1 mm
   geometric key scale for topology only; do not move any vertices. Connect
   faces only across a full undirected edge with exactly two incident faces.
   A corner touch or non-manifold edge cannot provide support.
2. For a visible candidate, take the existing nearest GTA partner. Centre the
   probe at world::closest_on_triangle(visible centroid, GTA partner), rather
   than the source face's centroid. The query point remains on the same surface
   under simple subdivision. Form a canonical orthonormal frame from the seed
   normal and the world axis least aligned with it, not a triangle edge.
3. The local region is a cube in that frame with half extent sqrt(SURFACE),
   0.5 m on each axis. Clip actual source polygons against all six planes.
   Count polygon area inside the cube, never whole-triangle or centroid counts.
4. Traverse only connected faces with positive clipped area, with both the
   neighbour-to-current normal dot and neighbour-to-seed normal dot at least
   CURVE_FACING=0.766. This prevents transitive wrapping around a tube. Do not
   cross an outside face to reach a separate interior patch. The connecting
   shared edge must itself intersect the cube: two positive clipped faces
   meeting only outside cannot donate support to each other.
5. Admit when accumulated clipped support reaches SURFACE=0.25 m2; stop early.
   Reuse visited-generation and stack buffers per park, avoiding an allocation
   proportional to the whole park for every query. Topology construction is
   O(edges log edges) using sorted edge keys. Each query is O(local faces),
   potentially O(park faces) for dense rejected geometry; physical bounds do
   not bound triangle count. Measure candidate/visited counts and native bake
   and live-area timing before promotion; do not add an arbitrary face cap.
6. Do not partition by archive name or packed audio tags. A continuous bowl can
   cross those boundaries. The existing per-partner material/surface mapping
   remains untouched. No outside/inactive geometry is consulted.
7. All existing candidate distance, facing, non-downward, stairs and deviation
   gates remain. Existing whole-partner cut limit and cut/snap rules remain.
   Rejected small faces stay GTA's; the helper never directly removes geometry.

Scope of subdivision claim: the new clipped-support predicate and broad-surface
regressions. The existing large-face shortcut is preserved, so this does not
claim global tessellation invariance of every legacy admission or deviation rule.

## Tests and measurements

- RED/GREEN: small connected facets of a bowl/wall must receive the same visible
  curve correction as the unsplit broad surface; current area cutoff fails.
- Local support: broad surfaces pass with coarse/fine triangles; a 5 cm by 2 m
  or 20 m strip stays below the cutoff even after fine subdivision and rotation.
  Finely tessellated 5 cm diameter long tubes cannot gain support by wrapping.
- Disconnected nearby geometry, corner-only contact, folded-back faces,
  non-manifold edges and out-of-cube bridges do not donate support. A continuous
  material/archive seam is not artificially disconnected.
- Existing rail/trim and flat-match controls, stairs, back faces, ledge/ground
  ownership, no-layer and outside-park tests must pass.
- Unit-test geometric clipping and repeated/reversed source order. Measure
  support helper work on the real reconstructed M2 inputs, without treating
  reconstruction as an exact baker-input acceptance test.
- Fresh diagnostic park box: measure M2 profiles against the visible model,
  current and true k; overlay source/visible seams. Preserve M4 railing/coping,
  stair coverage, currently improved k-worse witness (-1364.1,-1412.85), outer
  deck/ledge rail continuity, thin geometry and negative controls.
- Before promotion: floor coverage, nearby-layer counts, rail diffs, native
  verify and worst 5x5 segment count <52,000, a deterministic repeat of the
  diagnostic box.
  If improvements are selective, retain the measured better surfaces and reject
  the wider variant rather than installing it wholesale.
- Then build and verify one whole-map generation through the official script;
  install its matching pair/data transactionally with a recovery backup. Never
  install the diagnostic box or relabel old tiles.

## Risks and rollback

Locally overlapping source geometry could inflate support, and a wide enough
trim can qualify. Manifold adjacency, the seed cone and bounded clipping reduce
those risks but do not prove every map surface correct; the real box controls
are mandatory. Preserving the original large-face path limits regression scope.
If measurements do not improve the transition without regressions, keep the
installed 31c00a7 build and record the unsuccessful experiment. Rollback after
promotion restores the complete matching prior pair and world, never slices.

## Goldfish questions and answers

Fresh-context Goldfish checked source 31c00a7 and accepts implementation after
the shared-edge intersection and performance clarifications above. Retain all
existing gates. A failed neighbour-to-current normal test must not mark a face
visited: another qualifying neighbour may reach it. Installation is conditional
on actual final geometry, controls, budget and a verified whole-map generation.

## Native outcome and bounded refinement (29 October continuation)

The first candidate (rules e1f569479c52e6c7) passes native verification, has
2,523 worst 5x5 grind segments, and repeats byte-for-byte across 11 baked tiles.
M2 improves exactly to visible at both witnesses; current-better and M4 coping
controls remain unchanged. A fresh old-rules control reproduces the installed
raster, ruling out a different input or diagnostic-box boundary explanation.

It is not promotable: 16 newly uncovered raster samples (7 on visible upward
surfaces, nearest candidate edges roughly 0.2–8.7 mm away); 26 new >8 cm error
samples; 18.4 m lost/8.0 m gained grind-line samples. Layering improves only
131.7 -> 131.5 m2. Two genuine extra sloping cut faces appear above lower floors.

At (-1369.95,-1422.15), the original wall normal z is 0.01872 but its leftover
cut/snap fragment has z-normal 0.61707. At (-1359.85,-1411.65), a vertical wall
gets a fragment with z-normal 0.50991. Reconstructed production replacement
reproduces both exact fragment vertex arrays and identifies their source faces.
The newly admitted visible snap targets are vertical/nearly vertical sidewall
details (z-normal 0 and 0.13863), not the upward bowl transition being rescued.
The source/target normal dots are 0.9148/0.8751: merely tightening their facing
gate to 0.766 would not solve this. Do not claim the whole cut/snap algorithm fixed.

Refinement for this candidate: restrict ONLY the additional small-facet path to
upward source AND visible faces with normal.z > 0.5 (within 60 degrees of up),
the same upward-surface class used by the diagnostic floor raster. Keep the
large-face path and every existing gate unchanged. This deliberately scopes the
rescue to bowl floors/banks. It does not claim all small vertical bowl walls are
optimal; their existing GTA ownership remains until a separate complete visible
sidewall/boundary-ownership design handles their end edges without sloping
leftover fragments. The witnesses have source z-normals 0.788/0.953 and visible
upward faces; both must remain corrected. Preserve the rejected first variant
and all evidence instead of describing connected support as generally broken.

Tests: add production-path small vertical and slanted-sidewall controls whose
visible shape otherwise passes connected support; confirm the refinement keeps
their original GTA faces while the upward wall/chord regression still passes.
Keep geometric support tests unchanged (helper remains orientation-neutral).
Then new native diagnostic box and exact old-rules control comparisons: no
new deformed-wall witnesses, classify any remaining floor seams, rail changes,
layer changes, deterministic repeat and budget. Do not promote solely on M2.
If the narrower rescue still harms protected geometry, keep it offline and
continue the boundary-ownership investigation; no arbitrary per-location masks.

Fresh Goldfish (goldfish_upward_rescue) correctly restated this refinement and
approved the bounded production edit, not promotion. It notes that upward used
targets can still cut/snap steep neighbours; full output controls remain required.

## Local breadth refinement — proposal before production edit

The upward-only native candidate (rules 29eacb9f79af5bfe) preserves both M2
improvements and removes the two exact sidewall deformation witnesses. It still
has seven uncovered samples and seven >8 cm regression samples against the fresh
control. Remaining high extra fragments are around narrow upper coping/trim
surfaces; one removed decorative high point also affects the raster. No promotion.
Area alone cannot distinguish a long narrow strip from a broad bowl patch.
The current cube can even admit a rotated 0.2 m strip (a diagonal chord inside
the cube is longer than its side). An orientation-independent local breadth
test addresses that mechanism without changing source ownership or snapping.

Additional condition inside SurfaceSupport only: the convex hull of contributing
clipped polygons projected into the canonical seed tangent plane must have
minimum caliper width at least sqrt(SURFACE)/2 = 0.25 m. For every hull edge,
measure the projected point span along its perpendicular; take the minimum.
This is independent of the tangent basis rotation and measures breadth rather
than its axis-aligned box. Require BOTH the existing 0.25 m2 clipped area and
this width. Retain the upward restriction on source and visible candidates.

Accumulate clipped polygon vertices with reused per-park buffers. After area
reaches threshold, form a deterministic monotone-chain hull (sort exact float
coordinates with total_cmp, deduplicate exact coordinates). Do not stop on a
width failure: continue connected traversal, because later connected support
can supply legitimate breadth. Stop only once both predicates pass. Reuse hull
and point buffers between queries. Do not cap face count or alter adjacency,
the cube, normal cones, candidate selection, cut/snap, or large-face admission.
The hull fills concavities, so area and connectivity remain mandatory. A
branched trim can still defeat this predicate; actual negative controls decide.

Exploratory reconstruction gives M2 local minimum widths 0.32344 and 0.64233 m,
above the proposed 0.25 m threshold, with the original 0.29336/0.55977 m2 support.
Unlike the previously considered area/perimeter proxy (first witness only
0.2032 m), this does not discard the measured good transition. Script and output:
work/reassessment-20261028/local_width_probe.py and local-width.jsonl.

Tests: rotated narrow strips at 45 degrees must fail independent of their area;
broad subdivided/rotated surfaces and genuine connected curved support pass.
Direct hull controls include collinear points, duplicates, a rotated rectangle,
and a broad patch reached after an initially narrow fragment. Keep the existing
production and topology controls. Repeat performance measurement: hull work is
O(p log p + h*p) after threshold for p clipped points/h hull points and can recur
on rejected dense geometry. No whole-map promotion until actual native negative
samples, seam widths, line ownership and budget are resolved or explicitly
bounded against the project's recorded acceptance tolerance. Preserve all prior
variants, exact witnesses and positive M2 evidence.

### Native breadth outcome and final calibration proposal

The 0.25 m breadth candidate reduces >8 cm regression samples 7 -> 2 and new
unmatched grind samples 8.0 -> 3.5 m. It keeps both M2 witnesses exactly visible,
all current-better/M4 controls, 318 unit passes and native verification. The
remaining unwanted upper-coping fragment is at (-1388.65,-1408.55). Independent
source reconstruction identifies its newly admitted upper targets as a strip
only 0.2516 m broad; other newly admitted coping strips are 0.2553 m broad.
The existing 0.25 m breadth threshold therefore remains too close to the actual
trim size. Propose the universal new-path minimum breadth **0.30 m** instead.
This is calibration against actual unwanted geometry, not a geographic mask.
The two desired bowl witnesses remain 0.3234/0.6423 m broad; the first has a
23 mm margin, far larger than measured coordinate quantization. Do not increase
the cutoff further merely to erase a counterexample. Thin/medium targets retain
their old GTA ownership; the preserved large-face shortcut is unchanged.

The second raster regression is a narrow high decorative surface at z8.9, with
large local breadth. It must be classified geometrically rather than hidden by
a size cutoff. Seven new uncovered samples are the same small seams; measure
their edge distances and compare the project's earlier no-new-floor-patch >20 cm
acceptance criterion, while retaining them as open geometric imperfections.
Do not call them solved or claim gameplay acceptance. The final candidate needs
old-rules controls, zero new deformed-wall witnesses, inspected grind-line
changes, deterministic native repeats, full-generation verification and a user
test. If either remaining sample is a substantial unsupported surface rather
than a sub-centimetre seam, hold the candidate offline.

Performance with the breadth check: expanded 60,675-face reconstructed input
takes 232–237 ms, with 874 support queries / 2,334 visits, maximum 7 visits.
Mutation checks independently detect edge intersection, seed-cone removal
(30 cm diameter tube) and breadth removal (20 cm strip at 45 degrees). The
strengthened narrow-neck fixture additionally detects premature rejection on
width failure (reverse traversal), after widening its narrow section to 24 cm.

Performance: reconstructed one-tile production path 110–112 ms, 716 support
queries / 2,216 visited faces, maximum 11 visited per query. Expanded 60,675-face
surviving-input reconstruction takes 257–277 ms, 2,246 queries / 7,533 visited,
maximum 11. These are Release exploratory measurements, not GTA gameplay timing
or exact pre-bake acceptance. Native first box takes 23.2 seconds / ~1.96 GB peak.
