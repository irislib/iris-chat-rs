import copy
import unittest
from reuse_ios_core_cache import cache_proof


class CoreCacheProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.jobs = [{'name': 'iOS simulator build and composer tests', 'steps': [
            {'name': 'iOS simulator build', 'conclusion': 'success'},
            {'name': 'Save successful iOS compiler cache', 'conclusion': 'success',
             'started_at': '2026-10-19T20:44:09Z', 'completed_at': '2026-10-19T20:46:24Z'}]}]
        self.caches = [{'id': 1, 'key': 'exact-inputs', 'ref': 'refs/heads/test',
                        'created_at': '2026-10-19T20:46:23Z'}]

    def proof(self, jobs=None, caches=None, key='exact-inputs', ref='refs/heads/test'):
        return cache_proof(jobs or self.jobs, caches or self.caches, key, ref)

    def test_unique_successful_build_cache_is_accepted(self):
        self.assertEqual(1, self.proof()['id'])

    def test_other_input_cache_is_rejected(self):
        with self.assertRaises(AssertionError): self.proof(key='other-inputs')

    def test_other_branch_cache_is_rejected(self):
        with self.assertRaises(AssertionError): self.proof(ref='refs/heads/other')

    def test_duplicate_provenance_is_rejected(self):
        with self.assertRaises(AssertionError): self.proof(caches=self.caches*2)

    def test_cache_older_than_the_producer_is_rejected(self):
        stale = [dict(self.caches[0], created_at='2026-10-18T20:46:23Z')]
        with self.assertRaises(AssertionError): self.proof(caches=stale)

    def test_skipped_cache_save_is_not_a_source_receipt(self):
        jobs = copy.deepcopy(self.jobs); jobs[0]['steps'][1]['conclusion'] = 'skipped'
        with self.assertRaises(AssertionError): self.proof(jobs=jobs)

    def test_failed_build_is_rejected(self):
        jobs = copy.deepcopy(self.jobs); jobs[0]['steps'][0]['conclusion'] = 'failure'
        with self.assertRaises(AssertionError): self.proof(jobs=jobs)


if __name__ == '__main__': unittest.main()
