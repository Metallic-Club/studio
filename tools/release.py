#!/usr/bin/env python3
"""Build Metallic Studio, pack it with Velopack and publish it as a GitHub Release.

    python tools/release.py --channel dev --version 0.1.1
    python tools/release.py --app launcher --channel dev --version 0.1.1

`--app studio` (the default) is the Studio release described here. `--app launcher` builds the
Metallic Launcher (`Metallic.exe`) and the Player (`metallic-player.exe`, staged beside it), packs
them as MetallicLauncher on the Velopack channel `launcher-<channel>` with the WebView2 framework
check, and uploads to the same repository as a release tagged `launcher-v<version>-<channel>`.

Steps: cargo build --release, stage the files with the main exe renamed to "Metallic Studio.exe",
`vpk pack` (installer "MetallicStudio-<channel>-Setup.exe" plus full and delta packages), then
`vpk upload github` to Metallic-Club/studio as a release tagged v<version>-<channel>.

The same script runs by hand and in .github/workflows/release.yml. The GitHub token is read from
the environment variable GITHUB_TOKEN (the workflow's own token, or `gh auth token` by hand); it
is never written to a file and never printed. The vpk CLI has no documented environment variable for
its token, so it is passed as `--token` (masked in our own output; visible to other processes on the
build machine while vpk runs). `--dry-run` prints the commands and runs nothing.

The Velopack CLI version below must match the `velopack` crate version in
crates/metallic_studio/Cargo.toml, because Studio's updater and the packages must agree.
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

VPK_VERSION = "1.3.39-g8d2844f"
PACK_ID = "MetallicStudio"
PACK_TITLE = "Metallic Studio"
MAIN_EXE = "Metallic Studio.exe"
CARGO_BIN = "metallic_studio"
REPO_URL = "https://github.com/Metallic-Club/studio"
ROOT = Path(__file__).resolve().parent.parent
ICON_PNG = ROOT / "branding" / "placeholder-icon.png"
ICON_ICO = ROOT / "branding" / "icon.ico"
OUT_DIR = ROOT / "target" / "release-out"
VERSION_RE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", re.ASCII)
CARGO_TOML = ROOT / "Cargo.toml"

# The Launcher: one package holding the Launcher and the Player. Studio's own settings stay in the
# constants above so its release output does not change.
LAUNCHER_PACK_ID = "MetallicLauncher"
LAUNCHER_PACK_TITLE = "Metallic"
LAUNCHER_MAIN_EXE = "Metallic.exe"
LAUNCHER_CARGO_PACKAGES = ["metallic_launcher", "metallic_player"]
# (file in target/release, name in the package)
LAUNCHER_FILES = [("Metallic.exe", "Metallic.exe"), ("metallic-player.exe", "metallic-player.exe")]
LAUNCHER_CHANNEL_PREFIX = "launcher-"
LAUNCHER_TAG_PREFIX = "launcher-v"


def valid_version(version):
    """True for X.Y.Z with plain ASCII digits and no leading zeros."""
    return VERSION_RE.fullmatch(version) is not None


def workspace_version(cargo_toml_text):
    """The [workspace.package] version of the root Cargo.toml."""
    import tomllib
    version = tomllib.loads(cargo_toml_text).get("workspace", {}).get("package", {}).get("version")
    if not isinstance(version, str):
        sys.exit("Cargo.toml has no [workspace.package] version")
    return version


def check_version(version, cargo_toml_text):
    """Stop unless the release version equals the workspace version the built Studio reports."""
    found = workspace_version(cargo_toml_text)
    if found != version:
        sys.exit(f"--version {version} differs from the workspace version {found} in Cargo.toml. "
                 "Bump [workspace.package] version, merge to main, then release.")


def masked(command):
    """The command as text, with the value after --token hidden."""
    parts = []
    hide_next = False
    for part in command:
        parts.append("<token>" if hide_next else str(part))
        hide_next = part == "--token"
    return " ".join(f'"{p}"' if " " in p else p for p in parts)


def run(command, dry_run, check=True):
    print("+", masked(command), flush=True)
    if dry_run:
        return 0
    result = subprocess.run(command, cwd=ROOT)
    if check and result.returncode != 0:
        sys.exit(f"command failed with exit code {result.returncode}: {command[0]}")
    return result.returncode


def check_vpk_version(vpk, dry_run):
    if dry_run:
        return
    try:
        found = subprocess.run([vpk, "--version"], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        sys.exit(f"vpk not found. Install it: dotnet tool install --global vpk --version {VPK_VERSION}")
    if found != VPK_VERSION:
        sys.exit(f"vpk {found} is installed but this script is pinned to {VPK_VERSION}. "
                 f"Run: dotnet tool update --global vpk --version {VPK_VERSION}")


def make_icon(dry_run):
    """vpk needs an .ico. Use branding/icon.ico if it exists, else convert the PNG into the output dir."""
    if ICON_ICO.exists():
        return ICON_ICO
    target = OUT_DIR / "icon.ico"
    print(f"+ convert {ICON_PNG.relative_to(ROOT)} -> {target.relative_to(ROOT)}")
    if not dry_run:
        try:
            from PIL import Image
        except ImportError:
            sys.exit("Pillow is needed to build the icon: pip install Pillow (or add branding/icon.ico)")
        OUT_DIR.mkdir(parents=True, exist_ok=True)
        sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
        Image.open(ICON_PNG).convert("RGBA").save(target, sizes=sizes)
    return target


def pack_command(vpk, app, version, channel, stage, icon, notes):
    """The `vpk pack` command line. Studio's is unchanged; the Launcher's adds the WebView2
    framework check and uses the Velopack channel `launcher-<channel>`."""
    if app == "launcher":
        return [vpk, "pack", "--packId", LAUNCHER_PACK_ID, "--packVersion", version,
                "--packDir", str(stage), "--mainExe", LAUNCHER_MAIN_EXE, "--packTitle", LAUNCHER_PACK_TITLE,
                "--icon", str(icon), "--framework", "webview2", "--runtime", "win-x64",
                "--channel", f"{LAUNCHER_CHANNEL_PREFIX}{channel}", "--outputDir", str(OUT_DIR),
                "--releaseNotes", str(notes)]
    return [vpk, "pack", "--packId", PACK_ID, "--packVersion", version,
            "--packDir", str(stage), "--mainExe", MAIN_EXE, "--packTitle", PACK_TITLE,
            "--icon", str(icon), "--channel", channel, "--outputDir", str(OUT_DIR),
            "--releaseNotes", str(notes)]


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--app", choices=["studio", "launcher"], default="studio",
                        help="what to release (default: studio)")
    parser.add_argument("--channel", choices=["dev", "stable"])
    parser.add_argument("--version", help="X.Y.Z, for example 0.1.1")
    parser.add_argument("--required", action="store_true",
                        help="mark the release required: Studio then insists on the update")
    parser.add_argument("--no-upload", action="store_true", help="build and pack only")
    parser.add_argument("--dry-run", action="store_true", help="print the commands, run nothing")
    parser.add_argument("--jobs", type=int, default=4, help="cargo -j value")
    parser.add_argument("--vpk", default="vpk", help="path of the vpk executable")
    parser.add_argument("--check-version", action="store_true",
                        help="only check that --version equals the workspace version, then exit")
    parser.add_argument("--print-vpk-version", action="store_true", help="print the pinned vpk version and exit")
    args = parser.parse_args()

    if args.print_vpk_version:
        print(VPK_VERSION)
        return
    if args.check_version:
        if not args.version or not valid_version(args.version):
            parser.error("--check-version needs --version X.Y.Z")
        check_version(args.version, CARGO_TOML.read_text(encoding="utf-8"))
        print(f"Version {args.version} matches the workspace version.")
        return
    if not args.channel or not args.version:
        parser.error("--channel and --version are required")
    if not valid_version(args.version):
        parser.error("--version must look like 0.1.1 (no leading v, no channel suffix)")
    check_version(args.version, CARGO_TOML.read_text(encoding="utf-8"))

    launcher = args.app == "launcher"
    tag = f"{LAUNCHER_TAG_PREFIX if launcher else 'v'}{args.version}-{args.channel}"
    pack_channel = f"{LAUNCHER_CHANNEL_PREFIX}{args.channel}" if launcher else args.channel
    token = os.environ.get("GITHUB_TOKEN", "")
    if not args.no_upload and not args.dry_run and not token:
        sys.exit("GITHUB_TOKEN is not set (the workflow provides it; by hand: set it from `gh auth token`).")

    check_vpk_version(args.vpk, args.dry_run)

    if launcher:
        cargo = ["cargo", "build", "--release"]
        for package in LAUNCHER_CARGO_PACKAGES:
            cargo += ["-p", package]
        run(cargo + ["-j", str(args.jobs)], args.dry_run)
    else:
        run(["cargo", "build", "--release", "-p", CARGO_BIN, "-j", str(args.jobs)], args.dry_run)

    stage = OUT_DIR / "stage"
    notes = OUT_DIR / "release-notes.md"
    staged = LAUNCHER_FILES if launcher else [(f"{CARGO_BIN}.exe", MAIN_EXE)]
    for built, name in staged:
        print(f"+ stage {built} as {name} in {stage.relative_to(ROOT)}")
    if not args.dry_run:
        shutil.rmtree(OUT_DIR / "stage", ignore_errors=True)
        stage.mkdir(parents=True)
        for built, name in staged:
            exe = ROOT / "target" / "release" / built
            if not exe.exists():
                sys.exit(f"{exe} not found; did the build use another target dir (CARGO_TARGET_DIR)?")
            shutil.copy2(exe, stage / name)
        title = LAUNCHER_PACK_TITLE + " Launcher" if launcher else PACK_TITLE
        lines = [f"{title} {args.version} ({args.channel})"]
        if args.required:
            lines += ["", "required: true"]
        notes.write_text("\n".join(lines) + "\n", encoding="utf-8")

    icon = make_icon(args.dry_run)

    if not args.no_upload:
        # Fetch earlier packages so vpk can build delta updates. The first release has none.
        code = run([args.vpk, "download", "github", "--repoUrl", REPO_URL, "--channel", pack_channel,
                    "--outputDir", str(OUT_DIR), "--token", token], args.dry_run, check=False)
        if code != 0:
            print(f"WARNING: 'vpk download github' failed with exit code {code}. Expected for the very first "
                  "release of a channel; otherwise this release will have no delta package.", file=sys.stderr)

    run(pack_command(args.vpk, args.app, args.version, args.channel, stage, icon, notes), args.dry_run)

    if args.no_upload:
        print("Packed; not uploading (--no-upload).")
        return

    app_title = "Metallic Launcher" if launcher else "Metallic Studio"
    release_name = f"{app_title} {args.version} ({args.channel})"
    upload = [args.vpk, "upload", "github", "--repoUrl", REPO_URL, "--channel", pack_channel,
              "--outputDir", str(OUT_DIR), "--tag", tag, "--releaseName", release_name,
              "--publish", "--token", token]
    if args.channel == "dev":
        upload.append("--pre")
    run(upload, args.dry_run)
    print(f"Would publish release {tag}." if args.dry_run else f"Published release {tag}.")


if __name__ == "__main__":
    main()
