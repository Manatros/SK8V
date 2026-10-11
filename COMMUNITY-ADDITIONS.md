# SK8V Community Additions 1.1.0

An unofficial add-on for SK8V 1.1.6, containing the custom changes made for Jason. Requires an already installed and working copy of SK8V. This is not the full SK8V release.

## Requirements

- SK8V 1.1.6 installed and working: https://github.com/becaring/SK8V/releases
- GTA V Legacy Story Mode, tested on game build 1.0.3889.0.
- SK8V's existing prerequisites, including ScriptHookV and your prepared game assets.
- An Xbox or PlayStation controller supported by SK8V; LB/RB correspond to L1/R1 on PlayStation.
- Optional: a trainer that can create cat followers/bodyguards. Tested with Menyoo 2.4.4. Menyoo is not required by the code; equivalent player-group followers should work, but other trainers have not been tested.

## Install

1. Close GTA V.
2. Open your GTA V installation folder (the folder containing GTA5.exe).
3. Back up its existing SkateVLegacy.asi and SkateVRuntime.dll somewhere outside the game folder. Keep both originals together.
4. Drag SkateVLegacy.asi and SkateVRuntime.dll from this ZIP into the GTA V folder, replacing those two files. Administrator permission may be needed for Program Files.
5. Launch GTA V Legacy Story Mode as you normally do with SK8V.

Install both files together: the modified ASI uses additional exports in the modified runtime DLL. Keep your existing SkateVLegacy.ini and prepared assets. This package does not contain or replace your settings, and setup does not need to be rerun. Rerunning the original SK8V installer can overwrite these replacement files.

## Features and controls

### Board during a vehicle exit

Hold LB, press Y to exit a car, and keep LB held until the board appears. Exit momentum carries into skating. Prepared nearby collision must be available.

### Easier skitching

Hold RB while approaching the rear of an NPC-driven car. Traffic assistance slows the selected car relative to your skating speed so you can catch its bumper. Closing speed tapers near the car to reduce impacts. Once attached, the car switches to the separate towing speed instead of continuing to match the approaching skater.

Back + LB > Skating includes Skitch traffic assist, Skitch catch-up speed, Towing speed, and Tow to waypoint. Default catch-up speed is 3 m/s; default towing speed is 10 m/s (about 22 mph).

Tow to waypoint is OFF by default. Enable it and place a GTA map waypoint: while attached, the driver follows roads toward it and stops nearby. Moving the waypoint updates the route; removing it returns the car to wandering. Disabling traffic assist disables both speed assistance and routing.

Assisted drivers ignore traffic lights. While towing they can overtake, change lanes, and use opposing road lanes to avoid traffic. Off-road and shortcut routing are disabled, but GTA driving AI can still clip curbs. Player drivers and mission vehicles during an active mission are excluded. Releasing a car restores ordinary driving; its exact prior traffic route is not reconstructed.

### Session-marker ground retention

Static Skate collision is retained in a 128 x 128 metre square around your current session marker, even after leaving that area. A boundary guard pauses movement if surrounding collision has not finished loading. Place a marker after loading this build. Retention lasts for the current game session; setting another marker replaces the retained area. This does not retain nearby traffic.

### Shoulder cat

Spawn Cat or Cat 2 as a FOLLOWER/BODYGUARD, then enter Skate mode. The add-on carries one living cat belonging to the player's GTA group, selected within 40 metres. Ordinary cat spawns and ambient cats are intentionally ignored. The add-on does not spawn cats itself.

Back + LB > Shoulder cat controls carrying, shoulder selection, pose, offsets, and facing. Default: right shoulder and the game's ledge-rest pose. The cat follows the chosen anatomical shoulder and animated torso, including when rolling backward. Shoulder selection does not change automatically. Disable carrying or leave Skate mode to return the cat beside you and restore its follower role.

Menyoo is one way to create an eligible cat. Another trainer must actually add the cat to the player's GTA follower group; simply marking a ped as friendly or spawning it nearby is insufficient. No trainer is needed for the other features.

### HUD fix

Includes the HUD cache reset fix developed while adding these features.

## Settings and diagnostics

Menu settings save to the existing SkateVLegacy.ini. These additional keys can also be added under [SkateV]; the values below are the defaults:

```ini
SkitchTrafficAssist=1
SkitchTowWaypoint=0
SkitchClosingSpeed=3.0
SkitchTowSpeed=10.0
ShoulderCat=1
CatLeftShoulder=0
CatPose=0
CatSideways=0.12
CatForward=-0.14
CatHeight=0.04
CatYaw=0.0
```

Speeds use metres per second; cat offsets use metres and yaw uses degrees. CatPose: 0 = resting on ledge, 1 = sleeping, 2 = standing. Existing saved preferences take precedence over defaults.

SK8V's logs in %LOCALAPPDATA%\SkateV include speed/route diagnostics, marker-collision status and cat acquisition/release messages. If the cat does not appear, confirm it is a living follower in your GTA group and Carry follower cat is enabled.

## Remove

Close GTA V and restore BOTH original files from the backup you made. Alternatively, rerun the original SK8V 1.1.6 installer to restore its binaries. The new settings may remain in your INI; no assets are removed by uninstalling this add-on.

## Compatibility and validation

Built for SK8V 1.1.6, GTA V Legacy build 3889. Newer SK8V versions are untested. GTA V Enhanced, FiveM and GTA Online are outside the supported SK8V configuration. Do not mix these binaries with other custom SK8V binary replacements without merging and rebuilding their source changes.

The previous custom build was tested in game by the requester. This port to SK8V 1.1.6 has build and regression-test validation; it still needs an in-game check. Traffic AI behavior and placement on other player models may vary; shoulder offsets are adjustable.

## Credits, license and source

Original SK8V: becaring, https://github.com/becaring/SK8V. This package is an unofficial derivative and is not an upstream release.

The two replacement binaries contain SK8V and its dependencies compiled with the modifications; they are provided to install the add-on, not as a standalone mod. The full SK8V distribution, installers, game assets, ScriptHookV and Menyoo are not included.

LICENSE, NOTICE and THIRD-PARTY-NOTICES.txt preserve the upstream license and attribution information. See MODIFICATIONS.txt for the changed-file list and source/README.md for the source patch and build instructions.
## AI skater followers (1.2.0)

SK8V > AI skater followers enables independent human follower sessions. All cats and other animals are excluded. This controller never calls or changes the shoulder-cat manager.

Optional [SkateV] INI keys (defaults):

```ini
AiFollowers=0
AiFollowerGroups=1
AiFollowerMissions=1
AiFollowerTricks=1
AiFollowerMax=2
AiFollowerDistance=6.0
```

Maximum skaters is clamped to 1-4 and following distance to 3-20 metres in the menu. The feature only runs while the player is actively skating with player pose and board presentation enabled. Group followers must belong to the player's actual GTA group. Mission followers must be friendly mission entities with an active standard follow/go-to-entity task; arbitrary mission peds are not adopted. Unsupported character caches/skeletons fail setup and leave the GTA follower untouched.

Each follower has its own Skate physics/animation session, outfit, skeletal pose and board. Background loading never takes control of the follower until a valid pose is ready. Simulation steps run serially on the existing worker to keep the engine input bus isolated. The controller uses a breadcrumb route and its own generated inputs; the player's trick stream is never replayed. Low forward rails and rising surfaces provide independent trick opportunities, subject to following distance and cooldown. Cars/peds are obstacle proxies. No new world entities are created except owned follower boards.

Explicit mission tasks are preserved. Human group membership is temporarily suspended during AI ownership and restored when released. Disable/exit, combat, swimming, vehicle entry, deletion, external mission teleportation or stalled simulation remove the follower pose hook and board and restore normal control. No arbitrary mission task is cleared or overwritten. The mod does not implement full mission-specific navigation or Skate 3's complete NPC AI.

Setup/readiness and geometry trick choices are logged under %LOCALAPPDATA%\SkateV. New AI behavior needs in-game validation; the included real-asset simulation covers independent routes, stopping and turning.

1.2.1 fixes the GTA ground-query argument type during follower setup and adds a dedicated human-follower diagnostic log (SkateV-AI-Followers.log). It records reasons for skipped candidates, setup stages, ground height and argument encoding, loading wait, pose hook failures, board readiness and releases. Scan/status heartbeats are limited to once per 10 seconds; candidate decisions log on changes.
