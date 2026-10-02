import socket,struct,pathlib,json,time
root=pathlib.Path('/root/mosdns-rust-cache-lifecycle-20261001/proof')
sock=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);sock.bind(('127.0.0.1',20554))
count=0
while True:
    q,peer=sock.recvfrom(65535);count+=1
    if count==2:
        deadline=time.monotonic()+3
        while not (root/'release-refresh').exists() and time.monotonic()<deadline: time.sleep(.01)
    ttl=2 if count==1 else 300
    r=q[:2]+bytes([0x81,0x80,0,1,0,1,0,0,0,0])+q[12:]+bytes([0xc0,12,0,1,0,1])+struct.pack('>I',ttl)+bytes([0,4,192,0,2,min(count,254)])
    sock.sendto(r,peer); (root/'upstream-count.json').write_text(json.dumps({'count':count}))
