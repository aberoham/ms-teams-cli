#!/usr/bin/env bash
# Chooses the upstream release that mirror-upstream.yml builds, and writes
# build, source_ref, release_tag and version_tag to $GITHUB_OUTPUT.
#
# Reads UPSTREAM, GITHUB_REPOSITORY, GITHUB_OUTPUT and, for a run by hand,
# REQUESTED_TAG. test_mirror_find.py runs it against a fake gh.
set -euo pipefail

stable='^v([0-9]+)\.([0-9]+)\.([0-9]+)$'

# Succeeds when vX.Y.Z $1 is a later version than vX.Y.Z $2.
newer() {
  local a b i
  [[ "$1" =~ $stable ]] || return 1
  a=("${BASH_REMATCH[@]:1}")
  [[ "$2" =~ $stable ]] || return 1
  b=("${BASH_REMATCH[@]:1}")
  for i in 0 1 2; do
    if ((10#${a[i]} > 10#${b[i]})); then return 0; fi
    if ((10#${a[i]} < 10#${b[i]})); then return 1; fi
  done
  return 1
}

# Prints "missing" or "published" for this repository's mirror of an
# upstream tag. Anything else, including a lookup that fails for a reason
# other than 404, a draft, a release without all its assets, or a tag
# without a release, stops the run for a person to look at rather than
# being skipped or overwritten.
mirror_state() {
  local tag="$1" release="upstream-$1" out
  if ! out="$(gh api "repos/${GITHUB_REPOSITORY}/releases/tags/${release}" 2>&1)"; then
    if ! grep -q 'HTTP 404' <<<"$out"; then
      echo "Could not look up ${release}: ${out}" >&2
      return 1
    fi
    if out="$(gh api "repos/${GITHUB_REPOSITORY}/git/ref/tags/${release}" 2>&1)"; then
      echo "Tag ${release} exists without a release; delete it and run again" >&2
      return 1
    elif ! grep -q 'HTTP 404' <<<"$out"; then
      echo "Could not look up tag ${release}: ${out}" >&2
      return 1
    fi
    echo missing
    return 0
  fi
  # Five archives, the source manifest and the checksums, each fully
  # uploaded and not empty.
  local expected missing
  expected="$(printf '%s\n' \
    "teams-${tag}-aarch64-apple-darwin.tar.gz" \
    "teams-${tag}-x86_64-apple-darwin.tar.gz" \
    "teams-${tag}-aarch64-unknown-linux-musl.tar.gz" \
    "teams-${tag}-x86_64-unknown-linux-musl.tar.gz" \
    "teams-${tag}-x86_64-pc-windows-msvc.zip" \
    "teams-${tag}-source.txt" \
    checksums-sha256.txt)"
  missing="$(jq -r --arg expected "$expected" '
    ($expected | split("\n")) - [.assets[] | select(.state == "uploaded" and .size > 0) | .name]
    | join(" ")' <<<"$out")"
  if [[ "$(jq -r .draft <<<"$out")" != false || -n "$missing" ]]; then
    echo "${release} is a draft or lacks complete assets (${missing:-none missing}); finish or delete it" >&2
    return 1
  fi
  echo published
}

if [[ -n "${REQUESTED_TAG:-}" ]]; then
  candidates="$REQUESTED_TAG"
else
  # Upstream releases newer than the newest mirror, oldest first. Drafts
  # count, so an unfinished mirror stops the run. Assignments make a
  # failed lookup stop it too.
  mirrors="$(gh release list --repo "$GITHUB_REPOSITORY" --limit 1000 --json tagName --jq '.[].tagName')"
  floor=""
  while IFS= read -r release; do
    version="${release#upstream-}"
    if [[ "$release" == upstream-* ]] && [[ "$version" =~ $stable ]]; then
      if [[ -z "$floor" ]] || newer "$version" "$floor"; then
        floor="$version"
      fi
    fi
  done <<<"$mirrors"
  if [[ -z "$floor" ]]; then
    echo "Nothing is mirrored yet; run by hand with a tag" >&2
    exit 1
  fi
  state="$(mirror_state "$floor")"
  if [[ "$state" != published ]]; then
    echo "upstream-${floor} is a draft or could not be found" >&2
    exit 1
  fi

  listed="$(gh release list --repo "$UPSTREAM" \
    --exclude-drafts --exclude-pre-releases --limit 100 \
    --json tagName --jq '.[].tagName')"
  newer_tags=""
  while IFS= read -r candidate; do
    if newer "$candidate" "$floor"; then
      newer_tags+="${candidate#v}"$'\n'
    fi
  done <<<"$listed"
  candidates="$(printf '%s' "$newer_tags" | sort -t. -k1,1n -k2,2n -k3,3n | sed 's/^/v/')"
fi

tag=""
while IFS= read -r candidate; do
  [[ "$candidate" =~ $stable ]] || continue
  state="$(mirror_state "$candidate")"
  if [[ "$state" == missing ]]; then
    tag="$candidate"
    break
  fi
done <<<"$candidates"
if [[ -n "${REQUESTED_TAG:-}" && -z "$tag" ]]; then
  echo "Upstream ${REQUESTED_TAG} is not a stable tag, or is already mirrored" >&2
  exit 1
fi
if [[ -z "$tag" ]]; then
  echo "Nothing newer than upstream-${floor}"
  echo "build=false" >>"$GITHUB_OUTPUT"
  exit 0
fi

state="$(gh release view "$tag" --repo "$UPSTREAM" --json isDraft,isPrerelease --jq '"\(.isDraft) \(.isPrerelease)"')"
if [[ "$state" != "false false" ]]; then
  echo "Upstream ${tag} is a draft or a prerelease (${state}); not mirroring it" >&2
  exit 1
fi
sha="$(gh api "repos/${UPSTREAM}/commits/${tag}" --jq .sha)"
# Upstream releases from its main line; a tag made elsewhere is left for a
# person to judge.
position="$(gh api "repos/${UPSTREAM}/compare/${sha}...main" --jq .status)"
if [[ "$position" != ahead && "$position" != identical ]]; then
  echo "Upstream ${tag} (${sha}) is not on upstream's main; mirror it by hand if intended" >&2
  exit 1
fi

{
  echo "build=true"
  echo "source_ref=${sha}"
  echo "release_tag=upstream-${tag}"
  echo "version_tag=${tag}"
} >>"$GITHUB_OUTPUT"
echo "Mirroring upstream ${tag} (${sha}) as upstream-${tag}"
