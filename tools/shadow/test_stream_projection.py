import io
import json
import unittest

from stream_projection import project_json
from unattended import trace_projection


class Chunked(io.StringIO):
    def read(self, size=-1):
        if size < 0 or size > 4096:
            raise AssertionError('must not materialize a whole API response')
        return super().read(min(size, 31))


class StreamProjectionTests(unittest.TestCase):
    def test_large_response_projects_one_object_at_a_time_without_losing_false_or_unicode(self):
        entries = [{'trace_id': str(i), 'match_result': False, 'text': '中文', 'unused': 'x'*5000} for i in range(200)]
        source = json.dumps({'total':200,'logs':entries,'trailing':12345},ensure_ascii=False)
        result = project_json(Chunked(source), lambda x: {k:x[k] for k in ('trace_id','match_result','text')}, member='logs')
        self.assertEqual(len(result),200)
        self.assertEqual(result[-1],{'trace_id':'199','match_result':False,'text':'中文'})
        self.assertNotIn('unused',result[0])

    def test_array_and_truncated_json_are_distinguished(self):
        self.assertEqual(project_json(Chunked('[{"n":123456}, {"n":null}]'),lambda x:x),[{'n':123456},{'n':None}])
        with self.assertRaises(ValueError):
            project_json(Chunked('[{"n":123'),lambda x:x)

    def test_trace_batch_shares_repeated_strings_without_changing_evidence(self):
        project=trace_projection()
        text='n-'+'x'*64
        first=json.loads(json.dumps({'trace_id':text,'logger_name':'entry','msg':'dns query completed','match_result':False}))
        second=json.loads(json.dumps(first))
        self.assertIsNot(first['trace_id'],second['trace_id'])
        a,b=project(first),project(second)
        self.assertEqual(a,first)
        self.assertEqual(b,second)
        self.assertIs(a['trace_id'],b['trace_id'])
        self.assertIs(a['msg'],b['msg'])


if __name__=='__main__': unittest.main()
