# Horizon NX Companion

A computer app for [Horizon on the Switch](https://github.com/KoHaku743/horizon-nx): it does on your
computer what the Switch is slow at, and looks after the SD card.

macOS for now (Apple silicon and Intel). Windows and Linux builds will follow; the code already finds
SD cards there.

## What it does

| Page | What for |
|---|---|
| **SD card** | Finds the Switch's SD card (in a card reader, or over USB with hekate's USB tools) and shows what Horizon has on it. **Eject** removes macOS's `._` files first, then ejects. |
| **Game files** | Copies Minecraft 26.3's files, prepared by the Horizon launcher on this computer, to `switch/horizon/resources/` and packs them into `resources.hzpak`, exactly as the game builds it on its first start, so the Switch starts fast from the first run. Files already on the card aren't copied again. |
| **Horizon build** | Installs `horizon.nro` from this fork's releases (the checksum is checked). Test builds are behind a switch. |
| **Mods and packs** | Horizon's mod registry: content mods and resource packs, checked here with the same code as the game's Get Mods (a check takes seconds on a computer), then put on the card and turned on. **Check all** checks every mod at once, so the list says which ones work on the Switch build. Search, Install, Update, Remove. |
| **Worlds** | Backs up the Switch's worlds to `Documents/Horizon Switch Backups`, and copies worlds between the Switch and Minecraft on this computer. |
| **Logs** | Reads the game's logs (errors and warnings only, if you like) and zips them for a bug report. |
| **Free up space** | Lists what the game can make again (old logs, unfinished downloads, old server resource pack bundles, macOS junk files) with sizes; you pick what goes. Worlds, settings, your account, mods and the game files are never listed. |

## Install (macOS)

1. Download `horizon-nx-companion-macos.zip` from
   [Releases](https://github.com/KoHaku743/horizon-nx-companion/releases) (or from the latest
   [build](https://github.com/KoHaku743/horizon-nx-companion/actions/workflows/build.yml) run's artifacts),
   unzip it and move **Horizon NX Companion** to Applications.
2. The app isn't notarized by Apple, so the first time: right-click it, choose **Open**, then **Open**
   again. If macOS still refuses (newer versions): System Settings > Privacy & Security > **Open Anyway**,
   or in Terminal: `xattr -dr com.apple.quarantine "/Applications/Horizon NX Companion.app"`.

## Getting the game files

Horizon never ships Minecraft's files: they come from your own copy. Install the
[Horizon launcher](https://playhorizon.dev) on this computer and start it once with your Minecraft
account; it prepares the files (a folder ending in `horizon-client/26.3`). The companion finds that
folder by itself, or you choose it with **Choose…**.

## Using it with the Switch

- **Card reader:** take the SD card out of the Switch and put it in the computer.
- **USB cable:** boot hekate, then **Tools > USB Tools > SD Card**; the card appears as a drive.

Always **Eject** before you unplug the card or leave hekate's USB mode.

## Building

```sh
cargo run -p horizon-nx-companion          # run it
cargo test --workspace                     # tests (no network)
packaging/macos/bundle.sh                  # the .app and its zip, on a Mac
```

`crates/core` does the work (no window: SD card, `.hzpak`, releases, registry, saves, logs, cleanup),
`crates/app` is the window (egui). `vendor/` holds Horizon's `horizon-mods` and `horizon-shader-api`
crates (MIT, see `vendor/LICENSE-horizon`), so a mod is checked by the same code as in the game.

## Later

- Building `horizon.nro` on this computer (native mods compiled into a custom build).
- Windows and Linux releases.

MIT licensed. Not affiliated with Mojang or Microsoft.
