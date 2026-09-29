# macOS release signing

macOS release binaries use Developer ID with a stable identifier, hardened
runtime, and an Apple timestamp. Signing fails closed when credentials are
missing or the signature does not match the configured team and identity.

The `release` GitHub environment holds `MACOS_CERTIFICATE_P12_BASE64` (the
base64-encoded, password-protected PKCS #12 certificate/private-key bundle)
and `MACOS_CERTIFICATE_PASSWORD`. Its variables are `MACOS_SIGNING_IDENTITY`
(the certificate SHA-1 fingerprint) and `APPLE_TEAM_ID`. Restrict this
environment to the release tags accepted by the workflow. Require approval
by `aberoham`, allow self-review, and disable administrator bypass. Protect
`v*` tags with an admin-only creation/update/deletion ruleset. These live
settings must be applied before the workflow is enabled. The job actor guard
is defense in depth; it cannot replace these server-side controls. Review
the tagged workflow and signing script before approving a deployment.

The signing helper imports into a temporary keychain under `RUNNER_TEMP`,
adds Apple's public G2 intermediate, and restores the original keychain search
list during unconditional cleanup. It does not change certificate trust or
import credentials into the login keychain. The public intermediate is from
https://www.apple.com/certificateauthority/DeveloperIDG2CA.cer.

The signing job also notarizes each binary with the team API key. `APPLE_NOTARY_KEY_P8_BASE64` is a `release` environment secret; `APPLE_NOTARY_KEY_ID` and `APPLE_NOTARY_ISSUER_ID` are variables. The key reaches only the sign step, and is decoded into a private temporary directory for one submission. The job fails unless Apple returns `Accepted`. A bare binary cannot be stapled, so Gatekeeper checks the ticket online the first time a quarantined copy runs.

Signing runs after compilation and before release checksums are generated.
Keep the signing identifier stable across versions. Existing keychain items
created by older signatures may require one approval or a new sign-in. A
successful release signature does not prove that existing keychain grants
have migrated. Do not re-sign these release binaries with a local certificate.

## Mirrored upstream releases

`.github/workflows/mirror-upstream.yml` runs on weekday mornings from `next`,
the default branch, or by hand with an upstream tag, and does nothing from any
other branch. From osodevops/ms-teams-cli's 20 most recent stable releases it
takes the oldest one not yet mirrored, so two releases between runs are both
built; an older omission needs a run by hand with its tag. It stops for a
person, rather than skipping or overwriting, when an `upstream-vX.Y.Z` release
is a draft or lacks any of its seven expected assets, when the tag exists
without a release, when a lookup fails other than with 404, and when the
upstream tag is not on upstream's `main`.

It calls `release.yml` with upstream's repository and commit. The CI, build,
signing, notarization and attestation jobs are the ones a tag release runs;
CI and the build check out upstream's commit from upstream's own repository,
built with `--locked`, while the workflow and signing script come from `next`.
`release.yml` itself refuses a mirror that does not run from `next`, does not
build osodevops/ms-teams-cli, or builds any commit other than the one upstream
tagged.

The release is published as `upstream-vX.Y.Z`, with archives named
`teams-vX.Y.Z-<target>`, and is what the tap's `teams-cli` formula installs.
Its tag marks the `next` commit whose workflow built it, not upstream's commit:
the fork never needs upstream's history, and a tag on a commit whose workflow
files differ from the default branch would need a permission the workflow
token cannot hold. The provenance attestation therefore names the fork's
workflow run. `teams-vX.Y.Z-source.txt`, attested with the archives, records
the upstream repository, tag and commit that were built. Fork prereleases stay
under `vX.Y.Z-alpha.N` for `teams-cli-next`.

Because a mirror run deploys from the `next` branch rather than a tag, the
`release` environment's deployment policy allows `next` as well as the
prerelease tag patterns. The required reviewer still approves every signing
run, and `next` should accept changes only through reviewed pushes.

Anyone can check where an archive came from, and what source a mirrored
release was built from:

```bash
gh attestation verify teams-v0.8.0-aarch64-apple-darwin.tar.gz --repo aberoham/ms-teams-cli \
  --signer-workflow aberoham/ms-teams-cli/.github/workflows/release.yml
gh attestation verify teams-v0.8.0-source.txt --repo aberoham/ms-teams-cli \
  --signer-workflow aberoham/ms-teams-cli/.github/workflows/release.yml
cat teams-v0.8.0-source.txt
```
