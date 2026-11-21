## What's new in 0.3.0

For [Horizon NX 2.0.0](https://github.com/KoHaku743/horizon-nx/releases/tag/v2.0.0).

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
