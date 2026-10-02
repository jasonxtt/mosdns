import socket,struct,time,json,pathlib,urllib.request,os,signal,subprocess
root=pathlib.Path('/root/mosdns-rust-cache-lifecycle-20261001/proof'); base=root.parent
def api(path):
    with urllib.request.urlopen('http://127.0.0.1:20880'+path) as response:return response.read()
def query(identifier):
    q=struct.pack('>6H',identifier,0x0100,1,0,0,0)+b'\x04Case\x07example\0'+struct.pack('>HH',1,1)
    s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);s.settimeout(2);s.sendto(q,('127.0.0.1',20553));wire,_=s.recvfrom(65535);s.close()
    assert wire[:2]==q[:2] and wire[3]&15==0
    return {'id':identifier,'ttl':struct.unpack('>I',wire[-10:-6])[0],'answer':'.'.join(map(str,wire[-4:]))}
urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:20880/api/v1/audit/start', method='POST')).close()
events=[query(1),query(2)];assert events[0]['ttl']==2 and events[1]['answer']=='192.0.2.1'
time.sleep(2.1)
events.extend([query(3),query(4)]);assert events[2]['ttl']==5 and events[3]['ttl']==5
(root/'release-refresh').touch()
for _ in range(200):
    if (root/'upstream-count.json').exists() and json.loads((root/'upstream-count.json').read_text())['count']==2:break
    time.sleep(.01)
events.append(query(5));assert events[-1]['answer']=='192.0.2.2' and events[-1]['ttl']>=299
metrics=api('/metrics').decode();assert 'mosdns_cache_query_total{tag="alpha"} 5' in metrics and 'mosdns_cache_lazy_hit_total{tag="alpha"} 2' in metrics and 'mosdns_cache_query_total{tag="beta"} 1' in metrics
show=api('/plugins/alpha/show').decode();assert 'DomainSet:     group' in show and '192.0.2.2' in show
api('/plugins/alpha/save');assert (root/'alpha.gz').exists()
(root/'lifecycle.json').write_text(json.dumps({'events':events,'metrics':metrics,'show':show,'audit':json.loads(api('/api/v2/audit/logs?limit=20'))},ensure_ascii=False,indent=2))
owned=json.loads((root/'native.owned.json').read_text());pid=owned['pid'];assert pathlib.Path(f'/proc/{pid}/stat').read_text().split()[21]==owned['start'];os.kill(pid,signal.SIGTERM)
for _ in range(200):
    try:
        if pathlib.Path(f'/proc/{pid}/stat').read_text().split()[2]=='Z':break
    except FileNotFoundError:break
    time.sleep(.01)
else:raise RuntimeError('owner did not stop')
with (root/'native.log').open('ab') as log:
    process=subprocess.Popen([str(base/'rust/target/debug/mosdns'),'start','-c','config.yaml'],cwd=root,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
(root/'native.owned.json').write_text(json.dumps({'pid':process.pid,'start':pathlib.Path(f'/proc/{process.pid}/stat').read_text().split()[21]}))
for _ in range(200):
    try:api('/api/v1/cache/inventory');break
    except OSError:time.sleep(.01)
result=query(6);assert result['answer']=='192.0.2.2';assert json.loads((root/'upstream-count.json').read_text())['count']==2
show=api('/plugins/alpha/show').decode();assert 'DomainSet:     group' in show
(root/'restart.json').write_text(json.dumps({'hit':result,'upstream_count':2,'show':show,'metrics':api('/metrics').decode()},indent=2))
print('Real miss/fresh/lazy/follower/background/save/SIGTERM/restart/domain_set/metrics PASS')
