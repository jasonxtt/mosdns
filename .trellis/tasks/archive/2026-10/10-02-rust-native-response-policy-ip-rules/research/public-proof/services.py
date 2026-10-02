"""Start/stop only this proof's PID+start-time+cwd verified process groups."""
import os,pathlib,json,subprocess,signal,time,urllib.request,sys
ROOT=pathlib.Path(__file__).resolve().parent; BASE=ROOT.parent
# Deployed proof root is <task root>/proof; this helper is executed remotely.
def stamp(pid):return pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[19]
def active(name):
    path=ROOT/f'{name}.owned.json'
    if not path.exists():return None
    info=json.loads(path.read_text());pid=info['pid']
    try:
        if stamp(pid)!=info['start']:raise RuntimeError(f'{name} PID identity changed')
        state=pathlib.Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[0]
        if state=='Z':return None
        if pathlib.Path(f'/proc/{pid}/cwd').resolve()!=pathlib.Path(info['cwd']):raise RuntimeError(f'{name} cwd changed')
        return info
    except FileNotFoundError:return None

def start(name,args,cwd,env=None):
    if active(name):raise RuntimeError(f'{name} already running')
    with (ROOT/f'{name}.log').open('ab') as log:
        process=subprocess.Popen(args,cwd=cwd,env=env,stdin=subprocess.DEVNULL,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    (ROOT/f'{name}.owned.json').write_text(json.dumps({'pid':process.pid,'start':stamp(process.pid),'cwd':str(cwd),'args':args}))

def stop(name):
    info=active(name)
    if not info:return
    os.killpg(info['pid'],signal.SIGTERM)
    deadline=time.monotonic()+8
    while active(name) and time.monotonic()<deadline:time.sleep(.02)
    if active(name):raise RuntimeError(f'{name} did not stop/drain')

def ready():
    deadline=time.monotonic()+8
    while time.monotonic()<deadline:
        try:
            urllib.request.urlopen('http://127.0.0.1:21880/api/v1/audit/status',timeout=.2).close();return
        except OSError:time.sleep(.02)
    raise RuntimeError('native API not ready')
command=sys.argv[1]
config=sys.argv[2] if len(sys.argv)>2 else 'config.yaml'
if config not in ('config.yaml','config-tcp.yaml'):raise RuntimeError('unapproved proof config')
if command=='start':
    if not active('upstream'):start('upstream',['python3','upstream.py'],ROOT)
    start('native',[str(BASE/'rust/target/debug/mosdns'),'start','-c',config],ROOT);ready()
elif command=='restart':
    stop('native');start('native',[str(BASE/'rust/target/debug/mosdns'),'start','-c',config],ROOT);ready()
elif command=='vite':
    env=dict(os.environ);env['MOSDNS_DEV_TARGET']='http://127.0.0.1:21880'
    start('vite',['npm','run','dev','--','--config',str(ROOT/'vite.config.mjs'),'--host','127.0.0.1','--port','26173','--strictPort'],BASE/'webui-log',env)
elif command=='stop':stop('native')
elif command=='cleanup':
    for service in ['vite','native','upstream']:stop(service)
else:raise RuntimeError('unknown command')
print(command+' owned proof services PASS')
