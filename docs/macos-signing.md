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
the default branch, or by hand with an upstream tag. It picks
osodevops/ms-teams-cli's latest stable release, skips it if
`upstream-vX.Y.Z` is already published, fast-forwards `main` from upstream so
the tagged commit exists here, and calls `release.yml` with that commit. The
build, CI, signing, notarization and attestation are the same jobs a tag
release runs, applied to upstream's unmodified source; only the workflow and
signing script come from `next`. The release is published as
`upstream-vX.Y.Z`, with archives named `teams-vX.Y.Z-<target>`, and is what
the tap's `teams-cli` formula installs. Fork prereleases stay under
`vX.Y.Z-alpha.N` for `teams-cli-next`.

Because a mirror run deploys from the `next` branch rather than a tag, the
`release` environment's deployment policy allows `next` as well as the
prerelease tag patterns. The required reviewer still approves every signing
run.

Anyone can check where an archive came from:

```bash
gh attestation verify teams-v0.8.0-aarch64-apple-darwin.tar.gz --repo aberoham/ms-teams-cli
```
