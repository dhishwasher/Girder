import copy
import json
from pathlib import Path
import tempfile
import unittest

from tools.measure_typescript_esm_imports import pins, score


class ScoringTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.source = b'// \xc3\xa9\n/* claim */ target();'
        (self.root / 'test.ts').write_bytes(self.source)
        (self.root / 'app.ts').write_text('export function target() {}')
        self.offset = self.source.index(b'target')
        self.case = {'importer': 'test.ts', 'expected_class': 'must', 'expected_target': {
            'file': 'app.ts', 'predicted_path': 'crate::app::target', 'marker': 'export function target('
        }}
        self.manifest = {'marker': '/* claim */', 'must_claim_assumptions': ['indexed-source-snapshot',
                         'esm-native-execution', 'no-unmodeled-module-hooks-loaders-or-mocks-outside-or-inside-snapshot']}
        self.inspection = self.root / 'inspect.json'

    def document(self, cls='must', targets='(1)', assumptions=None):
        if assumptions is None:
            assumptions = self.manifest['must_claim_assumptions']
        evidence = (f'(assumptions:{json.dumps(assumptions)},calls:['
                    f'(site:(start_byte:{self.offset},end_byte:{self.offset+8},start_row:1,start_col:12),'
                    f'class:{cls},targets:[{targets}],reason:"proven-typescript-relative-esm-named-import",'
                    'coverage_gap:false)])')
        return {'nodes': [
            {'id': '0000000000000002', 'path': 'crate::test', 'file': str(self.root / 'test.ts'),
             'kind': 'Module', 'attributes': [['call_evidence_v1', evidence]]},
            {'id': '0000000000000001', 'path': 'crate::app::target', 'file': str(self.root / 'app.ts'),
             'kind': 'Function', 'span': {'start_byte': 7, 'end_byte': 27}},
        ]}

    def evaluate(self, doc):
        self.inspection.write_text(json.dumps(doc))
        return score(self.case, self.manifest, self.root, self.inspection)['exact_contract']

    def test_exact_target_and_conditional_assumptions(self):
        self.assertTrue(self.evaluate(self.document()))

    def test_missing_assumption_refused(self):
        self.assertFalse(self.evaluate(self.document(assumptions=['indexed-source-snapshot'])))

    def test_wrong_or_duplicate_target_refused(self):
        for targets in ('(3)', '(1),(1)', ''):
            self.assertFalse(self.evaluate(self.document(targets=targets)))

    def test_wrong_target_file_span_or_kind_refused(self):
        for key, value in [('file', str(self.root / 'other.ts')), ('kind', 'Field'),
                           ('span', {'start_byte': 10, 'end_byte': 27})]:
            doc = self.document()
            doc['nodes'][1][key] = value
            self.assertFalse(self.evaluate(doc))

    def test_missing_duplicate_and_unparseable_evidence_refused(self):
        for mode in ('missing', 'duplicate', 'malformed'):
            doc = self.document()
            if mode == 'missing':
                doc['nodes'][0]['attributes'] = []
            elif mode == 'duplicate':
                doc['nodes'].append(copy.deepcopy(doc['nodes'][0]))
            else:
                doc['nodes'][0]['attributes'][0][1] = 'unparseable'
            self.assertFalse(self.evaluate(doc))

    def test_unknown_requires_empty_targets(self):
        self.case['expected_class'] = 'unknown'
        self.assertTrue(self.evaluate(self.document(cls='unknown', targets='')))
        self.assertFalse(self.evaluate(self.document(cls='unknown')))
        self.assertFalse(self.evaluate(self.document()))

    def test_claim_at_same_offset_in_different_file_is_not_evidence(self):
        doc = self.document()
        doc['nodes'][0]['file'] = str(self.root / 'other.ts')
        self.assertFalse(self.evaluate(doc))

    def test_pins_preserve_symlink_identity(self):
        (self.root / 'alias.ts').symlink_to('app.ts')
        (self.root / 'dir').symlink_to('missing-directory', target_is_directory=True)
        snapshot = pins(self.root)
        self.assertEqual(snapshot['alias.ts'], {'symlink': 'app.ts'})
        self.assertEqual(snapshot['dir'], {'symlink': 'missing-directory'})


if __name__ == '__main__':
    unittest.main()
