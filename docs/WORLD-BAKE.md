# Story Mode collision generation

Offline collision prevents high-speed skating from depending entirely on GTA's
streaming radius. It must not combine mutually exclusive states or load
Online-only geometry. Raw fallback floors, city tiles and visible rails share
the Story archive policy. Rationale: [GTAXSKATE.md](../GTAXSKATE.md).

## Inputs and commands

Use the player's GTA V Legacy installation and locally extracted keys. Keep GTA,
FiveM, OpenIV and CodeWalker closed. Never distribute archives, keys or generated
geometry in the source repository. Build matching tools and host/runtime first.

Choose empty staging. `CACHE` is a prefix such as `STAGE/sk8v.svwc`, not a required
existing file. Material/track sidecars can be copied from the player's own setup.

```text
visual-rails STAGE/sk8v.visrails --gta GTA --keys KEYS
skatev-world-cache --bake-story-map STAGE/sk8v-map.svwc GTA KEYS
skatev-world-cache --bake-world STAGE/tiles X Y HALF GTA KEYS UNUSED_COLLISION CACHE 64
```

Try a representative small region first. Current Los Santos city coverage:

```text
skatev-world-cache --bake-world-box STAGE/tiles -2200 1500 -3500 1000 GTA KEYS UNUSED_COLLISION CACHE
skatev-world-cache --verify-world STAGE -2200 1500 -3500 1000
```

`UNUSED_COLLISION` retains an old positional argument. The corrected baker reads
world bounds and correctly placed interiors from mounted Story archives, not an
all-DLC flattened extraction. `--world-format` reports `SVWT-LIVE 1` for installers.

The Story fallback keeps invariant ped-facing walkable floors at the existing
normal threshold, excluding camera/vehicle-only faces and inactive variants.
Its adjacent `.policy` receipt binds generator policy and exact output bytes.
Old unverified maps are rejected. The rail exporter writes an equivalent receipt.

## Ownership and compatibility

SVWT v2 stores compressed final triangles, packed tags and rails in 64 m cells.
Each static cell replaces live geometry there, including valid empty tiles.
Missing/corrupt static cells use active collision. `live-cells.txt` explicitly
marks script/time/weather dependent cells: their missing baked files are
intentional coverage, not unfinished work.

Policy binds rules, source generation, size, count and body digest; every static
tile also binds its expected digest. Invalid policy on an existing enabled
dataset returns an error and retains the installed area. Never relabel old tiles
or silently enable unrestricted fallback.

Live patch geometry is restricted before expensive cleanup/rail generation,
with 13 m support for closed-solid handling, then clipped to cell ownership.
Raw floors cannot refill state-dependent cells. Empty states watch for later
loads. Active physics tubes remain; static visual/track sidecars are omitted
in live patches, so sidecar-only grind lines can be absent there.

Placed props are processed separately; moving entities keep their dynamic path.
The LRU budgets 96 MiB of retained decoded tiles. Installed areas, scratch,
temporary references and auxiliary queries add memory. Velocity prefetch and
teleport pruning remain. `BakedWorld` defaults on; an explicit off setting selects
the legacy live path, though unverified raw maps are still rejected.

## Verification and installation

The baker reads back written tiles. `--verify-world` checks both receipts,
source/rule/policy identities and complete region coverage by static plus live
cells. It decodes expected tiles and rejects missing invariant cells or baked
copies of live-only cells in a fresh stage.

Parallel slices must have identical source/policy metadata before merging; verify
the complete region afterward. Counting only `.svwt` files incorrectly retries
intentional live cells.

Install only verified map/rail files and receipts, `.svwt` files, and
`tiles/rules.txt`, `tiles/tile-size.txt`, `tiles/live-cells.txt`. Keep OBJ/MTL
previews, full inventories and progress logs in private staging. Preserve rollback
copies and promote the complete payload only after verification, with games and
archive editors still closed.

Tests are not gameplay acceptance. Compare the same positions/states with
upstream: streets, changing states, bridges, metro contacts, grinds and props.
Moving-object grinds and repeated-pattern stairs remain separate unresolved work.
