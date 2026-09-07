"""Create the updater's signed manifest from CI-built portable binaries."""
import json
import os
import pathlib
import re
import subprocess
import tempfile

version = os.environ["RELEASE_VERSION"]
if not re.fullmatch(r"\d+\.\d+\.\d+", version):
    raise ValueError("A stable semantic version is required")
release_dir = pathlib.Path("dist")
repo = os.environ["GITHUB_REPOSITORY"]
platforms = {}
with tempfile.TemporaryDirectory(prefix="tilekeep-sign-") as directory:
    key = pathlib.Path(directory) / "key"
    key.write_text(os.environ["TILEKEEP_UPDATE_SECRET_KEY"])
    key.chmod(0o600)

    def sign(path):
        subprocess.run(["minisign", "-S", "-s", str(key), "-m", str(path)],
                       input=os.environ.get("TILEKEEP_UPDATE_KEY_PASSWORD", "") + "\n",
                       text=True, check=True)
        subprocess.run(["minisign", "-V", "-P", os.environ["TILEKEEP_UPDATE_PUBKEY"], "-m", str(path)], check=True)
        return pathlib.Path(str(path) + ".minisig").read_text()

    for platform, filename in [("linux-x86_64", "tilekeep-linux-x86_64"),
                               ("windows-x86_64", "tilekeep-windows-x86_64.exe")]:
        binary = release_dir / filename
        platforms[platform] = {"url": f"https://github.com/{repo}/releases/download/v{version}/{filename}",
                               "signature": sign(binary)}
    manifest = release_dir / "latest.json"
    manifest.write_text(json.dumps({"schema": 1, "version": version, "platforms": platforms}, indent=2))
    sign(manifest)
