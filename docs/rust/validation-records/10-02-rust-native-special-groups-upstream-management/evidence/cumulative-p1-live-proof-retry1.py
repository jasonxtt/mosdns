#!/usr/bin/env python3
"""Isolated real DNS/HTTP supplier provenance proof; no external DNS."""
import importlib.util,json,pathlib,shutil,subprocess,time,urllib.request
root=pathlib.Path('/root/mosdns-rust-special-groups-20261003')
out=root/'evidence/cumulative-p1-1-live-retry1'; out.mkdir(exist_ok=False)
fixture=out/'fixture'; shutil.copytree(root/'evidence/s7-browser-fixture',fixture)
(fixture/'cache').mkdir()
spec=importlib.util.spec_from_file_location('proof',root/'evidence/s7-live-proof.py'); module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
processes=[]; handles=[]
try:
 for port,answer,identity in [(25555,'192.0.2.90','default'),(25556,'192.0.2.50','lower'),(25557,'192.0.2.51','higher'),(25558,'192.0.2.52','isolated'),(25559,'192.0.2.55','lower_v2')]:
  f=(out/(identity+'.log')).open('w');handles.append(f)
  processes.append(subprocess.Popen(['python3',str(root/'evidence/s7-controlled-dns-peer.py'),'--transport','tcp' if identity=='higher' else 'udp','--port',str(port),'--answer',answer,'--peer-id',identity,'--log',str(out/'peer-queries.jsonl')],stdout=f,stderr=subprocess.STDOUT))
 f=(out/'native-host.log').open('w');handles.append(f)
 processes.append(subprocess.Popen([str(root/'target-candidate/debug/mosdns'),'start','-c',str(fixture/'config.yaml')],cwd=fixture,stdout=f,stderr=subprocess.STDOUT))
 for _ in range(100):
  try:
   module.request('GET','/api/v2/audit/logs?page=1&limit=100');break
  except OSError:time.sleep(.1)
 else:raise RuntimeError('API did not start')
 with (out/'live-proof.json').open('w') as f:subprocess.run(['python3',str(root/'evidence/s7-live-proof.py')],stdout=f,check=True)
 proof=json.loads((out/'live-proof.json').read_text());logs=proof['audit']['body']['logs'];checks=[]
 for entry,peer,minimum in [('lower_supplier','127.0.0.1:25556',1),('lower_supplier_v2','127.0.0.1:25559',1),('higher_supplier','127.0.0.1:25557',1)]:
  matches=[r for r in logs if r.get('final_upstream')==entry];misses=[r for r in matches if r['upstream_diagnostics']['attempts']];hits=[r for r in matches if not r['upstream_diagnostics']['attempts']]
  assert misses and len(hits)>=minimum,(entry,matches)
  for hit in hits:
   assert hit['selected_upstream']==peer
   assert hit['upstream_diagnostics']['selected']=={'entry':entry,'peer':peer,'transport':'tcp' if entry=='higher_supplier' else 'udp'}
  checks.append(dict(entry=entry,misses=len(misses),hits=len(hits),selected=hits[0]['upstream_diagnostics']['selected'],hit_attempts=[]))
 peers=[json.loads(line) for line in (out/'peer-queries.jsonl').read_text().splitlines()];assert len(peers)==6,len(peers)
 assert proof['save']['status']==200 and proof['unsupported_save']['status']==400
 (out/'assertions.json').write_text(json.dumps(dict(status='PASS',checks=checks,query_count=len(proof['queries']),actual_peer_requests=len(peers),no_external_dns=True),indent=2)+'\n')
 print(json.dumps(checks))
finally:
 for p in reversed(processes):
  p.terminate()
 for p in reversed(processes):
  try:p.wait(timeout=10)
  except subprocess.TimeoutExpired:p.kill();p.wait()
 for f in handles:f.close()
 (out/'processes.json').write_text(json.dumps([dict(pid=p.pid,returncode=p.returncode,args=p.args) for p in processes],indent=2)+'\n')
