#!/usr/bin/env python3
"""Run one Cargo build/test command while owning a local artifact lane.

Standard library only. See docs/build-lanes.md for the cache ownership contract.
"""
import argparse
from contextlib import AbstractContextManager
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import time
import tomllib


class LaneError(Exception):
    """Unsafe input or inconsistent Cargo metadata; no build should run."""


COMMANDS = {"build", "check", "test", "clippy", "bench", "run", "doc"}
LOCK_NAME = ".metallic-lane.lock"
OWNER_NAME = ".metallic-lane-owner.json"
CACHE_SIGNATURE = b"Signature: 8a477f597d28d172789f06886806bc55"


def checked_path(path):
    """Return an absolute path, rejecting existing symlink/reparse components."""
    path = Path(os.path.abspath(path))
    for component in [*reversed(path.parents), path]:
        try:
            info = component.lstat()
        except FileNotFoundError:
            continue
        if (stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0)
                & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)):
            raise LaneError(f"symlink/reparse path is not a build lane: {component}")
    return path.resolve()


def checked_lane(path, workspace):
    """Reject roots and lanes containing the workspace before any mutation."""
    lane = checked_path(path)
    if (lane == Path(lane.anchor) or workspace == lane or lane in workspace.parents
            or workspace in lane.parents):
        raise LaneError("target must be a non-root directory outside the source workspace")
    if lane.exists() and not lane.is_dir():
        raise LaneError("target is not a directory")
    return lane


def checked_arguments(arguments):
    """Allow build commands only, with workspace/artifact location fixed here."""
    if not arguments or arguments[0] not in COMMANDS:
        raise LaneError("expected one of: " + ", ".join(sorted(COMMANDS)))
    # Clippy forwards its tail to rustc; other supported tails are application
    # arguments (or rejected by Cargo itself), so do not interpret them as paths.
    if arguments[0] == "clippy" and "--" in arguments:
        tail = iter(arguments[arguments.index("--") + 1:])
        for flag in tail:
            if flag in ("-A", "-W", "-D", "-F", "--cap-lints"):
                value = next(tail, "")
            elif flag.startswith(("-A", "-W", "-D", "-F")) and len(flag) > 2:
                value = flag[2:]
            elif flag.startswith("--cap-lints="):
                value = flag.partition("=")[2]
            else:
                raise LaneError("clippy tail supports lint flags only")
            if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_:-]*", value):
                raise LaneError("invalid clippy lint name/level")
    cargo_arguments = arguments[:arguments.index("--")] if "--" in arguments else arguments
    forbidden = ("--target-dir", "--build-dir", "--artifact-dir", "--out-dir",
                 "--manifest-path", "--config", "--lockfile-path")
    for argument in cargo_arguments[1:]:
        if (any(argument == flag or argument.startswith(flag + "=") for flag in forbidden)
                or argument.startswith(("-C", "-m", "-Z"))):
            raise LaneError(f"Cargo location/config overrides are not allowed: {argument}")
    return list(arguments)


def prepare_cache(lane):
    """Tag only a verified empty lane; refuse unmarked nonempty directories."""
    entries = {path.name for path in lane.iterdir()} - {LOCK_NAME, OWNER_NAME}
    if not entries:
        # Explicit intermediate directories do not always get tagged by Cargo.
        # Never bless an arbitrary nonempty directory, and never overwrite a tag.
        with (lane / "CACHEDIR.TAG").open("xb") as marker:
            marker.write(CACHE_SIGNATURE + b"\n# Metallic empty Cargo lane.\n")
        return False
    marker = lane / "CACHEDIR.TAG"
    checked_path(marker)
    if not marker.is_file() or not marker.read_bytes().startswith(CACHE_SIGNATURE):
        raise LaneError("nonempty target lacks a Cargo cache marker; choose a new empty lane")
    return True


def compilation_selection(arguments, environment):
    """Select matching clean scope; Cargo's default package clean is not release.

    Keep this parser deliberately small: separate short flags are required except
    the common attached package, feature and jobs values, and repeated verbosity.
    Values passed to a test/application after -- do not select Cargo artifacts.
    """
    profile = "release" if arguments[0] == "bench" else "dev"
    targets = []
    values = iter(arguments[1:arguments.index("--")] if "--" in arguments else arguments[1:])
    for argument in values:
        if argument in ("--release", "-r"):
            profile = "release"
        elif argument in ("--profile", "--target"):
            value = next(values, None)
            if not value or value.startswith("-"):
                raise LaneError(f"missing value for {argument}")
            if argument == "--profile":
                profile = value
            else:
                targets.append(value)
        elif argument.startswith("--profile="):
            profile = argument.partition("=")[2]
            if not profile:
                raise LaneError("empty profile")
        elif argument.startswith("--target="):
            target = argument.partition("=")[2]
            if not target:
                raise LaneError("empty target")
            targets.append(target)
        elif (argument.startswith("-") and not argument.startswith("--")
              and len(argument) > 2 and not argument.startswith(("-p", "-F", "-j"))
              and set(argument[1:]) != {"v"}):
            raise LaneError(f"split combined short flags into separate arguments: {argument}")
    # Built-in test/bench profiles share the dev/release output directories.
    profile = {"test": "dev", "bench": "release"}.get(profile, profile)
    if not re.fullmatch(r"[A-Za-z0-9_-]+", profile):
        raise LaneError("profile must be an ASCII Cargo profile name")
    return {"profile": profile, "targets": sorted(set(targets)),
            "configured_target": environment.get("CARGO_BUILD_TARGET", "")}


def configuration_fingerprint(workspace, environment):
    """Notice Cargo config edits without recording their potentially secret text.

    Cargo applies .cargo/config[.toml] from the invocation root and ancestors,
    plus CARGO_HOME. Follow includes so changing an included build.target also
    invalidates provenance. Only hashes, never config contents, enter the owner.
    """
    home = Path(environment.get("CARGO_HOME", Path.home() / ".cargo"))
    if not home.is_absolute():
        home = workspace / home
    directories = {home, *(parent / ".cargo" for parent in [workspace, *workspace.parents])}
    digest = hashlib.sha256()
    active = set()

    def visit(path):
        path = path.resolve()
        if path in active:
            raise LaneError("cyclic Cargo configuration include")
        digest.update(os.fsencode(path) + b"\0")
        if not path.exists():
            digest.update(b"missing\0")
            return
        content = path.read_bytes()
        digest.update(content + b"\0")
        try:
            includes = tomllib.loads(content.decode("utf-8")).get("include", [])
            if not isinstance(includes, list):
                raise ValueError("include must be an array")
            active.add(path)
            for included in includes:
                name = included["path"] if isinstance(included, dict) else included
                if not isinstance(name, str):
                    raise ValueError("include path must be a string")
                visit(path.parent / name)
        except (ValueError, KeyError) as error:
            raise LaneError("invalid Cargo configuration include") from error
        finally:
            active.discard(path)

    for directory in sorted(directories):
        # Cargo prefers the extensionless file when both are present.
        legacy = directory / "config"
        visit(legacy if legacy.is_file() else directory / "config.toml")
    return digest.hexdigest()


class LaneLock(AbstractContextManager):
    """Exclusive process lock retained until the complete invocation exits.

    The lock file is never unlinked: replacing its inode would split ownership.
    OS locks are released on process exit, including an abnormal Python exit.
    """

    def __init__(self, lane):
        self.path = lane / LOCK_NAME
        self.file = None

    def __enter__(self):
        checked_path(self.path)
        self.file = self.path.open("a+b")
        try:
            if os.name == "nt":
                import msvcrt
                # Nonblocking + retry avoids msvcrt.LK_LOCK's finite retry limit.
                while True:
                    self.file.seek(0)
                    try:
                        msvcrt.locking(self.file.fileno(), msvcrt.LK_NBLCK, 1)
                        break
                    except OSError as error:
                        if error.errno not in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                            raise
                        time.sleep(0.05)
            else:
                import fcntl
                fcntl.flock(self.file.fileno(), fcntl.LOCK_EX)
            return self
        except BaseException:
            self.file.close()
            raise

    def __exit__(self, *exception):
        # Closing releases the lock on both platforms.
        self.file.close()


def workspace_packages(metadata, workspace, lane):
    """Get all local workspace package names from Cargo, never a fixed list."""
    if Path(metadata["workspace_root"]).resolve() != workspace:
        raise LaneError("Cargo metadata workspace differs from the requested workspace")
    if Path(metadata["target_directory"]).resolve() != lane:
        raise LaneError("Cargo metadata target differs from the guarded lane")
    members = set(metadata["workspace_members"])
    packages = sorted(package["name"] for package in metadata["packages"]
                      if package["id"] in members and package.get("source") is None)
    if not packages or len(packages) != len(members):
        raise LaneError("expected a nonempty workspace of local packages")
    return packages


def write_owner(path, owner):
    """Atomically publish provenance after cleanup, never a success claim."""
    checked_path(path)
    descriptor, temporary = tempfile.mkstemp(prefix=".metallic-owner-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(owner, output, sort_keys=True)
            output.write("\n")
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def execute(command, *, capture_output=False, **options):
    """Retain the lane lock while a child exits after a console interruption.

    Console Ctrl-C normally reaches both processes. Do not release our lock
    while Cargo is still unwinding; repeated Ctrl-C also waits for its exit.
    Forced process termination remains outside this cooperative lock contract.
    """
    output = subprocess.PIPE if capture_output else None
    with subprocess.Popen(command, stdout=output, stderr=output, text=True,
                          encoding="utf-8", **options) as process:
        while True:
            try:
                stdout, stderr = process.communicate()
                return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)
            except KeyboardInterrupt:
                print("cargo-lane: waiting for Cargo to exit before releasing lane", file=sys.stderr)


def run_lane(workspace, target, arguments, *, reset=False, cargo=("cargo",)):
    """Run metadata, optional package clean and command under a single lock.

    Cargo runs as argument arrays without a shell. Every Cargo failure returns
    its status; a failed clean cannot publish a new owner or launch the build.
    ``cargo`` is an executable prefix for the isolated stub-process tests.
    """
    arguments = checked_arguments(arguments)
    selection = compilation_selection(arguments, os.environ)
    workspace = Path(workspace).resolve(strict=True)
    if not (workspace / "Cargo.toml").is_file():
        raise LaneError("workspace must contain Cargo.toml")
    lane = checked_lane(target, workspace)
    lane.mkdir(parents=True, exist_ok=True)
    # Recheck after creation before creating lock/state or invoking Cargo.
    checked_lane(lane, workspace)
    environment = os.environ.copy()
    environment["CARGO_TARGET_DIR"] = str(lane)
    environment["CARGO_BUILD_TARGET_DIR"] = str(lane)
    # Keep intermediates inside the same lock even if user config splits them.
    environment["CARGO_BUILD_BUILD_DIR"] = str(lane)
    with LaneLock(lane):
        checked_lane(lane, workspace)
        state = lane / OWNER_NAME
        checked_path(state)
        populated = prepare_cache(lane)
        options = {"cwd": workspace, "env": environment}
        result = execute([*cargo, "metadata", "--no-deps", "--format-version", "1",
                          "--locked"], capture_output=True, **options)
        if result.returncode:
            sys.stderr.write(result.stderr)
            return result.returncode
        try:
            packages = workspace_packages(json.loads(result.stdout), workspace, lane)
        except (ValueError, KeyError, TypeError) as error:
            raise LaneError(f"invalid Cargo metadata: {error}") from error
        owner = {"schema": 2, "workspace": str(workspace), "target": str(lane),
                 "packages": packages, "selection": selection,
                 "configuration": configuration_fingerprint(workspace, environment)}
        try:
            previous = json.loads(state.read_text(encoding="utf-8"))
        except (FileNotFoundError, ValueError, UnicodeError):
            previous = None
        if reset or previous != owner:
            if populated:
                print("cargo-lane: cleaning local workspace packages for lane ownership", flush=True)
                clean = [*cargo, "clean", "--target-dir", str(lane),
                         "--profile", selection["profile"]]
                for target in selection["targets"]:
                    clean.extend(["--target", target])
                for package in packages:
                    clean.extend(["--package", package])
                result = execute(clean, **options)
                if result.returncode:
                    return result.returncode
            write_owner(state, owner)
        command = [*cargo, arguments[0], "--target-dir", str(lane), *arguments[1:]]
        return execute(command, **options).returncode


def main(argv=None):
    """CLI: python tools/cargo_lane.py --target-dir PATH -- test --workspace -j 4."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--reset", action="store_true", help="force package-only recovery clean")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    options = parser.parse_args(argv)
    arguments = options.command[1:] if options.command[:1] == ["--"] else options.command
    try:
        return run_lane(options.workspace, options.target_dir, arguments, reset=options.reset)
    except (LaneError, OSError) as error:
        print(f"cargo-lane: {error}", file=sys.stderr)
        return 2
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
