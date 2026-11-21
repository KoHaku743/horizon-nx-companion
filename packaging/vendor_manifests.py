"""Trims the vendored Horizon crates' Cargo.toml files to what the companion builds.

usage: vendor_manifests.py <vendor/horizon> <crate>...

Drops dev-dependencies and test, bench and example targets (they stay in horizon-nx), and
horizon-mod-vocab's build-time generator: its output is vendored as generated/. Their licence is
horizon-nx's workspace one (LGPL-3.0-or-later), written out: this workspace's is the companion's
own (MIT), which they must not inherit."""
import re
import sys

dest, crates = sys.argv[1], sys.argv[2:]
for crate in crates:
    path = f"{dest}/{crate}/Cargo.toml"
    text = open(path).read()
    text = text.replace("license.workspace = true", 'license = "LGPL-3.0-or-later"')
    text = re.sub(r"(?ms)^\[(dev-dependencies|\[example\]|\[test\]|\[bench\])\].*?(?=^\[|\Z)", "", text)
    if crate == "horizon-mod-vocab":
        text = re.sub(r"(?ms)^\[build-dependencies\].*?(?=^\[|\Z)", "", text)
        text = re.sub(r"(?m)^mixed-output-fixture = .*\n", "", text)
    open(path, "w").write(text)
