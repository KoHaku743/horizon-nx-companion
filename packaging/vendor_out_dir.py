"""The OUT_DIR of a package's build script, from `cargo --message-format=json` on stdin.

usage: cargo check -p <package> --message-format=json | python3 vendor_out_dir.py <package>"""
import json
import sys

package = sys.argv[1]
found = ""
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    if message.get("reason") != "build-script-executed":
        continue
    # A path package's id: "path+file:///.../crates/<package>#0.1.0" (or "<package> 0.1.0 (...)").
    package_id = message.get("package_id", "")
    if f"/{package}#" in package_id or package_id.startswith(f"{package} "):
        found = message["out_dir"]
print(found)
