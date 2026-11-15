#!/usr/bin/env python3
"""Release-request rejection cases; no network or publication."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("recipe_release", Path(__file__).with_name("recipe-release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseRequestTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for name, version in (("lmm-api", "0.1.23"), ("lmm-core", "0.1.18")):
            path = self.root / "recipes" / name / "RECIPE.md"
            path.parent.mkdir(parents=True)
            path.write_text(f'# {name}\n\n```toml\n[package]\nname = "{name}"\nversion = "{version}"\n```\n')
        self.request = {"version": "0.1.23", "core": "0.1.18", "source_version": "0.1.20", "source_revision": "a" * 40}

    def test_recipe_version_and_gitlink_must_match_the_request(self):
        release.validate_request(self.root, self.request, "a" * 40)
        for change in ({"version": "0.1.22"}, {"core": "0.1.17"}, {"source_revision": "b" * 40}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                release.validate_request(self.root, {**self.request, **change}, "a" * 40)

    def test_release_identity_cannot_be_a_branch_or_command(self):
        for change in ({"source_revision": "main"}, {"version": "0.1.23; echo invalid"}, {"source_version": "latest"}, {"extra": "ignored"}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                release.validate_request(self.root, {**self.request, **change}, "a" * 40)

    def test_published_annotated_source_tag_resolves_to_the_gitlink(self):
        values = [
            {"draft": False, "published_at": "2026-11-15T00:00:00Z", "tag_name": "v0.1.20", "html_url": "https://github.com/TokenNotIncluded/lmm-api-source/releases/tag/v0.1.20"},
            {"object": {"type": "tag", "sha": "b" * 40}},
            {"object": {"type": "commit", "sha": "a" * 40}},
        ]
        with patch.object(release, "github", side_effect=values) as api:
            self.assertEqual(release.published_source(self.request), values[0]["html_url"])
            self.assertEqual(api.call_args.args[1], "git/tags/" + "b" * 40)

    def test_unpublished_source_and_mismatching_tags_are_rejected(self):
        published = {"draft": False, "published_at": "2026-11-15T00:00:00Z", "tag_name": "v0.1.20", "html_url": "https://example.invalid/release"}
        for change in ({"draft": True}, {"published_at": None}, {"tag_name": "v0.1.19"}):
            with self.subTest(change=change), patch.object(release, "github", return_value={**published, **change}), self.assertRaises(ValueError):
                release.published_source(self.request)
        with patch.object(release, "github", side_effect=[published, {"object": {"type": "commit", "sha": "b" * 40}}]), self.assertRaises(ValueError):
            release.published_source(self.request)


if __name__ == "__main__":
    unittest.main()
