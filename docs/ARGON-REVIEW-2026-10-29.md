# Argon handoff review and next local test

Argon's intervening map work follows the checked plan and is preserved. It
implemented connected upward bank support plus direct shared-edge partner
ownership, including the original-chord deviation distinction that prevents
admitting an otherwise flat neighbour. It did not reset the world, weaken the
facing rule globally, change A49 track lines, or replace authored prop repairs.

Independent verification of the installed candidate `test-map-recovery-20261029-0eb21e5`:

- Runtime SHA256 `0d8bb4108086f14f662ee8159449954915bdac2a0048792eca97a83ccdd331e9`;
  host SHA256 `93baab164ef02976d125ed669e782261c4f56f6ac0272f455f04dfe9e3c0dea9`.
- Matching world rules `0eb21e5711ea49fb`, source `e5b5dc07c473a150`; native
  verification of all 25,912 invariant cells and 200 live cells passed:
  57,286,992 triangles, 841,779 rail records. Runtime regression: 320 pass,
  21 explicitly ignored. Existing full-map audit loses no old grind coverage
  across the five parks; the measured M2 residual chord seam/rail is removed.
- Actual code was snapshotted before further work. No Argon source was reverted.

## What the first test establishes

The 12:46:09 host log reports `custom: Grind every stair step=1, Unmarked stairs
as steps (test)=1`. Argon reset those saved values after the run at 13:09. The
experimental switches confound stair/step-grind comparisons. They do not erase
the human's remaining bowl-fragment feedback: HighResParks was enabled and no
stale-bake or plain-GTA comparison was logged. Zero high-res replacements in the
live census does not mean baked park replacement was off.

The file named crash log contains memory telemetry, with no recorded fatal
exception or panic for this run. NO FLOOR diagnostics describe the live input
before the baked/live merge; they are not by themselves proof of a final hole.
No new video or MARK-INFO positions accompanied this run in the inspected files.

## Next test change

The local test host starts with one canonical best collision profile, including
nearby entity collision. Best/reset/preset/status values derive from the same
table. Old experimental preferences cannot silently become the startup profile.
Manual diagnostic changes and comparison remain available and logged for the
session; another test start restores the baseline. The normal release build
option defaults OFF. Personal INI, User snapshot, bindings and board files are
not written by the new normalizer. Existing first-run behavior is unchanged.

The nearby-entity toggle appears once under World collision and never sends its
host-only sentinel as a runtime option. Runtime and static world remain exactly
the verified Argon generation. A new world bake is not needed for this host-only
test change. Fresh review caught and corrected its initial menu-routing error.

Validation: Release host with test baseline ON; independent fresh CMake default
OFF; enabled and disabled world-settings fixtures; menu pages, presets, save
text/permissions, camera, interaction, dropper and prop fixtures all pass (9).
Tests check preserved personal fields, unchanged INI bytes, canonical/preset key
uniqueness, DynamicWorld status, exact runtime option IDs and no sentinel output.

## Remaining work: do not mark fixed

- The human reports remaining connector fragments. Preserve that feedback; use
  World > Mark this spot at each catch/fragment in the next unchanged baseline
  run so actual source faces and active rules can be attributed.
- Missing metro/Vinewood pillar and partial tunnel collision need source and
  placement ownership proof. Metro templates already exist with material 64;
  deleting that material or blindly enabling all prop baking would regress
  working geometry. Earlier M1 telemetry gathered zero nearby bodies, so a
  missing template is not its established cause.
- Shop menu opened at the correct shop in the captured run, while automatic
  display placement failed. Physical opening, measured shelf slots and seller
  placement remain separate work. Do not hide the whole stall or guess a door.
- The original halfpipe 0.217 m step remains after removal of invisible approach
  slopes. Full GTA stunt-jump support while skating remains an open feature;
  no vehicle impersonation or physics/transition workaround is included.

This is a local test candidate, not gameplay acceptance or a public release.
Keep the complete binary/world rollback pair. No release before 2 November.
