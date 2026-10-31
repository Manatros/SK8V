# GTAxSkate asset preparation tools

These tools prepare the board library and Self Radio soundtrack from a player's
own Skate 3 disc data. They are the source for the `skate-ped.exe` and
`skate-music.exe` helpers used by the GTAxSkate installer. They belong beside the
SK8V implementation so an upstream developer can build and adopt the complete
board-shop integration without obtaining a second runtime or the historical
stand-alone GTA plugin.

`SOURCE.json` records the original sanitized repository revision and SHA-256 of
every mirrored source file. The source files and their relative dependency paths
are unchanged. Hashes describe captured checkout bytes, including original line
endings, rather than Git's normalized text blobs. This move does not establish that a new binary was rebuilt from
this mirror; released binaries retain their own exact corresponding-source ZIP
and receipt.

## Build

Use the Rust toolchain pinned by the fork's `rust-toolchain.toml`. From this
directory, run:

```powershell
cargo build --release --locked --manifest-path gta/skate-ped/Cargo.toml
cargo build --release --locked --manifest-path gta/skate-music/Cargo.toml
cargo test --locked --manifest-path gta/skate-ped/Cargo.toml
cargo test --locked --manifest-path gta/skate-music/Cargo.toml
```

The executables are written under each tool's `target/release/` directory unless
`CARGO_TARGET_DIR` selects another location. See each executable's `--help` for
disc-data inputs. The tools do not contain or distribute converted game assets.

`skate-ped` depends on the exact `gta/crates/skate_boards` snapshot beside it.
Do not substitute the runtime's similarly named board crate without reviewing
the differences and rebuilding the helper. `skate-music` uses its own locked
dependencies. Dependency downloads need network access on the first build.

## Licenses and release source

Both executables declare GPL-3.0-or-later; their source distribution must include
the complete corresponding source, required dependency source and build inputs
for the exact released binaries. The shared board crate declares Apache-2.0.
Copies of both license texts are included here; original notices remain intact.

GTAxSkate binary packages include `GPL-CORRESPONDING-SOURCE.zip`,
`GPL-SOURCE-RECEIPT.json` and tool notices. Preserve those assets and verify their
hashes when packaging. A moving source branch does not replace that exact source
bundle. No v0.7.0 source asset or release approval is implied by this mirror.
