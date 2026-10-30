# Releasing Metallic Studio

How a team member publishes a build, and how the one-time GitHub setup works. Status: implemented, not yet tested
end to end (the owner round trip is pending, see the last section).

## How it fits together
- `tools/release.py` builds `metallic_studio`, packs it with Velopack (`vpk pack`) and uploads it with
  `vpk upload github` to the private repo `Metallic-Club/studio` as a release tagged `vX.Y.Z-<channel>`.
- `.github/workflows/release.yml` runs the same script when a tag `v*` is pushed. It uses the workflow's own
  `GITHUB_TOKEN`; there are no other secrets.
- Studio (`crates/metallic_studio/src/updater.rs`) lists the releases of its channel, offers a newer one, downloads it
  and restarts. Because the repo is private, each person signs in once with GitHub's device sign-in
  (`github_auth.rs`); the token lives in Windows Credential Manager under `MetallicStudio/github`.
- The Velopack CLI is pinned to `1.3.39-g8d2844f` in `tools/release.py` (`VPK_VERSION`). It is the same version as the
  `velopack` crate in `crates/metallic_studio/Cargo.toml`. Change both together.

## Cut a dev release (the team channel)
1. Merge your work into `main`. Pick the next version `X.Y.Z` (bump the patch for every dev build). The version must
   also be in `[workspace.package] version` of the root `Cargo.toml` so the running Studio knows its own version.
2. Tag and push (the owner or lead does this; never from a feature branch):
   `git tag v0.1.1-dev && git push origin v0.1.1-dev`.
3. Watch the "Release" workflow in GitHub Actions. When it is green, the release `v0.1.1-dev` has
   `MetallicStudio-dev-Setup.exe`, the full and delta packages and `releases.dev.json`.
4. Everyone on the dev channel gets the offer within 6 hours, or at the next Studio start.

By hand instead of the workflow (needs .NET and the pinned `vpk`: `dotnet tool install --global vpk --version
1.3.39-g8d2844f`): set `GITHUB_TOKEN` in your shell from `gh auth token`, then
`python tools/release.py --channel dev --version 0.1.1`. Add `--dry-run` to see the commands, `--no-upload` to pack
only.

## Stable differs in these ways
- Tag `vX.Y.Z-stable`, command `--channel stable`. It is published as a normal release; dev releases are marked
  pre-release.
- Stable and dev are separate feeds. A Studio only sees releases of its own channel (dev ignores `-stable` tags).
- Only the owner decides when a stable release is cut. Dev builds come first; stable is a dev build that was tested.
- A release can be marked required (Studio then insists on restarting): add `--required` to the script, or put the line
  `required: true` in the release notes. Use it only for breaking changes (file format, protocol).

## Register the GitHub OAuth app (owner, one time)
Studio needs a client ID to ask GitHub for a sign-in code. The client ID is public; there is no client secret in Studio.
1. Open GitHub, switch to the organisation Metallic-Club: Settings, Developer settings, OAuth Apps, New OAuth App.
   (Org owner rights are needed.)
2. Application name: `Metallic Studio`. Homepage URL: the Metallic website or the repo URL.
3. Authorization callback URL: GitHub insists on a value; enter `https://github.com/Metallic-Club`. It is never used
   by the device flow.
4. Tick **Enable Device Flow**. Register the application.
5. Copy the **Client ID** (starts with `Iv1.` or `Ov23`). Do not generate or copy a client secret; Studio does not use it.
6. Paste the client ID as the only line of `config/github_oauth_client_id.txt`, replacing `REPLACE_WITH_CLIENT_ID`,
   and commit it (it is not secret). It is compiled into Studio, so a new build is needed.
7. Each team member signs in the first time Studio asks: a code appears, the browser opens
   `https://github.com/login/device`, they type the code and approve. The scope requested is `repo`, which GitHub
   requires to read releases of a private repo. If the organisation restricts OAuth apps, the owner must approve the
   app under Organisation settings, Third-party access.

Until step 6 is done, sign-in reports: "put the OAuth app's client ID into config/github_oauth_client_id.txt".

## Install on a teammate's PC
Download `MetallicStudio-dev-Setup.exe` from the release page (needs repo access), run it. It installs per user (no
admin prompt) with a Start-menu entry and uninstaller. Updates arrive inside Studio as "Restart to update".

## Not done yet (owner round trip)
Nobody has published a real release yet. Still to do with the owner: register the OAuth app and paste the client ID;
publish `0.1.0-dev`; install from the Setup.exe; publish `0.1.1-dev`; see Studio offer and apply it; take screenshots.
Studio's `main` must also call `velopack::VelopackApp::build().run()` first thing (see the task 13 report), otherwise
the installer hooks do not run.
