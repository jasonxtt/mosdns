from pathlib import Path
import socket,struct,urllib.request,json,sys,time
root=Path(__file__).parent
protocol=sys.argv[1]
def api(path,method='GET'):
    return urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:21980'+path,method=method)).read()
api('/api/v1/audit/start','POST')
if len(sys.argv)>2 and sys.argv[2]=='cold':api('/plugins/cache/flush')
results=[]
for index,peer in enumerate(['127.0.0.2','127.0.0.3','127.0.0.2']):
    query=struct.pack('>6H',200+index,0x100,1,0,0,0)+b'\x03ecs\x07browser\x07example\0'+struct.pack('>HH',1,1)
    sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM if protocol=='udp' else socket.SOCK_STREAM);sock.settimeout(3);sock.bind((peer,0))
    if protocol=='udp':sock.sendto(query,('127.0.0.1',21953));wire,_=sock.recvfrom(65535)
    else:
        sock.connect(('127.0.0.1',21953));sock.sendall(struct.pack('>H',len(query))+query)
        def read(n):
            result=b''
            while len(result)<n:
                chunk=sock.recv(n-len(result));assert chunk;result+=chunk
            return result
        wire=read(struct.unpack('>H',read(2))[0])
    sock.close()
    assert wire[:2]==query[:2] and wire[3]&15==0 and struct.unpack('>H',wire[10:12])[0]==0
    answer='.'.join(map(str,wire[-4:]));assert answer==f'192.0.2.{int(peer[-1])*10}',answer
    results.append({'protocol':protocol,'peer':peer,'answer':answer,'wire':wire.hex()})
logs=json.loads(api('/api/v2/audit/logs?limit=20'))
current=logs['logs'][:3]
for entry,expected in zip(current,reversed(results)):
    assert entry['client_ip']==expected['peer'] and entry['answers'][0]['data']==expected['answer'] and entry['answer_details_status']=='complete'
if len(sys.argv)>2 and sys.argv[2]=='cold':
    for entry in current[1:]:
        selected=entry['upstream_diagnostics']['selected']
        assert selected=={'branch_id':0,'entry':'controlled','peer':'127.0.0.1:21954','transport':'udp'}
        assert entry['upstream_diagnostics']['attempts'][0]['outcome']=='response'
show=api('/plugins/cache/show').decode()
assert '[ecs:127.0.0.2/32/0]' in show and '[ecs:127.0.0.3/32/0]' in show
api('/plugins/cache/save')
(root/f'{protocol}-oracle.json').write_text(json.dumps({'dns':results,'logs':logs,'cache_show':api('/plugins/cache/show').decode(),'metrics':api('/metrics').decode()},ensure_ascii=False,indent=2))
print(protocol,'DNS peer/ECS partition, cold/hit, suppressed generated echo and API capture PASS')
