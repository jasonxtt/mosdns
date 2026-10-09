import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from load import corpus, semantic, summarize, classify


class MeasurementTests(unittest.TestCase):
    def test_drain_uses_actual_deadline_and_checks_ttl(self):
        value = {'rcode':0,'questions':[], 'answers':[('x.',1,1,'192.0.2.1',300)],'authority':[],'additional':[]}
        expected = semantic(value)
        self.assertEqual(classify(value,expected,10.08,10,.5),'correct')
        self.assertEqual(classify(value,expected,10.51,10,.5),'late_correct')
        value['answers'][0]=('x.',1,1,'192.0.2.1',301)
        self.assertEqual(classify(value,expected,10.08,10,.5),'wrong')

    def test_weighted_corpus_retains_counts_and_query_types(self):
        rows = corpus('0000000090 0 hot.example.com|x\n0000000010 0 cold.example.com|x\n')
        for row, expected in zip(rows[:4], (63, 27, 7, 3)):
            self.assertAlmostEqual(row['weight'], expected)
        self.assertEqual([r['type'] for r in rows[:4]], [1, 28, 1, 28])
        self.assertAlmostEqual(sum(r['weight'] for r in rows[4:]) / sum(r['weight'] for r in rows), .02)

    def test_timeout_unsent_and_wrong_remain_in_denominator(self):
        rows = [dict(planned=0, sent=0.001, finished=.002, status='correct'),
                dict(planned=.01, sent=.011, finished=None, status='timeout'),
                dict(planned=.02, sent=None, finished=None, status='unsent'),
                dict(planned=.03, sent=.031, finished=.032, status='wrong')]
        value = summarize(rows, 1, .5)
        self.assertEqual(value['planned'], 4)
        self.assertEqual(value['sent'], 3)
        self.assertEqual(value['correct_on_time'], 1)
        self.assertEqual(value['correct_rate'], .25)
        self.assertEqual(value['planned_latency_ms']['p99'], 2)

    def test_semantic_preserves_addresses_and_authority_but_ignores_ttl(self):
        value = {'rcode': 0, 'questions': [('x.', 1, 1)],
                 'answers': [('x.', 1, 1, '192.0.2.1', 300)], 'authority': [], 'additional': []}
        other = dict(value, answers=[('x.', 1, 1, '192.0.2.1', 299)])
        self.assertEqual(semantic(value), semantic(other))
        other['answers'][0] = ('x.', 1, 1, '192.0.2.2', 299)
        self.assertNotEqual(semantic(value), semantic(other))

    @unittest.skipUnless(sys.platform=='linux', 'Linux /proc sampling')
    def test_open_loop_sends_while_replies_are_delayed(self):
        import argparse
        import http.server
        import json
        import os
        import socketserver
        import tempfile
        import threading
        import time
        from dnswire import answer, question, decode
        from load import measure
        class DNS(socketserver.ThreadingUDPServer):
            allow_reuse_address=True
            daemon_threads=True
        class Reply(socketserver.BaseRequestHandler):
            def handle(self):
                wire,sock=self.request
                time.sleep(.1)
                sock.sendto(answer(wire,'192.0.2.1','2001:db8::1'),self.client_address)
        class Metrics(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200); self.end_headers(); self.wfile.write(b'')
            def log_message(self,*args): pass
        # Port53 inside a disposable namespace; refuses an existing listener.
        with DNS(('127.0.0.1',53),Reply) as dns, http.server.HTTPServer(('127.0.0.1',0),Metrics) as http:
            threads=[threading.Thread(target=s.serve_forever,daemon=True) for s in (dns,http)]
            for t in threads:t.start()
            try:
                with tempfile.TemporaryDirectory() as name:
                    root=pathlib.Path(name); c=root/'corpus.json'; oracle=root/'oracle.json'
                    c.write_text(json.dumps([dict(name='x.example.com',type=1,weight=1)]))
                    oracle.write_text(json.dumps([dict(index=0,semantic=semantic(decode(answer(question('x.example.com',1,0),'192.0.2.1','2001:db8::1'))))]))
                    args=argparse.Namespace(output=root/'out',corpus=c,oracle=oracle,pid=os.getpid(),mock_pid=os.getpid(),
                                            seed=1,warmup=.1,rates=[200],duration=.2,recovery=.2,deadline=.05,api_port=http.server_port)
                    measure(args)
                    result=json.loads((args.output/'summary.json').read_text())['stages'][1]['all']
                    self.assertEqual(result['planned'],40)
                    self.assertGreaterEqual(result['sent'],36)
                    self.assertEqual(result['correct_on_time'],0)
                    self.assertGreater(result['statuses'].get('late_correct',0),0)
            finally:
                dns.shutdown(); http.shutdown()


if __name__ == '__main__':
    unittest.main()
