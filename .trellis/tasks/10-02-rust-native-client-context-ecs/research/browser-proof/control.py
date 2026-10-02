from pathlib import Path
import subprocess,json,os,signal,time,urllib.request,hashlib,sys
root=Path(__file__).parent
binary=Path('/dev/shm/mosdns-rust-client-ecs-20261002/target/debug/mosdns')
def stamp(pid):return Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[19]
def start(name,args):
    with (root/f'{name}.log').open('ab') as log:
        p=subprocess.Popen(args,cwd=root,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    (root/f'{name}.owned.json').write_text(json.dumps({'pid':p.pid,'start':stamp(p.pid),'args':args,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest() if name=='native' else None}))
    return p.pid
def stop(name):
    owned=json.loads((root/f'{name}.owned.json').read_text());pid=owned['pid'];assert stamp(pid)==owned['start'];os.kill(pid,signal.SIGTERM)
    for _ in range(300):
        try:
            data=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
            if data[0]=='Z':break
        except FileNotFoundError:break
        time.sleep(.01)
    else:raise RuntimeError('owned process did not close')
    with (root/'shutdown.jsonl').open('a') as out:out.write(json.dumps({'name':name,'pid':pid,'closed':True,'dump_exists':(root/'cache.gz').exists()})+'\n')
mode=sys.argv[1]
if mode=='start':start('services',['python3','services.py'])
elif mode in ['tcp','restart']:stop('native')
elif mode=='stop':
    stop('native');stop('services');sys.exit(0)
if mode=='tcp':
    config=(root/'config.yaml').read_text().replace('type: udp_server','type: tcp_server').replace('enable_audit: true}', 'enable_audit: true, idle_timeout: 10}');(root/'config.yaml').write_text(config)
start('native',[str(binary),'start','-c','config.yaml'])
for _ in range(300):
    try:urllib.request.urlopen('http://127.0.0.1:21980/api/v1/cache/inventory').close();break
    except OSError:time.sleep(.01)
else:raise RuntimeError('native not ready')
print('task-owned',mode,'ready')
