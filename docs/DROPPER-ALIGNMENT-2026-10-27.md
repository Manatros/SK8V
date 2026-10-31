# Dropper collision outline and centred narrow rail — 2026-10-27

Status: implemented in isolated candidate; fresh design and diff reviews passed;
gameplay test and deployment pending.

Request: narrow object-dropper grind rail shifted to one side again; centre it. Current user asks for clean topology aligned with visible objects and removal of unseen protruding collision.

Baseline: b27eec6 (released v0.7.0 source); isolated codex/map-recovery-20261027 checkout. Actual installed prop_skate_rail template has 72 triangles; its straight top bar is x +/-0.035785675, y +/-2.46520996, z 0.301282..0.38392276 m. The existing centroid filter keeps 22 triangles, including angled ends, but deletes the bar underside. A static-area asset regression will identify actual final line positions before any correction.

Invariants: BACKLOG A49 train tracks retain two edge lines per steel rail; do not centre arbitrary thin props (FAILED-APPROACHES 10-23); A50 GTA collision remains the base, with the current user amendment permitting another base only where measured better. Preserve physics, model transforms and canopy filtering. Existing centroid deletion and end-flush vertex mutation remain outside the source-preservation claim; no NEW source vertex movement is allowed to centre grind lines. No game writes.

Confirmed unrelated generator: Templates::install calls entry_slopes for every model except the rail. Any flat outline 0.03..0.35 m above the lowest point produces invisible outward ramps at rise/run 0.25. This was the explicit 19 October halfpipe entry solution (SOLUTIONS); the current request to remove unseen geometry and A50 supersede it. Removing that generator preserves source collision but restores the halfpipe's original 0.217 m entrance step, pending a visible-model solution.

Design: remove automatic entry_slopes use and the now unused helper/constants/tests; replace with a regression exercising install on a raised source shape and verifying no added triangles/outward extent. Keep existing end-flush behavior. For the narrow rail, change only the demonstrated point of failure after RED asset evidence; do not synthesize collision shapes or move source vertices to hide a line error. If the line defect lies in world/rail extraction, coordinate that file with the parent agent.

Validation: RED/GREEN runtime regression; actual installed narrow rail template through install and world::build_area_tagged, with count, top-height, longitudinal span, and repeated-run determinism checks. Actual halfpipe source-vs-installed geometry count/extent check. Tests read assets only, never write production data. Parent performs peer review; user gameplay remains required.

Goldfish clarification: parent review identified ambiguity about existing end-flush/centroid policies and insufficient line assertions. Clarified retained exceptions above; asset test must check count, height, span and deterministic output as well as centring. Existing k diagnostics are the same active geometry, not an independent old baseline.

## Addendum 28 October: reproduced fresh-placement alignment defect

The original origin-space/no-ground probe passed. The complete fixed-prop path (Templates::gta_triangles -> world::build_area_tagged) at translation (3.25,-4.5,.02), yaw 0/.37/1.57/2.31 reproduces it. Ground is not responsible: prop-only and raw rails::find_metres already contain the same shift. At yaw0, top-bar endpoint x becomes +.011547089 m; yaw2.31 becomes -.011547089 m. Bent-end line fragments also survive. Raw evidence work/dropper-pipeline.log, exit101.

Actual source component probe (work/components-probe.cjs): 72 source faces form six disconnected, closed 12-face components using vertex coincidence <=1e-5 m. The unique long top bar has extent (.07157135,4.93041992,.08264071) m and z .30128205..38392276. Other components span at most .3219 m. Current centroid filter keeps 22 faces: bar without underside plus fragments of angled supports. Those fragments let the generic finder merge a sloping end into the top rail differently under transforms. Existing policy intended topbar-only but does not implement component ownership.

Revised narrow-rail design (only model 0x03154ddc): group source faces by coincident vertices (<=1e-5 m), require a closed component (each undirected triangle edge belongs to exactly two faces). Select exactly one near-top elongated horizontal component: XY long extent >=1 m; transverse XY and vertical extent <=.15 m; long extent >=10 times max(transverse, vertical); component top within .10 m of source max-z. Retain all original faces of that complete component including underside and caps. No new geometry or vertex moves. No changes to rails.rs/world.rs or train-track behavior. Existing end-flush remains, with this flat authored bar having nothing to flush.

If no unique component passes, preserve the full authored input collision and log a warning rather than silently deleting arbitrary faces. Add asset test that verifies 12 original faces survive (with underside), source coordinates remain exact, and component selection is unchanged when source face order reverses. Add a synthetic two-component ambiguity test requiring failure/preservation. Existing fixed-prop rotated/translated asset test retains count/span/height/centering/determinism assertions. This correction also removes the unwanted sloping end fragments, so requiring the one long line to remain at top height is now meaningful.
Connectivity clarification for implementation: canonicalise coincident source vertices within 1e-5 m, connect faces by a shared complete undirected edge (two canonical vertices), not a corner-only contact. Actual six source boxes are unchanged by this stricter component definition. Component closure requires all undirected edges to have exactly two incident faces and no degenerate canonical triangle. Work is bounded to this model's small source template and executed once when the model is loaded.

Fresh goldfish_component review (28 October): READY after specifying an order-invariant full pairwise vertex proximity graph (Euclidean <=1e-5 m), deterministic canonical vertex IDs, shared-edge components and collapsed-triangle rejection. Thresholds are inclusive. Zero/multiple matches preserve selector input and warn. Required tests: exact 12 source faces including caps/underside, reversed-input set equivalence, exactly one total rail at four transforms with endpoints/top/span/centring/determinism, and bypass for another model. Implementing only that checked design.

Resource guard (root and peer_dropper supplemental review, 28 October): the
pairwise selector accepts at most 512 source faces (actual template: 72).
Larger replacement assets return None before vertex allocation or welding,
preserving the full input with the existing warning. This bounds only the new
selector's quadratic work; downstream processing still receives the preserved
input. A 512/513-face boundary regression checks unique-bar admission followed
by preserve-and-warn rejection, without relying on malformed geometry.

Implementation evidence (28 October): selected complete authored top bar, unchanged 12 original faces including underside/caps; no rail/world algorithm edits. Focused `cargo test --offline -p skatev-runtime 20261027 -- --include-ignored --nocapture` passed 5/5, exit0 (`work/dropper-component-green.log`). Real fixed-prop path with ground and four translated/yawed transforms now yields exactly one straight line at z .38392276 m, y +/-2.46521 m and maximum lateral error below .000000835 m. Prior same probe failed at 3/4 yaw values, with 11.547 mm lateral shift and an unwanted sloping tail (`work/dropper-pipeline.log`, exit101). Original halfpipe test failed 570 vs558 source faces; now558 vs558. Removing automatic approach slopes was RED/GREEN separately (`work/slopes-red.log`, exit101). Normal ambiguity/open/degenerate/order/bypass tests passed. No deployment, game writes, or gameplay acceptance.

Fresh diff peer review: peer_dropper (28 October), no blocking behavioral
findings against the checked design and actual diff. Supplemental resource-cap
review also found no blockers. Reviewer did not build or test in game.
Root independently checked the implementation and ran the final six focused
tests (including real rail/halfpipe assets and the 512/513 resource guard):
6 passed, exit 0, `work/final-focused.log`. The earlier runtime suite passed
309 tests with 20 explicitly ignored; the final guarded runtime suite passed
310 with 20 explicitly ignored, exit 0 (`work/runtime-final-tests.log`). The
Release host also built successfully. No gameplay result is inferred.

Remaining limitation: preserving halfpipe authored geometry restores its raised entrance step; a visible authored solution remains gameplay work. Unsupported changed rail templates preserve full authored geometry and issue a warning rather than applying this component policy.
