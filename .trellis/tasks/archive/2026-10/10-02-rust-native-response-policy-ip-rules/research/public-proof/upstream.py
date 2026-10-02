"""Owned loopback DNS peers. Records the actual current question received."""
import socket,struct,pathlib,json,threading,signal
ROOT=pathlib.Path(__file__).resolve().parent
stop=threading.Event(); lock=threading.Lock()
def name(value):
    return b''.join(bytes([len(label)])+label.encode('ascii') for label in value.rstrip('.').split('.'))+b'\0'
def rr(owner,kind,ttl,data):
    return name(owner)+struct.pack('>HHIH',kind,1,ttl,len(data))+data
def serve(port,role):
    sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);sock.bind(('127.0.0.1',port));sock.settimeout(.1)
    while not stop.is_set():
        try:q,peer=sock.recvfrom(65535)
        except socket.timeout:continue
        cursor=12;labels=[]
        while q[cursor]:
            length=q[cursor];cursor+=1;labels.append(q[cursor:cursor+length].decode());cursor+=length
        domain='.'.join(labels)+'.';cursor+=1;kind,cls=struct.unpack('>HH',q[cursor:cursor+4]);cursor+=4
        negative=domain=='nx.target.example.'
        if negative:
            answers=[];authority=[rr('target.example.',6,321,name('ns.example.')+name('mb.example.')+struct.pack('>5I',1,2,3,4,5))]
        else:
            address='198.51.100.42' if role=='backup' else ('192.0.2.42' if domain in ('fallback.example.','fallback6.example.') else '198.51.100.10')
            if kind==28:
                address='2001:db8::42' if role=='primary' and domain=='fallback6.example.' else '2001:db9::42'
                payload=socket.inet_pton(socket.AF_INET6,address);answer_kind=28
            else:payload=socket.inet_aton(address);answer_kind=1
            answers=[rr(domain,5,60,name('final.example.')),rr('final.example.',answer_kind,70,payload)];authority=[]
        flags=0x8580 | (3 if negative else 0)
        response=q[:2]+struct.pack('>5H',flags,1,len(answers),len(authority),0)+q[12:cursor]+b''.join(answers+authority)
        sock.sendto(response,peer)
        with lock:
            with (ROOT/'peer-questions.jsonl').open('a') as out:out.write(json.dumps({'role':role,'question':domain,'qtype':kind,'qclass':cls})+'\n')
    sock.close()
for sig in (signal.SIGTERM,signal.SIGINT):signal.signal(sig,lambda *_:stop.set())
threads=[threading.Thread(target=serve,args=(port,role)) for port,role in [(21554,'primary'),(21555,'backup')]]
for thread in threads:thread.start()
for thread in threads:thread.join()
