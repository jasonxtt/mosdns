"""Run remotely against this task's owned real process. Assertions use decoded wire/API."""
import socket,struct,ipaddress,json,pathlib,urllib.request,subprocess,time,hashlib
ROOT=pathlib.Path(__file__).resolve().parent
proof={};serial=0
def api(path,method='GET'):
    with urllib.request.urlopen(urllib.request.Request('http://127.0.0.1:21880'+path,method=method),timeout=3) as r:
        b=r.read()
        try:return json.loads(b)
        except ValueError:return b.decode()
def domain(s):return b''.join(bytes([len(x)])+x.encode() for x in s.split('.'))+b'\0'
def readname(b,p,seen=None):
    seen=set() if seen is None else seen;labels=[]
    while True:
        assert p<len(b) and p not in seen;seen.add(p);n=b[p];p+=1
        if n==0:return '.'.join(labels)+'.',p
        if n&192==192:
            target=((n&63)<<8)|b[p];suffix,_=readname(b,target,seen);return '.'.join(labels)+('.' if labels else '')+suffix,p+1
        assert n<=63 and p+n<=len(b);labels.append(b[p:p+n].decode());p+=n

def take(s,n):
    out=b''
    while len(out)<n:
        x=s.recv(n-len(out));assert x;out+=x
    return out

def query(name,kind=1,tcp=False):
    global serial
    serial+=1;q=struct.pack('>6H',serial,0x100,1,0,0,0)+domain(name)+struct.pack('>2H',kind,1)
    if tcp:
        with socket.create_connection(('127.0.0.1',21556),3) as s:
            s.sendall(struct.pack('>H',len(q))+q);b=take(s,struct.unpack('>H',take(s,2))[0])
    else:
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as s:
            s.settimeout(3);s.sendto(q,('127.0.0.1',21553));b=s.recv(65535)
    ident,flags,qd,an,ns,ar=struct.unpack('>6H',b[:12]);assert ident==serial and qd==1
    question,p=readname(b,12);qt,qc=struct.unpack('>2H',b[p:p+4]);p+=4
    assert question==name+'.' and qt==kind and qc==1 and flags&0x8000
    sections=[]
    for count in [an,ns,ar]:
        records=[]
        for _ in range(count):
            owner,p=readname(b,p);typ,cls,ttl,n=struct.unpack('>HHIH',b[p:p+10]);p+=10;end=p+n;assert end<=len(b)
            if typ in [1,28]:data=str(ipaddress.ip_address(b[p:end]))
            elif typ==5:data,_=readname(b,p)
            elif typ==6:
                m,a=readname(b,p);r,a=readname(b,a);data=[m,r,*struct.unpack('>5I',b[a:end])]
            else:data=b[p:end].hex()
            records.append({'owner':owner,'type':typ,'class':cls,'ttl':ttl,'data':data});p=end
        sections.append(records)
    assert p==len(b)
    return {'question':question,'qtype':qt,'flags':flags,'rcode':flags&15,'answer':sections[0],'authority':sections[1],'additional':sections[2],'wire_hex':b.hex()}
def run(command,config='config.yaml'):
    subprocess.run(['python3',str(ROOT/'services.py'),command,config],check=True,cwd=ROOT)
def peers():return [json.loads(x) for x in (ROOT/'peer-questions.jsonl').read_text().splitlines()]
def checkpoint(label,value):proof[label]=value;(ROOT/'evidence.json').write_text(json.dumps(proof,indent=2))

api('/plugins/stored/flush');api('/api/v1/audit/start','POST')
h=query('host.example');assert [(x['data'],x['ttl']) for x in h['answer']]==[('192.0.2.1',10)];checkpoint('hosts_a',h)
h6=query('host.example',28);assert [(x['data'],x['ttl']) for x in h6['answer']]==[('2001:db8::1',10)];checkpoint('hosts_aaaa',h6)
e=query('empty-family.example',28);assert not e['answer'] and e['authority'][0]['ttl']==300 and e['authority'][0]['data']==['fake-ns.mosdns.fake.root.','fake-mbox.mosdns.fake.root.',2021110400,1800,900,604800,86400];checkpoint('empty_family',e)
r=query('original.example');assert [(x['owner'],x['type'],x['ttl']) for x in r['answer']]==[('original.example.',5,1),('target.example.',5,30),('final.example.',1,30)];assert r['answer'][0]['data']=='target.example.';checkpoint('redirect_udp',r)
count=len(peers());cached=query('original.example');assert len(peers())==count;checkpoint('warm_cache',cached)
n=query('nx.original.example');assert n['rcode']==3 and n['answer'][0]['data']=='nx.target.example.' and n['authority'][0]['data']==['ns.example.','mb.example.',1,2,3,4,5] and n['authority'][0]['ttl']==30;checkpoint('negative_redirect',n)
f=query('fallback.example');assert f['answer'][-1]['data']=='198.51.100.42';checkpoint('ip_fallback_udp',f)
f6=query('fallback6.example',28);assert f6['answer'][-1]['data']=='2001:db9::42';checkpoint('ip_fallback_ipv6_udp',f6)
audit=api('/api/v2/audit/logs?limit=50');checkpoint('audit_udp',audit)
for label in ('fallback.example','fallback6.example'):
    record=next(x for x in audit['logs'] if x['query_name']==label);assert record['selected_upstream']=='127.0.0.1:21555' and len(record['upstream_diagnostics']['attempts'])==2
record=next(x for x in audit['logs'] if x['query_name']=='host.example' and x['query_type']=='A');assert 'selected_upstream' not in record and not record['upstream_diagnostics']['attempts']
record=next(x for x in audit['logs'] if x['query_name']=='original.example');assert 'selected_upstream' not in record and not record['upstream_diagnostics']['attempts']
checkpoint('inventory_before_restart',api('/api/v1/cache/inventory'))
# Immutable loader stays on old policy even after disk replacement, with cache flushed first.
(ROOT/'hosts.txt').write_text('host.example 192.0.2.99 2001:db8::99\nempty-family.example 192.0.2.2\n')
api('/plugins/stored/flush');old=query('host.example');assert old['answer'][0]['data']=='192.0.2.1';checkpoint('immutable_live_owner',old)
api('/plugins/stored/save');before=json.loads((ROOT/'native.owned.json').read_text());run('restart');after=json.loads((ROOT/'native.owned.json').read_text());assert before['pid']!=after['pid'];retained=query('host.example');assert retained['answer'][0]['data']=='192.0.2.1';checkpoint('retained_dump_real_restart',{'before':before,'after':after,'response':retained})
# No DNS query producers between durable Flush and SIGTERM/final-save/restart.
api('/plugins/stored/flush');checkpoint('inventory_after_flush',api('/api/v1/cache/inventory'));run('stop');run('restart');new=query('host.example');assert new['answer'][0]['data']=='192.0.2.99';checkpoint('flush_sigterm_restart',new)
api('/plugins/stored/flush');run('restart','config-tcp.yaml');api('/api/v1/audit/start','POST');t=query('original.example',tcp=True);assert [(x['type'],x['ttl']) for x in t['answer']]==[(5,1),(5,30),(1,30)];checkpoint('redirect_tcp',t)
tf=query('fallback.example',tcp=True);assert tf['answer'][-1]['data']=='198.51.100.42';checkpoint('ip_fallback_tcp',tf);checkpoint('audit_tcp',api('/api/v2/audit/logs?limit=50'))
run('restart');api('/plugins/stored/flush');api('/api/v1/audit/start','POST')
query('host.example');query('original.example');query('fallback.example');query('nx.original.example');checkpoint('audit_for_vue',api('/api/v2/audit/logs?limit=50'));checkpoint('actual_peer_questions',peers())
assert any(x['question']=='target.example.' for x in peers()) and not any(x['question']=='original.example.' for x in peers())
checkpoint('binary_sha256',hashlib.sha256((ROOT.parent/'rust/target/debug/mosdns').read_bytes()).hexdigest());checkpoint('result','PASS')
print('Actual UDP/TCP/API/SIGTERM/restart/retained-dump/Flush proof PASS')
