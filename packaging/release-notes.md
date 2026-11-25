## What's new in 0.4.0

For [Horizon NX 2.2.1](https://github.com/KoHaku743/horizon-nx/releases/tag/v2.2.1).

- **Overview** (in the SD card page's place) says what's wrong on the card, worst first, with a button for what the companion can fix, or **Fix all**.
- **Game files that are too old are caught.** Files prepared for 1.x make 2.x load for minutes, go black and close; the companion used to call such a card ready.
- **Worlds, packs and mods with Cyrillic, Japanese or other non-Latin names show up on the Switch.** They're copied in the form Horizon NX 2.2 keeps such names, and ones already hidden on the card can be renamed.
- **Backups can be restored**, to the Switch or the computer, and one world can be backed up by itself. A second backup in the same minute no longer replaces the first.
- **Worlds** show their icon, their name in the game, the game mode and when they were last played.
- **Bug reports** include the crash reports (the game's and Atmosphère's), `panic.txt` and `env.txt`.
- The card is noticed when you plug it in or pull it out, and reading it no longer freezes the window.
- A new look, light or dark as your system is.
- **When Horizon's mod registry can't be reached**, Mods and packs shows the last copy of it saved on this computer or the card (as the game's Get Mods does) and says so. Mods checked on this computer before still install.

## What was new in 0.3.0

- **Mods and packs** checks mods with Horizon 2.0's own code (the same as the game's Get Mods), so it agrees with the game about which mods the Switch build installs. Mods that run code (WebAssembly mods, such as Zoom 3.0) are marked **Runs code: not for the Switch** once checked: **Check all** finds them before you install anything.
- Mods and packs installed from the computer show as installed in the game's Get Mods, with its **Update** button when there's a newer version (they're recorded in `mod-browser.json`, as the game records its own).
- **Free up space** also offers the CPU profiler's report (`cpu-profile.txt`).
- **Licence:** the companion is now under the GNU LGPL, version 3 or later, like Horizon (whose code it now includes). Everything stays open source; releases up to 0.2.0 were MIT.

## What was new in 0.2.0

- **Sync with Horizon** (new page): copies mods, resource packs and the server list between Horizon on your computer and Horizon on the Switch, in either direction. It adds what the other side doesn't have and updates what's different, and never deletes anything. Mods keep their on/off setting. Servers are added to the other side's list, which is backed up first as `servers.dat_old`. Only content mods go to the Switch, because the Switch build can't run mods with code; the page says which ones stay behind and why.
- **Worlds** now copy between the Switch and **Horizon** on your computer (`horizon/saves`), not the official Minecraft launcher's folder.

## Downloads

| System | File | First start |
|---|---|---|
| macOS 11+ (Apple silicon and Intel) | `horizon-nx-companion-macos.zip` | Not notarized: right-click the app > **Open**, twice. Or System Settings > Privacy & Security > **Open Anyway**. |
| Windows 10/11 (x64) | `horizon-nx-companion-windows.zip` | Not signed: SmartScreen > **More info** > **Run anyway**. |
| Linux (x86_64) | `horizon-nx-companion-linux-x86_64.tar.gz` | Run `./horizon-nx-companion`, or `./install.sh` to add it to your applications. |

Tested on macOS. The Windows and Linux builds haven't been tried on real machines yet: please report how they work.

Everything else (copying and packing the game files, installing builds, mods from the registry, worlds, logs, freeing up space) is described in the [readme](https://github.com/KoHaku743/horizon-nx-companion#readme) and the [0.1.0 notes](https://github.com/KoHaku743/horizon-nx-companion/releases/tag/v0.1.0).
