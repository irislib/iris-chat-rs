#!/usr/bin/env python3

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
HASHTREE_NPUB = "npub1399g0q2gtwjcglyjcg3jw3rcllqhm375pwases5hkvqa56aqe5wsz2eaap"
ZAPSTORE_NPUB = "npub1wyvg2agqh7sq0y6pga3rayr45uhr0fg5ucz4yjg36rmv4t8yrvrsslkwpm"


class ReleaseIdentityTests(unittest.TestCase):
    def test_distribution_uses_separate_fixed_publishers(self) -> None:
        common = (ROOT / "scripts" / "distribution_common.sh").read_text()
        distributor = (ROOT / "scripts" / "distribute").read_text()
        self.assertIn(HASHTREE_NPUB, common)
        self.assertIn(ZAPSTORE_NPUB, common)
        self.assertIn("IRIS_HASHTREE_NSEC_PATH", common)
        self.assertIn("IRIS_HASHTREE_CONFIG_DIR", common)
        self.assertIn("IRIS_ZAPSTORE_NSEC_PATH", common)
        self.assertIn("require_hashtree_identity", distributor)
        self.assertIn("require_zapstore_identity", distributor)
        self.assertNotIn("IRIS_RELEASE_NOSTR_KEY_PATH", common + distributor)

    def test_zapstore_config_names_the_dedicated_publisher(self) -> None:
        config = (ROOT / "zapstore.yaml").read_text()
        self.assertIn(f"pubkey: {ZAPSTORE_NPUB}", config)
        self.assertNotIn("IrisChat-release-latest.apk", config)


if __name__ == "__main__":
    unittest.main()
