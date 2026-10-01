import unittest
from tools.dispatch_audit_reconciliation import acceptance, combine


class AuditReconciliationTests(unittest.TestCase):
    def test_105_entries_with_only_52_call_sites_cannot_pass(self):
        rows = [{'status': 'scored', 'cell': 'exact', 'true_class': 'must', 'observed_class': 'must'} for _ in range(52)]
        rows += [{'status': 'not_a_call_site'} for _ in range(53)]
        self.assertFalse(acceptance({'results': rows})['passed'])

    def test_empty_must_remains_undefined(self):
        rows = [{'status': 'scored', 'cell': 'exact', 'true_class': 'unknown', 'observed_class': 'unknown'} for _ in range(100)]
        gate = acceptance({'results': rows})
        self.assertIsNone(gate['must_precision'])
        self.assertFalse(gate['passed'])

    def test_unresolved_relocation_or_unsound_cell_fails(self):
        rows = [{'status': 'scored', 'cell': 'exact', 'true_class': 'must', 'observed_class': 'must'} for _ in range(100)]
        self.assertTrue(acceptance({'results': rows})['passed'])
        self.assertFalse(acceptance({'results': rows + [{'status': 'site_relocation_failed'}]})['passed'])
        rows[0] = {'status': 'scored', 'cell': 'overclaim', 'true_class': 'unknown', 'observed_class': 'must'}
        self.assertFalse(acceptance({'results': rows})['passed'])

    def test_reserve_cannot_be_skipped_or_original_changed(self):
        original = [{'crate': 'c', 'file': 'f', 'line': 1, 'true_class': 'must'}]
        reserve = [{'crate': 'c', 'file': 'f', 'line': 2}, {'crate': 'c', 'file': 'f', 'line': 3}]
        label = dict(reserve[0], true_class='unknown', rationale='opaque', confidence='high')
        self.assertEqual(combine(original, reserve, [label], 2), original + [label])
        with self.assertRaises(ValueError):
            combine(original, reserve, [dict(label, line=3)], 2)
        with self.assertRaises(ValueError):
            combine(original, reserve, [label, dict(label, line=3)], 2)
