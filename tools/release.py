#!/usr/bin/env python3
"""Build Metallic Studio, pack it with Velopack and publish it as a GitHub Release.

    python tools/release.py --channel dev --version 0.1.1

Steps: cargo build --release, stage the files with the main exe renamed to "Metallic Studio.exe",
`vpk pack` (installer "MetallicStudio-<channel>-Setup.exe" plus full and delta packages), then
`vpk upload github` to Metallic-Club/studio as a release tagged v<version>-<channel>.

The workflow separates --build-only from --publish-only --input-exe PATH on fresh runners.
The build phase never invokes Velopack or reads a publishing token. The publishing phase never
invokes Cargo and treats the downloaded executable as packaging data, not a program to run.
The combined command remains available by hand; this is not a sandbox for a local build script.

The GitHub token is read from
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
import stat
import subprocess
import sys
from pathlib import Path

VPK_VERSION = "1.3.39-g8d2844f"
PACK_ID = "MetallicStudio"
PACK_TITLE = "Metallic Studio"
MAIN_EXE = "Metallic Studio.exe"
CARGO_PACKAGE = "metallic_studio"
CARGO_BIN = "metallic-studio"
REPO_URL = "https://github.com/Metallic-Club/studio"
ROOT = Path(__file__).resolve().parent.parent
ICON_PNG = ROOT / "branding" / "placeholder-icon.png"
ICON_ICO = ROOT / "branding" / "icon.ico"
OUT_DIR = ROOT / "target" / "release-out"
VERSION_RE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", re.ASCII)
CARGO_TOML = ROOT / "Cargo.toml"


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


def run(command, dry_run, check=True, env=None):
    print("+", masked(command), flush=True)
    if dry_run:
        return 0
    result = subprocess.run(command, cwd=ROOT, env=env)
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


def build_studio(jobs, dry_run):
    """Build the manifest's binary without passing GitHub credentials to Cargo."""
    environment = os.environ.copy()
    for name in ("GITHUB_TOKEN", "GH_TOKEN"):
        environment.pop(name, None)
    run(["cargo", "build", "--release", "--locked", "-p", CARGO_PACKAGE,
         "--bin", CARGO_BIN, "--target-dir", str(ROOT / "target"), "-j", str(jobs)],
        dry_run, env=environment)
    executable = ROOT / "target" / "release" / f"{CARGO_BIN}.exe"
    if not dry_run and not executable.is_file():
        sys.exit(f"{executable} not found after the build")
    return executable


def stage_executable(executable, dry_run):
    """Copy one input file into the fixed staging directory without executing it."""
    stage = OUT_DIR / "stage"
    print(f"+ stage {executable.name} as {MAIN_EXE} in {stage.relative_to(ROOT)}")
    if dry_run:
        return stage
    try:
        source_info = executable.lstat()
    except OSError as error:
        sys.exit(f"cannot read input executable: {error}")
    if (not stat.S_ISREG(source_info.st_mode) or executable.is_symlink()
            or getattr(source_info, "st_file_attributes", 0)
            & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)):
        sys.exit("input executable must be a regular file, not a link/reparse point")
    # Verify the resolved recursive-delete target before removing generated files.
    # In particular, refuse an existing target/stage junction pointing elsewhere.
    expected = ROOT.resolve() / "target" / "release-out" / "stage"
    if stage.resolve() != expected:
        sys.exit("stage directory resolves outside its fixed workspace location")
    if executable.resolve().is_relative_to(expected):
        sys.exit("input executable must be outside the generated stage directory")
    if stage.exists():
        if not stage.is_dir():
            sys.exit("stage path is not a directory")
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    shutil.copy2(executable, stage / MAIN_EXE)
    return stage


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--channel", choices=["dev", "stable"])
    parser.add_argument("--version", help="X.Y.Z, for example 0.1.1")
    parser.add_argument("--required", action="store_true",
                        help="mark the release required: Studio then insists on the update")
    parser.add_argument("--no-upload", action="store_true", help="build and pack only")
    phase = parser.add_mutually_exclusive_group()
    phase.add_argument("--build-only", action="store_true",
                       help="only build the Cargo binary; no packaging or token access")
    phase.add_argument("--publish-only", action="store_true",
                       help="pack/upload an existing executable without invoking Cargo")
    parser.add_argument("--input-exe", type=Path,
                        help="regular executable file required by --publish-only")
    parser.add_argument("--dry-run", action="store_true", help="print the commands, run nothing")
    parser.add_argument("--jobs", type=int, default=4, help="cargo -j value")
    parser.add_argument("--vpk", default="vpk", help="path of the vpk executable")
    parser.add_argument("--check-version", action="store_true",
                        help="only check that --version equals the workspace version, then exit")
    parser.add_argument("--print-vpk-version", action="store_true", help="print the pinned vpk version and exit")
    args = parser.parse_args(argv)

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

    if args.jobs < 1:
        parser.error("--jobs must be positive")
    if args.publish_only and args.input_exe is None:
        parser.error("--publish-only requires --input-exe")
    if args.input_exe is not None and not args.publish_only:
        parser.error("--input-exe is only valid with --publish-only")
    if args.build_only:
        executable = build_studio(args.jobs, args.dry_run)
        print(f"Built artifact input: {executable}")
        return

    tag = f"v{args.version}-{args.channel}"
    token = os.environ.get("GITHUB_TOKEN", "")
    if not args.no_upload and not args.dry_run and not token:
        sys.exit("GITHUB_TOKEN is not set (the workflow provides it; by hand: set it from `gh auth token`).")

    check_vpk_version(args.vpk, args.dry_run)

    executable = args.input_exe if args.publish_only else build_studio(args.jobs, args.dry_run)

    stage = stage_executable(executable, args.dry_run)
    notes = OUT_DIR / "release-notes.md"
    if not args.dry_run:
        lines = [f"Metallic Studio {args.version} ({args.channel})"]
        if args.required:
            lines += ["", "required: true"]
        notes.write_text("\n".join(lines) + "\n", encoding="utf-8")

    icon = make_icon(args.dry_run)

    if not args.no_upload:
        # Fetch earlier packages so vpk can build delta updates. The first release has none.
        code = run([args.vpk, "download", "github", "--repoUrl", REPO_URL, "--channel", args.channel,
                    "--outputDir", str(OUT_DIR), "--token", token], args.dry_run, check=False)
        if code != 0:
            print(f"WARNING: 'vpk download github' failed with exit code {code}. Expected for the very first "
                  "release of a channel; otherwise this release will have no delta package.", file=sys.stderr)

    run([args.vpk, "pack", "--packId", PACK_ID, "--packVersion", args.version,
         "--packDir", str(stage), "--mainExe", MAIN_EXE, "--packTitle", PACK_TITLE,
         "--icon", str(icon), "--channel", args.channel, "--outputDir", str(OUT_DIR),
         "--releaseNotes", str(notes)], args.dry_run)

    if args.no_upload:
        print("Packed; not uploading (--no-upload).")
        return

    upload = [args.vpk, "upload", "github", "--repoUrl", REPO_URL, "--channel", args.channel,
              "--outputDir", str(OUT_DIR), "--tag", tag, "--releaseName", f"Metallic Studio {args.version} ({args.channel})",
              "--publish", "--token", token]
    if args.channel == "dev":
        upload.append("--pre")
    run(upload, args.dry_run)
    print(f"Would publish release {tag}." if args.dry_run else f"Published release {tag}.")


if __name__ == "__main__":
    main()
