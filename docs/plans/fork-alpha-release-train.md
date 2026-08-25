# Fork Alpha/Beta Release Train

Date: 2026-08-25
Owner: aberoham
Status: approved — phase 1 implementation in progress
Reference implementation: `osodevops/ms-teams-cli` maintainer Sion Smith

## Purpose

You contribute long-lived branches to several upstream projects and want to
cut installable alpha and beta packages from your own ahead work before
upstream lands it. You do not want a hard fork, just a personal distribution
point that is occasionally ahead and that tracks upstream quickly afterward.

This plan covers the reusable pattern and its three first targets:

- `osodevops/ms-teams-cli` (Rust) fork `aberoham/ms-teams-cli`
- `rlrghb/olkcli` (Go, GoReleaser) fork `aberoham/olkcli` at
  `/Users/ingersolla/Desktop/work/claude/ms-outlook-cli`
- `TheHutGroup/ms-entra-cli` (Go, no release automation) at
  `/Users/ingersolla/Desktop/work/claude/ms-entra-cli`, visibility
  `internal`

Phase 1 implements the full pattern for `ms-teams-cli`. Phases 2 and 3
repeat it for the two Go repos with the deltas noted below.

## Decisions confirmed 2026-08-25

- Integration branch is `next` (not `alpha`). Alphas and betas both tag from
  `next`.
- Homebrew distribution overwrites the `teams` binary in the personal tap
  `aberoham/tap`. Side-by-side coexistence via a second formula or binary
  alias `teams-next` is deferred until someone needs both kegs on one machine.
  See section 8 for the deep trade-off.
- Scoop buckets are deferred. First alpha will ship only GitHub Releases and
  Homebrew. Scoop is Windows-only and the verification step can be re-added
  later behind a flag.
- Private entra work is gated on a public mirror. Cutting installable Homebrew
  packages from an `internal` repo is not possible without moving code to a
  public repo. Noted as prerequisite in section 7 and risks in section 9.
- Prerelease version base floats ahead of upstream head. If `next` falls
  behind `origin/main`, rebase `next` onto `main` and bump the next tag to a
  version larger than both heads, keeping the suffix. See section 4, Versioning and rebasing.

## 1. Survey — what Sion Smith runs upstream for teams

Sources: `Cargo.toml:5`, `.github/workflows/auto-tag.yml`,
`.github/workflows/release.yml`, `.github/workflows/ci.yml`,
`docs/release-readiness.md`, `Formula/teams-cli.rb` and
`scripts/update-teams-formula.rb` in `osodevops/homebrew-tap`.

### Version source of truth

`Cargo.toml` package version is the source. `auto-tag.yml` watches pushes to
`main` with `paths: [Cargo.toml]`, diffs `Cargo.toml` to detect a bump, and
pushes an annotated tag `vX.Y.Z`. The tag push is what triggers `release.yml`.

### Release pipeline

`release.yml` on `push tags v*`:

1. Matrix 5 targets: `x86_64-apple-darwin` (macos-14), `aarch64-apple-darwin`
   (macos-latest), `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`
   (ubuntu-latest, the latter via `cross`), `x86_64-pc-windows-msvc`
   (windows-latest).
2. Native or `cross` build, then packages `teams-vX.Y.Z-<target>.tar.gz` or
   `.zip` containing `bin/teams` plus `docs/man/teams.1`,
   `teams-config.5`, `teams-auth.7`, `teams-agent-contract.7`,
   `teams-examples.7` and doc copies of `README.md`, `CHANGELOG.md`, `LICENSE`,
   `SECURITY.md`.
3. Uploads artifacts, downloads all in a `release` job, generates
   `checksums-sha256.txt`, creates the GitHub Release with
   `softprops/action-gh-release`.
4. Dispatches `osodevops/homebrew-tap` via `peter-evans/repository-dispatch`
   with `HOMEBREW_TAP_TOKEN` and payload `{formula: teams-cli, tag, repo}` and
   dispatches `osodevops/scoop-bucket`. The Homebrew job then polls
   `api.github.com/repos/osodevops/homebrew-tap/contents/Formula/teams-cli.rb`
   for 5 minutes until the 4 platform URLs and sha256 values match.

### Homebrew and Scoop receivers

`osodevops/homebrew-tap/.github/workflows/update-teams-formula.yml` validates
`TAG =~ ^v\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)?$` and
`RELEASE_REPOSITORY == osodevops/ms-teams-cli`, downloads
`checksums-sha256.txt`, runs `ruby scripts/update-teams-formula.rb` which
regex-replaces 4 `url` + `sha256` blocks, then commits.

`osodevops/scoop-bucket/.github/workflows/update-manifest.yml` has 8 product
jobs; the `update-teams` job rewrites `bucket/teams.json` with the new version,
URL, and sha256 for the Windows zip.

### CI

`ci.yml` runs `cargo check`, `cargo fmt -- --check`, `cargo clippy -D warnings`,
and `cargo test` on ubuntu, macos, windows.

### Fork state at survey time

`aberoham/ms-teams-cli` fork at `v0.4.0`, branch `main` tracks upstream,
ahead branches `fix/presence-writes` (5 ahead), `feat/auth-list-identities`,
`feat/message-soft-delete`, `fix/presence-status-expiry`,
`fix/windows-credential-chunking`. No releases. `aberoham/homebrew-tap` fork
exists but is stale at `v0.2.7`.

## 2. Survey — olkcli

Sources: `go.mod`, `.goreleaser.yaml`, `.goreleaser-prebuild.yaml`, `Makefile`,
`.github/workflows/ci.yml`, `.github/workflows/release.yml`, `Casks/olk.rb`,
`npm/`, `scripts/build-npm.mjs`.

- Go 1.26.4. No Cargo analogue. Version injected via ldflags
  `-X internal/cmd.Version={{.Version}}`.
- Release on `push tags v*` with a two-stage pipeline: `pre-build` on ubuntu
  runs goreleaser on `.goreleaser-prebuild.yaml` to produce linux and windows
  archives (CGO_ENABLED=0, `release.disable: true`), uploads to
  `prebuilt-archives/`. `release` on macos-latest downloads those and runs
  goreleaser on `.goreleaser.yaml` to produce darwin amd64 and arm64
  (CGO_ENABLED=1) and to publish the unified Release, checksums, and Cask.
- `homebrew_casks` publishes Cask `olk` to `rlrghb/homebrew-tap` with
  `TAP_GITHUB_TOKEN`, including the quarantine `xattr -dr` postflight hook for
  unsigned macOS binaries. Cask lives at `Casks/olk.rb`, not `Formula/`.
- Extra publish stages for npm `olkcli` and MCP Registry gated on
  `vars.PUBLISH_NPM == true`.

Fork: `aberoham/olkcli` at `local/main-plus-prs-92-94` ahead by HTML replies,
folder paths, delegated mailbox writes.

## 3. Survey — entra

- Go 1.26.4, `cmd/entra`, single binary `entra`.
- No `.github` workflows, no goreleaser configs, no tap.
- Repo visibility `internal` at `TheHutGroup/ms-entra-cli`. No tags or releases.

## 4. Common design

### Branching

- `main` tracks `origin/main` exactly. No divergence, easy sync.
- `next` is the integration branch that merges your ahead PRs. Every alpha or
  beta tag is cut from `next`. This avoids the upstream `auto-tag.yml` path
  filter and lets you rebase freely.

### Tag trigger

Keep the idiomatic `on: push: tags: v*` in each repo. Allow manual tag push

```
git tag v1.13.0-alpha.1 && git push fork v1.13.0-alpha.1
```

as the primary cut mechanism, with optional `workflow_dispatch` with a `tag`
input as a safety net. Do not retarget `auto-tag.yml` to `next`; disable it in
the fork instead.

### Versioning and rebasing

Use SemVer prerelease identifiers: `vX.Y.Z-alpha.N`, then `vX.Y.Z-beta.1`, then
stable `vX.Y.Z`. GitHub marks any tag containing `-` as a prerelease, GoReleaser
and `softprops/action-gh-release` do the same automatically. Homebrew and
`--version` output show the full string.

Practical rule when `next` falls behind: upstream `main` shipped `v0.5.0` while
your `next` was at `v0.5.0-alpha.1`. Rebase `next` onto `main`, then set the
next tag to a version larger than both heads, such as `v0.6.0-alpha.1` or
`v0.5.1-alpha.1`. The suffix guarantees ordering `alpha.1 < alpha.2 < beta.1 <
stable` so users never confuse your build with a finished release.

cargo and ldflags both carry the prerelease. For Rust bump `Cargo.toml` to
`0.5.0-alpha.1`; for Go use the tag value directly.

### Distribution

- Personal tap `aberoham/homebrew-tap` holds all three tools:
  `Formula/teams-cli.rb`, `Casks/olk.rb`, `Formula/entra.rb` or `Casks/entra.rb`.
  Reuses the existing fork of `osodevops/homebrew-tap` rather than creating a
  tap per project.
- Fork releases dispatch to the personal tap with a PAT. For olk set
  `vars.PUBLISH_NPM` to false in the fork so npm and MCP steps are skipped for
  alphas.
- Entra prereleases from an `internal` repo cannot be referenced by a public
  tap. Ship entra alphas as private GitHub Releases reachable only by org
  members or via `go install @version` until a public mirror exists.

### Secrets

Per fork repo that dispatches:

- `HOMEBREW_TAP_TOKEN` or `TAP_GITHUB_TOKEN`: PAT with `repo` and `workflow`
  scope on `aberoham/homebrew-tap`, or a fine-grained PAT with
  `contents: write`. Create in the fork's repository secrets.
- `SCOOP_BUCKET_TOKEN`: only if Scoop re-enabled for teams.
- Tag creation uses the workflow `GITHUB_TOKEN` with `contents: write`, not the
  Homebrew token.


## 5. Per-repo delta — teams (phase 1)

This is the reference implementation. Implement first because it has the most
complete upstream pattern to model from.

Files to patch in `aberoham/ms-teams-cli`:

- `.github/workflows/release.yml`
  - Retarget `homebrew.repository` from `osodevops/homebrew-tap` to
    `aberoham/homebrew-tap`.
  - Parameterize `RELEASE_REPOSITORY` and `GITHUB_REPOSITORY` instead of
    hardcoding `osodevops/ms-teams-cli`. Let verification URLs derive from
    `${{ github.repository }}` and `${{ github.ref_name }}`.
  - Add `prerelease: ${{ contains(github.ref_name, '-') }}` to the
    `softprops/action-gh-release` step so tags with `-` are flagged.
  - Drop or gate `publish-scoop-manifest` behind a repo variable until a
    bucket fork exists.
  - Adjust Homebrew verification poll to query
    `aberoham/homebrew-tap/contents/Formula/teams-cli.rb` and expect URLs
    `github.com/${GITHUB_REPOSITORY}/releases/download/${RELEASE_TAG}/`.

- `.github/workflows/auto-tag.yml`
  - Disable in the fork by adding `if: github.repository ==
    'osodevops/ms-teams-cli'` guard. Alphacuts will be manual tags.

Files to patch in `aberoham/homebrew-tap`:

- `scripts/update-teams-formula.rb`: relax the repo check from
  `repository == "osodevops/ms-teams-cli"` to allow
  `aberoham/ms-teams-cli`, or remove the check and take the repo from args.
  Also allow the URL regex to match `github.com/<any>/ms-teams-cli` instead
  of hardcoded `osodevops`.

- `.github/workflows/update-teams-formula.yml`: relax the validation guard
  `if RELEASE_REPOSITORY != osodevops/ms-teams-cli` and change the URL prefix
  to `${{ env.RELEASE_REPOSITORY }}` so checksums are fetched from the fork
  release.

Flow for first alpha:

1. Clone updater changes into `aberoham/homebrew-tap` and push to `main`.
2. Create PAT with `repo` on `aberoham/homebrew-tap`, add as
   `HOMEBREW_TAP_TOKEN` secret in `aberoham/ms-teams-cli` repository settings
   via `gh secret set`.
3. Create branch `next` from `main`, merge desired ahead branches, bump
   `Cargo.toml` version to `0.5.0-alpha.1`, update `CHANGELOG.md`.
4. `git tag v0.5.0-alpha.1 && git push fork v0.5.0-alpha.1`.
5. Watch actions: matrix 5, checksums, prerelease flagged, tap workflow runs,
   formula shows 4 new URLs and sha256 values, verification poll passes.
6. Local check: `brew tap aberoham/tap && brew install --build-from-source
   aberoham/tap/teams-cli` or bottle install, then `teams --version` prints
   `0.5.0-alpha.1`.

## 6. Per-repo delta — olkcli (phase 2)

- In `aberoham/olkcli`, patch `.goreleaser.yaml` `homebrew_casks.repository`
  to `owner: aberoham, name: homebrew-tap` so the Cask is published to the
  personal tap. For alphas reuse Cask `olk` in the personal tap only — do not
  overwrite `rlrghb/homebrew-tap/Casks/olk.rb`. Consider a future
  `Casks/olk-next.rb` if side-by-side is wanted.
- In `.github/workflows/release.yml`, the two-stage goreleaser flow stays. Set
  repository variable `PUBLISH_NPM` to `false` in the fork so `npm-publish`
  and `registry-publish` jobs are skipped for prereleases.
- Ensure Go module path handling: if the fork keeps `module
  github.com/rlrghb/olkcli`, ldflags path stays
  `github.com/rlrghb/olkcli/internal/cmd` even when built from fork.
- Create branch `next`, tag `v1.13.0-alpha.1` ahead of upstream `v1.12.0`,
  watch linux/windows prebuild then macos release, verify Cask hash.

## 7. Per-repo delta — entra (phase 3, gated)

Prerequisite: public repo for release artifacts.

- Add `.github/workflows/ci.yml` (go vet, build, test on ubuntu, macos,
  windows).
- Add `.goreleaser.yaml` minimal: build `entra` binary, archives
  `entra_{{Version}}_{Os}_{Arch}}`, checksum, homebrew tap to
  `aberoham/homebrew-tap` as Formula or Cask.
- Add `.github/workflows/release.yml` on `push tags v*` that runs
  `goreleaser/goreleaser-action` with `TAP_GITHUB_TOKEN`.
- First tag `v0.1.0-alpha.1` from `next` or `main`.

Until the repo is public, ship alpha as tag `v0.1.0-alpha.1` with attached
tarballs reachable only inside the org, plus `go install
github.com/TheHutGroup/ms-entra-cli/cmd/entra@v0.1.0-alpha.1`.

## 8. Overwriting versus side-by-side — deep trade

Decision: ship overwriting now, reserve side-by-side.

### A. Overwriting: single formula and single binary `teams`

Your tap `aberoham/tap/teams-cli` shadows `osodevops/tap/teams-cli`. A user
who taps yours gets your build at `$(brew --prefix)/bin/teams`. Upgrading
replaces whichever was linked last.

Pros

- No code rename. `Cargo.toml` `[[bin]] name = teams`, man pages `teams.1`,
  docs `teams message ...`, tests `teams --help`, and skill files all stay
  identical. No divergence to rebase.
- One formula, one update script, one verification check.
- Simple mental model: one install, it is either stable or next train.
- Verification already exists for one keg.

Cons

- Stable and alpha cannot coexist on one machine. Comparing behaviors requires
  reinstall and re-link.
- If a user has both taps active, `brew upgrade` ambiguity and pin
  confusion.
- Shared config dir `~/.config/teams-cli` and OS keychain entries are shared
  so schema changes in alpha can affect stable, though this is minor.

### B. Coexistent: versioned formula or alias binary `teams-next`

Publish a second formula, by convention `teams-cli@next` or `teams-next`,
installing to `bin/teams-next` with its own keg and man pages.

Implementation cost

- Cargo: add alias install `bin.install "bin/teams" => "teams-next"` or a
  second `[[bin]]`, plus man page rename, completions name change, tests.
- Go: Cask `binary: olk-next`, optional `npm/olk-next`.
- Tap needs two files and update logic branching per tag, release must
  decide which formula to patch per run.

Pros

- Stable and alpha run side by side, ideal for QA and for using different
  Entra profiles or tenants.
- Isolated config and keychain if you also change config dir name.

Cons

- Maintenance roughly doubles. Two checksums blocks, two livecheck stanzas,
  two README lines, every release chooses target and risks drift.
- User confusion on which command to type. Every bug report must name the
  binary.
- Homebrew policy reserves `@version` for stable major lines, not rolling
  alphas. A `teams-cli@next` formula would be non-idiomatic and might not
  pass `brew audit`.
- Rename ripples into shell completions, skills, man page names, and docs.

Recommendation: keep overwriting as the default alpha path. If a concrete need
for simultaneous installs appears, add a second formula `teams-next` with alias
binary `teams-next`, gated in `release.yml` on tag pattern such as
`*-next.*`, instead of paying the cost now.

## 9. Risks and mitigations

- Tap updater rejects fork releases because `update-teams-formula.rb` hardcodes
  `osodevops/ms-teams-cli`. Mitigate by patching tap repo first before first
  fork alpha.
- Tag collision: pushing `v0.5.0` without prerelease suffix would shadow a
  future upstream stable. Enforce in docs and optionally in a pre-push hook
  that tags on the fork must contain `-alpha` or `-beta`.
- Stale `aberoham/homebrew-tap` `Formula/teams-cli.rb` still at `v0.2.7` until
  first alpha deploys. First alpha self-heals this.
- Token scope mistakes cause dispatch to be accepted but not run. Mitigated by
  the verification poll that must query the personal tap, not upstream.
- Private entra taps cannot fetch private archives. Mitigate by gating entra
  phase on a public mirror repo.

## 10. Implementation order

1. Write this spec.
2. Phase 1 teams: patch `aberoham/homebrew-tap` then
   `aberoham/ms-teams-cli` workflows, create `next`, tag first prerelease,
   verify.
3. Phase 2 olkcli: patch goreleaser tap target, fork release vars, tag.
4. Phase 3 entra: add goreleaser and workflows after public repo exists.
5. Extract a template checklist for the next tool.

## 11. What is a Scoop bucket

Scoop is the Windows counterpart to Homebrew on macOS and Linux. You run
`scoop install <tool>` and it fetches a zip from GitHub Releases. A bucket is
the registry that tells Scoop where to fetch. It is a GitHub repo full of JSON
files, one per tool. Each JSON has `version`, `url`, and `hash` pointing at the
Windows zip you published, plus `bin` mapping. Upstream has
`osodevops/scoop-bucket` with `bucket/teams.json`. The release pipeline's
`publish-scoop-manifest` job dispatches to that repo, which rewrites the JSON.
For fork alphas you can defer Scoop and let Windows testers download the zip
from the Releases page. When needed, fork `aberoham/scoop-bucket`, patch
`update-manifest.yml` to accept `aberoham/ms-teams-cli`, and wire
`SCOOP_BUCKET_TOKEN`.

## 12. References

- Sion Smith upstream release pipeline at
  https://github.com/osodevops/ms-teams-cli
- `osodevops/homebrew-tap` updater
  `scripts/update-teams-formula.rb` and
  `.github/workflows/update-teams-formula.yml`
- `osodevops/scoop-bucket` `bucket/teams.json`
- `rlrghb/olkcli` goreleaser configs and Cask `Casks/olk.rb`
