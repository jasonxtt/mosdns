"""Task-owned loopback upstream and existing Vue bundle/API proxy."""
from pathlib import Path
import http.server, urllib.request, urllib.error, socket, struct, json, threading
root=Path(__file__).parent
bundle=root/'bundle'
def upstream():
    sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);sock.bind(('127.0.0.1',21954))
    while True:
        wire,peer=sock.recvfrom(4096)
        end=12
        while wire[end]:end+=1+wire[end]
        end+=5
        opts=wire[end+11:]; cursor=0; ecs=None
        while cursor<len(opts):
            code,size=struct.unpack('>HH',opts[cursor:cursor+4]);body=opts[cursor+4:cursor+4+size];cursor+=4+size
            if code==8:ecs=body
        assert ecs is not None and ecs[:4]==bytes([0,1,32,0]) and ecs[4:7]==bytes([127,0,0]),ecs
        last=ecs[-1]*10
        option=struct.pack('>HH',8,len(ecs))+ecs[:3]+bytes([24])+ecs[4:]
        response=wire[:2]+struct.pack('>5H',0x8180,1,1,0,1)+wire[12:end]+bytes.fromhex('c00c000100010000003c0004')+bytes([192,0,2,last])+bytes.fromhex('00002904d000000000')+struct.pack('>H',len(option))+option
        with (root/'supplier.jsonl').open('a') as out:out.write(json.dumps({'peer':list(peer),'ecs':list(ecs),'answer':f'192.0.2.{last}'})+'\n')
        sock.sendto(response,peer)
class Proxy(http.server.SimpleHTTPRequestHandler):
    def __init__(self,*args,**kwargs):super().__init__(*args,directory=str(bundle),**kwargs)
    def do_GET(self):
        if self.path.startswith(('/api/','/plugins/','/metrics')):self.proxy()
        else:super().do_GET()
    def do_POST(self):self.proxy()
    def proxy(self):
        size=int(self.headers.get('Content-Length',0));body=self.rfile.read(size) if size else None
        request=urllib.request.Request('http://127.0.0.1:21980'+self.path,data=body,method=self.command)
        try:response=urllib.request.urlopen(request)
        except urllib.error.HTTPError as error:response=error
        data=response.read();self.send_response(response.status);self.send_header('Content-Type',response.headers.get('Content-Type','application/json'));self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
threading.Thread(target=upstream,daemon=True).start()
http.server.ThreadingHTTPServer(('127.0.0.1',21981),Proxy).serve_forever()
