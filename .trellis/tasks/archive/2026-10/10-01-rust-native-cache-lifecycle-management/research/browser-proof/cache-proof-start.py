import subprocess,pathlib,json,os
base=pathlib.Path('/root/mosdns-rust-cache-lifecycle-20261001'); root=base/'proof'
def stamp(pid): return pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
def start(name,args,cwd,env=None):
    matches=[]
    if name=='upstream':
        for entry in pathlib.Path('/proc').iterdir():
            if not entry.name.isdigit(): continue
            try:
                if (entry/'cwd').resolve()==root and (entry/'cmdline').read_bytes().split(b'\0')[:2]==[b'python3',b'upstream.py']: matches.append(int(entry.name))
            except (OSError,PermissionError): pass
    if matches:
        if len(matches)!=1: raise RuntimeError('multiple owned upstreams')
        pid=matches[0]
    else:
        with (root/f'{name}.log').open('ab') as log:
            process=subprocess.Popen(args,cwd=cwd,env=env,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True);pid=process.pid
    (root/f'{name}.owned.json').write_text(json.dumps({'pid':pid,'start':stamp(pid),'args':args,'cwd':str(cwd)}))
start('upstream',['python3','upstream.py'],root)
start('native',[str(base/'rust/target/debug/mosdns'),'start','-c','config.yaml'],root)
env=dict(os.environ);env['MOSDNS_DEV_TARGET']='http://127.0.0.1:20880'
start('vite',['npm','run','dev','--','--host','127.0.0.1','--port','25173','--strictPort'],base/'webui-log',env)
print('Owned loopback proof services started')
