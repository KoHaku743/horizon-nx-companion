The first release of **Horizon NX Companion**: an app for your computer that gets the Switch's SD card ready for [Horizon NX](https://github.com/KoHaku743/horizon-nx) and does the slow jobs the Switch would otherwise do itself.

## What it does

- **Game files:** copies Minecraft 26.3's files, prepared by the Horizon launcher, to the SD card and packs them into `resources.hzpak` the same way the game would. The Switch then starts fast from the very first run instead of building the pack itself.
- **Horizon build:** installs `horizon.nro` from the Horizon NX releases and checks its checksum. Test builds are behind a switch.
- **Mods and packs:** browse Horizon's mod registry with search. Mods are checked with the same code as the game's Get Mods, in seconds, so the list tells you which ones work on the Switch build. Install, Update and Remove from here.
- **Worlds:** back up the Switch's worlds, and copy worlds between the Switch and Minecraft on your computer.
- **Logs:** read the game's logs (or only the errors) and zip them for a bug report.
- **Free up space:** old logs, unfinished downloads, old server resource pack bundles and macOS junk files, with sizes. You pick what goes; worlds, settings, your account, mods and the game files are never listed.
- **Eject:** cleans up macOS's `._` files, then ejects the card.

It finds the SD card in a card reader or over USB with hekate (**Tools > USB Tools > SD Card**).

## Downloads

| System | File | First start |
|---|---|---|
| macOS 11+ (Apple silicon and Intel) | `horizon-nx-companion-macos.zip` | Not notarized: right-click the app > **Open**, twice. Or System Settings > Privacy & Security > **Open Anyway**. |
| Windows 10/11 (x64) | `horizon-nx-companion-windows.zip` | Not signed: SmartScreen > **More info** > **Run anyway**. |
| Linux (x86_64) | `horizon-nx-companion-linux-x86_64.tar.gz` | Run `./horizon-nx-companion`, or `./install.sh` to add it to your applications. |

Tested on macOS. The Windows and Linux builds are new and haven't been tried on real machines yet: please report how they work.

You need the Horizon launcher on the same computer, started once with your Minecraft account and its files prepared: Horizon never ships Minecraft's files.
