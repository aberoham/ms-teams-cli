"""Tests of mirror-find.sh's choice of upstream release, against a fake gh."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("mirror-find.sh")
FORK = "aberoham/ms-teams-cli"
UPSTREAM = "osodevops/ms-teams-cli"
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "aarch64-unknown-linux-musl",
           "x86_64-unknown-linux-musl"]

# Answers gh from the routes in $FAKE_GH_ROUTES, applying --jq as gh does.
# An api route that is not listed answers 404.
FAKE_GH = r'''
import json, os, subprocess, sys
args = sys.argv[1:]
jq = None
if "--jq" in args:
    i = args.index("--jq")
    jq = args[i + 1]
    del args[i:i + 2]
if args[0] == "api":
    key = "api " + args[1]
elif args[:2] == ["release", "list"]:
    key = "release list " + args[args.index("--repo") + 1]
elif args[:2] == ["release", "view"]:
    key = "release view " + args[args.index("--repo") + 1] + " " + args[2]
else:
    sys.exit("fake gh: unexpected " + " ".join(sys.argv[1:]))
routes = json.load(open(os.environ["FAKE_GH_ROUTES"]))
if key not in routes:
    print("gh: Not Found (HTTP 404)", file=sys.stderr)
    sys.exit(1)
body = json.dumps(routes[key])
if jq is not None:
    body = subprocess.run(["jq", "-r", jq], input=body, capture_output=True, text=True, check=True).stdout
sys.stdout.write(body if body.endswith("\n") else body + "\n")
'''


def complete_assets(tag):
    names = [f"teams-{tag}-{t}.tar.gz" for t in TARGETS]
    names += [f"teams-{tag}-x86_64-pc-windows-msvc.zip", f"teams-{tag}-source.txt", "checksums-sha256.txt"]
    return [{"name": n, "state": "uploaded", "size": 100} for n in names]


class MirrorFindTests(unittest.TestCase):
    def find(self, upstream, mirrored, requested="", drafts=(), incomplete=()):
        """Runs the script with upstream's stable releases, newest first as gh
        lists them, and the fork's complete mirrors. Returns its exit code,
        outputs and stderr."""
        routes = {
            f"release list {UPSTREAM}": [{"tagName": t, "publishedAt": f"2026-09-{30 - i:02}T00:00:00Z"}
                                         for i, t in enumerate(upstream)],
            f"release list {FORK}": [{"tagName": f"upstream-{t}"} for t in [*mirrored, *drafts, *incomplete]],
        }
        for tag in mirrored:
            routes[f"api repos/{FORK}/releases/tags/upstream-{tag}"] = {"draft": False, "assets": complete_assets(tag)}
        for tag in incomplete:
            routes[f"api repos/{FORK}/releases/tags/upstream-{tag}"] = {"draft": False, "assets": complete_assets(tag)[1:]}
        for tag in upstream:
            sha = f"sha-{tag}"
            routes[f"release view {UPSTREAM} {tag}"] = {"isDraft": False, "isPrerelease": False}
            routes[f"api repos/{UPSTREAM}/commits/{tag}"] = {"sha": sha}
            routes[f"api repos/{UPSTREAM}/compare/{sha}...main"] = {"status": "ahead"}
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            (directory / "routes.json").write_text(json.dumps(routes))
            gh = directory / "gh"
            gh.write_text(f"#!{sys.executable}\n{FAKE_GH}")
            gh.chmod(0o755)
            output = directory / "output"
            output.touch()
            env = {**os.environ, "PATH": f"{directory}{os.pathsep}{os.environ['PATH']}",
                   "FAKE_GH_ROUTES": str(directory / "routes.json"), "GITHUB_OUTPUT": str(output),
                   "GITHUB_REPOSITORY": FORK, "UPSTREAM": UPSTREAM, "REQUESTED_TAG": requested}
            result = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True)
            outputs = dict(line.split("=", 1) for line in output.read_text().splitlines())
        return result.returncode, outputs, result.stderr

    def assert_mirrors(self, expected, *args, **kwargs):
        code, outputs, stderr = self.find(*args, **kwargs)
        self.assertEqual(code, 0, stderr)
        self.assertEqual(outputs, {"build": "true", "source_ref": f"sha-{expected}",
                                   "release_tag": f"upstream-{expected}", "version_tag": expected})

    def assert_refuses(self, message, *args, **kwargs):
        code, outputs, stderr = self.find(*args, **kwargs)
        self.assertNotEqual(code, 0)
        self.assertNotIn("build", outputs)
        self.assertIn(message, stderr)

    # Upstream's history when mirroring began at v0.8.0: v0.1.0 was chosen
    # on every scheduled run, failed to package, and v0.9.0 never shipped.
    UPSTREAM_HISTORY = ["v0.9.0", "v0.8.0", "v0.7.0", "v0.6.0", "v0.5.0", "v0.4.0", "v0.3.0",
                        "v0.2.8", "v0.2.7", "v0.2.6", "v0.2.5", "v0.2.4", "v0.2.3", "v0.2.2",
                        "v0.2.1", "v0.1.0"]

    def test_releases_from_before_mirroring_began_are_not_chosen(self):
        self.assert_mirrors("v0.9.0", self.UPSTREAM_HISTORY, ["v0.8.0"])

    def test_two_new_releases_are_mirrored_oldest_first_by_version(self):
        self.assert_mirrors("v0.9.0", ["v0.10.0", "v0.9.0", "v0.8.0", "v0.1.0"], ["v0.8.0"])
        self.assert_mirrors("v0.10.0", ["v0.10.0", "v0.9.0", "v0.8.0", "v0.1.0"], ["v0.8.0", "v0.9.0"])

    def test_nothing_new_builds_nothing(self):
        code, outputs, stderr = self.find(self.UPSTREAM_HISTORY, ["v0.8.0", "v0.9.0"])
        self.assertEqual(code, 0, stderr)
        self.assertEqual(outputs, {"build": "false"})

    def test_a_gap_below_the_newest_mirror_is_left_for_a_run_by_hand(self):
        code, outputs, stderr = self.find(self.UPSTREAM_HISTORY, ["v0.7.0", "v0.9.0"])
        self.assertEqual(code, 0, stderr)
        self.assertEqual(outputs, {"build": "false"})
        self.assert_mirrors("v0.8.0", self.UPSTREAM_HISTORY, ["v0.7.0", "v0.9.0"], requested="v0.8.0")

    def test_without_any_mirror_a_schedule_refuses_to_guess(self):
        self.assert_refuses("run by hand", self.UPSTREAM_HISTORY, [])
        self.assert_mirrors("v0.1.0", self.UPSTREAM_HISTORY, [], requested="v0.1.0")

    def test_an_unfinished_newest_mirror_stops_the_run(self):
        self.assert_refuses("is a draft or could not be found", self.UPSTREAM_HISTORY, ["v0.8.0"], drafts=["v0.9.0"])
        self.assert_refuses("lacks complete assets", self.UPSTREAM_HISTORY, ["v0.8.0"], incomplete=["v0.9.0"])

    def test_a_requested_tag_already_mirrored_is_refused(self):
        self.assert_refuses("already mirrored", self.UPSTREAM_HISTORY, ["v0.8.0"], requested="v0.8.0")

    def test_versions_compare_numerically(self):
        self.assert_mirrors("v0.10.0", ["v0.10.0", "v0.9.0"], ["v0.9.0"])
        code, outputs, stderr = self.find(["v0.10.0", "v0.9.0"], ["v0.10.0", "v0.9.0"])
        self.assertEqual(outputs, {"build": "false"}, stderr)


if __name__ == "__main__":
    unittest.main()
