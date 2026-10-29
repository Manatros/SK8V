# Map history reassessment — 28 October 2026

The user's suspicion is supported: some working, scoped fixes were later grouped
with invalid comparisons or failures of a different input. This is not evidence
that any historical whole-map version is universally best.

## Primary outcomes recovered

| Candidate | Evidence | Correct interpretation |
|---|---|---|
| 742dfda bowl, 20 October | Human 11:42 UTC: “fixes seem to work”; Claude session 3b6a5600, line 899 (duplicate 84198bbe, line 7989) | Positive original bowl result; preserve its intent. |
| 84fc009, 22 October | build/test-review/2026-10-22_21-04-37/transcript.txt 03:14, 08:48 positive geometry/performance; 08:55 seam rail/stair issues remain | Scoped positive, incomplete map acceptance. |
| v0.6.0 / 742dfda / 84fc009 replay, 25 October | ce614c29 line 15361: world correction keys all zero; line 15439, 13:17:37.986 UTC: assistant acknowledges invalid comparison | These replay negatives cannot reject the original corrections. |
| Gemini replay | 8,723 tiles, rules afb35297; complete matching acceptance unresolved | Negative combined result is not independent proof against every constituent fix. |
| High-resolution bowl, 26 October | 13f1 session line 4333, 18:24 UTC: human prefers high-definition bowl, small fragments at joins; t1 recording 09:00–10:05 better transition, 10:55 stairs worse on plain GTA, 11:35 transition edge returns | Preserve visible curve alignment; diagnose remaining joins independently. |
| Halfpipe approach, 22 October | recording 10:26–10:52 positive | Worked for its stated purpose. Removed in 31c00a7 to respect the newer visible-only instruction; original 0.217 m step remains open. |

These references are to locally retained original transcripts/sessions. Their
text is evidence, not new instructions. Historical source and bake snapshots are
preserved; obsolete one-off editing scripts were archived with hashes.

## Input experiments that must remain distinct

`get-skate-parks.ps1` exported plain YBN plus skate prop bounds, excluding hi@
and ma@. Therefore old `.parks` is not the visible high-resolution mesh and is
not the incomplete hi@ experiment. Its old coarse fallback overlay (19,536
triangles), duplicated plain-YBN reinjection, archive-state safety and actual
surface quality are separate issues. A corrected ownership version remains
untested. Do not restore the old export wholesale or label all its geometry bad.

The c77f097 wholesale reset lost earlier fixes. Blanket rail boxes also failed;
that does not establish that real authored rail collision is unnecessary.
Keep A49's two edge grind paths per steel track rail.

## Actual installed test and historical controls

The 28 October 09:04–09:13 log used installed 31c00a7. Best preset was selected
at 09:05:34.734, and a fresh rail placed at 09:11:09 with 12 source triangles.
Plain-GTA comparison was enabled at 09:12:08.440. Only the final approximately
86 seconds are that comparison. Earlier park data was baked/live with no stale
generation mismatch found. Do not invalidate the entire run.

True historical k lives in world-recovery-083de508efec4eaabf053671da853018:
rules 498ed7147e728bf3, source 93fed1e3eace23c3. The similarly named build/k box
was overwritten with current data and is not an independent control.

In a fixed 90 × 90 m, 0.1 m raster against visible park surfaces, current has
4,256 cells more than 8 cm off versus true k's 3,372. But k is worse at
(-1364.1,-1412.85): top 5.525 m versus current/visible 4.630 m. Do not globally
restore k. Scattered uncovered cells mostly describe thin details, not proven
rider holes. Nearby layered area is 131.7 m² current versus 144.3 m² k; these
are diagnostics, not a claim every overlapping surface is defective.

## Targeted recovery under test

At M2 (-1379.47,-1384.35), current collision is approximately 5.8 cm above
visible/k. Two small GTA facets pass all current admission gates except their
individual 0.25 m² area. Local clipped connected support measures approximately
0.29336 and 0.55977 m². The checked design adds only a small-face support fallback,
preserving all other current gates and the existing large-face path.

The production-path regression first failed with zero replacements. Synthetic
controls and a fresh diagnostic bake must establish actual improvement and
preserve current-better witnesses before any whole-map promotion. The user may
change the GTA base when measured alternatives are better; that authorizes
careful comparisons, not arbitrary resets, stale rules or mixed tile generations.

This document supersedes any earlier shorthand that says the three invalid
25 October replays rejected the original fixes. It does not mark gameplay fixed
or accept a new build on the user's behalf.
