# Park rules revision 5 implementation

Status: local implementation candidate; gameplay acceptance and integration peer review pending.

Implementation: Codex gpt-6.1-sol, completing Claude Fable's unverified `297d967` on `feat/park-rules-rev5`, base `1e1b77a`. Scope: checked design revisions 5/5b/6, BACKLOG A49/A50/A52/A56 and B137.4. The external evidence report is `build/test-review/2026-11-16-rules5/report.md` in the workspace.

## Decisions and hooks

- R1b widens only the railing-only box rule to the 20 m reach ring. Glass, stairs, insufficient bar evidence and solid visible walls preserve GTA collision. The existing exact-source La Mesa manual exception remains separate. The ownership mesh is clipped; use the checked design's solid-wall fallback, intersecting actual visible triangles with the broad-side 5 cm slabs rather than testing their centroids. All removed ring-box vertices must stay in the ring.
- R4 runs after restored bar-top lines and before the final crease/duplicate passes, in baked and live builds. It grows an existing ridge line, preserving the finder's original start rule. A continuous crest needs 2 cm above the plateau and a 7 cm drop on the low side at 0.5 m. Missing crest samples are bounded to 0.5 m. The 15 m maximum applies to the total 3D extension of both ends of one line; this corrects revision 6's per-end wording to revision 5b's per-line requirement. Other lines and park boundaries stop growth; extensions produced earlier in the pass also supply cover.
- R5 runs in `prune_park_creases` immediately before tile serialization. It uses 3D distance at the checked design's sample spacing, 3 cm maximum covered distance, 5 degrees and 80 percent coverage. Longer lines win, ties use first-point coordinate bits with the remaining coordinates as a deterministic secondary key. Unique tails of at least 35 cm remain. Actual `kept_tracks` witnesses neither disappear nor cover other lines (A49). Unchanged lines preserve output order.
- `SKATEV_R1b_OFF`, `SKATEV_R4_OFF` and `SKATEV_R5_OFF` are baker-only trial switches. The runtime does not read these environment variables; trial labels remain incompatible with installable tiles.

## Failed attempts and corrections

- The inherited WIP suite failed two R5 tests: unchanged twins were reordered, and the alleged unique-tail fixture was actually fully covered by its longer line. Preserve unchanged output order and give the shorter fixture a real 1 m protruding tail.
- Added failing-first regressions reproduced a broad wall being removed when its triangle centroid lay outside the box, an R4 extension crossing a short existing line, and almost 30 m being added to one line. Triangle/slab intersection, hard stops and a shared 3D length budget correct these cases.
- The WIP off-switch test helper changed a process-global atomic. Replace it with a panic-safe thread-local test override so parallel tests cannot disable each other's rules. Production trial state remains process-global as required by baker workers.
- A temporary test-only disable of all three new rules reproduced eight expected positive-test failures, including the ring replacement, 12 m ridge continuation, equal-length and 3D duplicate cases. That disable was removed before final testing and baking. Raw logs and exit codes are retained.
- RTK's final test-log heuristic labelled two passing tests as errors because their names contain "panic"/"error"; the raw Cargo exit and suite summaries are authoritative.

## Validation

`cargo test --offline --locked --release -p skatev-runtime -p skatev-world-cache -p svwc`: 475 runtime, 42 world-cache, 7 visual-rails binary and 51 svwc tests passed (575 total, 27 ignored, zero failures), using this worktree's own target directory.

Five bounded park bakes, per-rule counts, rev4 comparisons, requested mark renders and the 22 regression spots are recorded in the evidence report. No whole-map empirical census is claimed. No game-folder writes, installation or push are authorized or performed.

Reviewer record: Claude Sonnet's existing Goldfish review covers the design; Claude Fable authored the interrupted WIP. Codex performed implementation review and validation. No new independent implementation peer review is claimed. The integrating orchestrator must carry the evidence and failures into the workspace registries and current-state record, and arrange the integration review. Gameplay remains Manatros' decision.

## Completion review: exact revised hooks before final correction

Fresh implementation peer review found that clipped ownership faces cannot veto a solid wall wholly in the ring, that corner-based admission changes rev4 behavior even with R1b off, and that five missing 10 cm samples can bridge a 60 cm gap. These findings are accepted. The local source probe also found the 38.4 m duplicate is produced by splitting one looping raw polyline into tile pieces; comparing only distinct unsplit lines cannot remove it.

- R1b: `export_parks` writes SVVP v2 with independently framed ownership and reach sections in the same receipt-bound file. Ownership remains centroid-clipped and shader-filtered exactly as before. Reach contains whole triangles overlapping the expanded region and includes pane/alpha shaders as wall veto evidence. `visparks::load` separates reach from ownership; missing/malformed/stale reach keeps universal ring boxes. `see_through_boxes_with_reach` uses reach for ring fill and solid-wall tests only, never for surface ownership. The manual registry remains independent. This producer/consumer plan was checked by the fresh peer before implementation completion.
- R5: final `prune_park_creases` compares the same 64 m pieces that serialization writes, so a returning raw polyline cannot hide a duplicate. It preserves the original unsplit list when nothing is removed. Actual train witnesses remain excluded.
- R4: count the full distance from the last accepted crest through a gap to the next accepted crest, bounded to 0.5 m. Surface samples must be shown, walkable faces. Follow the true crest history when correcting a small initial offset, keeping the existing 5 cm sideways step and 15 m total per-line budget.

This record supersedes the earlier validation/status paragraphs where later evidence differs. Final results and exact revision will be recorded after the new tests and five bakes finish.
