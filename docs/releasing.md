# Releasing Metallic Studio

How a team member publishes a build, and how the one-time GitHub setup works. Status: implemented, not yet tested
end to end (the owner round trip is pending, see the last section).

## How it fits together
- `tools/release.py` builds package `metallic_studio` and binary `metallic-studio.exe`, packs it with Velopack (`vpk pack`) and uploads it with
  `vpk upload github` to the private repo `Metallic-Club/studio` as a release tagged `vX.Y.Z-<channel>`.
- `.github/workflows/release.yml` splits a tag release across two fresh runners. The build job has `contents: read`,
  removes checkout credentials and runs only `--build-only`, without a publishing token or release build cache.
  The publish job has `contents: write`, checks out the tagged SHA again, and runs `--publish-only --input-exe PATH`.
  It never invokes Cargo. Only its pack/upload step receives `GITHUB_TOKEN`; there are no other secrets.
  Both jobs check the tag's main ancestry and workspace version. Every action is pinned to a full commit SHA.
- Studio (`crates/metallic_studio/src/updater.rs`) lists the releases of its channel, offers a newer one, downloads it
  and restarts. Because the repo is private, each person signs in once with GitHub's device sign-in
  (`github_auth.rs`); the session lives in Windows Credential Manager under `MetallicStudio/github`.
- The Velopack CLI is pinned to `1.3.39-g8d2844f` in `tools/release.py` (`VPK_VERSION`). It is the same version as the
  `velopack` crate in `crates/metallic_studio/Cargo.toml`. Change both together.

## Credential and artifact boundary

The build job uploads only `metallic-studio.exe` with upload-artifact v7, without
an archive. The publisher downloads the immutable artifact ID produced by that
job in this workflow run. Download-artifact v8 checks the server digest with
`digest-mismatch: error`; `skip-decompress: true` keeps input data from extracting
over the trusted checkout. Scripts, release notes and icons come from the fresh
tagged checkout, not the build artifact. The script copies the binary to the same
installed name, `Metallic Studio.exe`, and preserves the existing package ID,
installer/feed names and download-before-pack delta flow.

This keeps Cargo, proc macros and dependency build scripts off the runner that
has publishing permission. It does not establish that the compiled application
is trustworthy: a compromised dependency can still alter the application it
builds. The publisher also trusts its pinned packaging tools to parse the input.
No signing or deployment runs as part of the helper tests. Authenticode signing
and its certificate remain an owner decision; this change does not add it.

The tag ruleset below is an authorization requirement. Someone who can push an
arbitrary tag can also select workflow source, so the workflow's ancestry check
alone is not a release-authorization boundary. The related wording/setup follow-up
is tracked in [#206](https://github.com/Metallic-Club/studio/issues/206).

Both jobs use Python 3.13; manual use needs Python 3.11+ for `tomllib`. To inspect
the phases without building, downloading, installing or uploading anything:

```text
python -I tools/release.py --build-only --channel dev --version 0.1.0 --dry-run
python -I tools/release.py --publish-only --input-exe PATH/metallic-studio.exe --channel dev --version 0.1.0 --dry-run
python -I tools/test_release.py
```

Use the current workspace version in the examples. The tests exercise both CLI
phases with subprocess stubs and temporary inert input bytes, check workflow
credential boundaries, and compare staging names with the real Cargo manifest
([#236](https://github.com/Metallic-Club/studio/issues/236)). A separate read-only
`release-checks.yml` runs them on relevant PRs without starting a release.

Sources: [GitHub credential hardening](https://docs.github.com/en/actions/reference/security/secure-use),
[upload-artifact](https://github.com/actions/upload-artifact),
[download-artifact](https://github.com/actions/download-artifact), and the pinned
[Velopack Windows pack path](https://github.com/velopack/velopack/blob/8d2844f/src/vpk/Velopack.Packaging.Windows/Commands/WindowsPackCommandRunner.cs).

## Cut a dev release (the team channel)
1. Pick the next version `X.Y.Z` (bump the patch for every dev build), set it in `[workspace.package] version` of the
   root `Cargo.toml`, and merge to `main`. The release fails if `--version` (or the tag) differs from that value, so a
   Studio always knows its own version.
2. Tag the merged commit and push (the owner or lead does this; a tag on a commit that is not on `main` is refused):
   `git tag v0.1.1-dev && git push origin v0.1.1-dev`.
3. Watch the "Release" workflow in GitHub Actions. When it is green, the release `v0.1.1-dev` has
   `MetallicStudio-dev-Setup.exe`, the full and delta packages and `releases.dev.json`.
4. Everyone on the dev channel gets the offer within 6 hours, or at the next Studio start.

By hand instead of the workflow (needs .NET and the pinned `vpk`: `dotnet tool install --global vpk --version
1.3.39-g8d2844f`): set `GITHUB_TOKEN` in your shell from `gh auth token`, then
`python tools/release.py --channel dev --version 0.1.1`. Add `--dry-run` to see the commands, `--no-upload` to pack
only, `--check-version` to only check the version. The vpk CLI has no documented environment variable for its token, so
the script passes `--token` (masked in the script's own output, but visible to other processes on that PC while vpk
runs). The manual combined command removes `GITHUB_TOKEN` and `GH_TOKEN` from the
Cargo child's environment, but cannot isolate other credentials or processes on
the local account. Prefer the workflow's separate runners for that boundary.

## Stable differs in these ways
- Tag `vX.Y.Z-stable`, command `--channel stable`. It is published as a normal release; dev releases are marked
  pre-release.
- Stable and dev are separate feeds. A Studio only sees releases of its own channel (dev ignores `-stable` tags).
- Only the owner decides when a stable release is cut. Dev builds come first; stable is a dev build that was tested.
- A release can be marked required (Studio then insists on restarting): add `--required` to the script, or put the line
  `required: true` in the release notes. Use it only for breaking changes (file format, protocol).

## Protect the release tags (owner, one time)
Anyone who can push a tag `v*` can start a release. Add a tag ruleset: repo `Metallic-Club/studio`, Settings, Rules,
Rulesets, New ruleset, New tag ruleset. Target tags matching `v*`; enable Restrict creations, Restrict updates and
Restrict deletions, and put only the owner (and the lead account) on the bypass list.

## Register the GitHub App (owner, one time)
Studio uses a GitHub App, not an OAuth App: an OAuth App with the `repo` scope would give each user's token read and
write access to every private repo that person can see. The App below can only read file contents of one repo. Studio
needs the App's client ID to ask GitHub for a sign-in code. The client ID is public; Studio never uses a client secret
or a private key.
1. Open GitHub, switch to the organisation Metallic-Club: Settings, Developer settings, GitHub Apps, New GitHub App.
   (Org owner rights are needed.)
2. GitHub App name: `Metallic Studio` (must be unique on GitHub; add a suffix if taken). Homepage URL: the Metallic
   website or the repo URL.
3. Identifying and authorizing users: leave the callback URL empty if GitHub allows it, otherwise enter
   `https://github.com/Metallic-Club` (the device flow never uses it). Tick **Enable Device Flow**.
   Leave "Expire user authorization tokens" ticked (the default): Studio refreshes them by itself, and the refresh
   token is kept in Credential Manager next to the access token. Leave "Request user authorization (OAuth) during
   installation" and "Redirect on update" off.
4. Webhook: untick **Active** (no webhook).
5. Permissions, Repository permissions: **Contents: Read-only**. Nothing else (metadata read is added automatically).
   No organisation or account permissions.
6. Where can this GitHub App be installed: **Only on this account** (Metallic-Club).
7. Create the app. On its page copy the **Client ID** (starts with `Iv1.` or `Iv23`). Do not generate a client secret or
   a private key.
8. Install the app: app page, Install App, Metallic-Club, **Only select repositories**, choose `Metallic-Club/studio`
   only.
9. Paste the client ID as the only line of `config/github_app_client_id.txt`, replacing `REPLACE_WITH_CLIENT_ID`, and
   commit it (it is not secret). It is compiled into Studio, so a new build is needed.
10. Each team member signs in the first time Studio asks: a code appears, the browser opens
    `https://github.com/login/device`, they type the code and approve. They also need read access to the repo
    themselves; the App only limits what the token may do, it never adds access.

If you turn off "Expire user authorization tokens" instead, tokens never expire and no refresh token exists; Studio
handles both. Expiring tokens are the safer choice.

Until step 9 is done, sign-in reports: "put the GitHub App's client ID into config/github_app_client_id.txt".

## Install on a teammate's PC
Download `MetallicStudio-dev-Setup.exe` from the release page (needs repo access), run it. It installs per user (no
admin prompt) with a Start-menu entry and uninstaller. Updates arrive inside Studio as "Restart to update".

## Not done yet (owner round trip)
Nobody has published a real release yet. Still to do with the owner: register the GitHub App and paste the client ID;
add the tag ruleset; publish `0.1.0-dev`; install from the Setup.exe; publish `0.1.1-dev`; see Studio offer and apply
it; take screenshots. Studio's `main` must also call `velopack::VelopackApp::build().run()` first thing (see the task 13
report), otherwise the installer hooks do not run.
