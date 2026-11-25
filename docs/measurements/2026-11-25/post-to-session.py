import os, socket, sys, json, time
# post2.py <auth:yes|no|wrong> <hold-seconds> <json-line> [<json-line> ...]  -- one connection, several lines
p=os.environ['CLAUDE_CODE_MESSAGING_SOCKET']; tok=os.environ['CLAUDE_CODE_MESSAGING_TOKEN']
auth, hold, lines = sys.argv[1], float(sys.argv[2]), sys.argv[3:]
s=socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect(p)
if auth!='no': s.sendall((json.dumps({"type":"auth","token":tok if auth=='yes' else 'f'*32})+"\n").encode())
for l in lines: s.sendall((l+"\n").encode()); time.sleep(0.3)
s.settimeout(hold); t0=time.time()
try: d=s.recv(4096); r='closed by peer after %.1fs'%(time.time()-t0) if d==b'' else 'reply %r'%d[:200]
except socket.timeout: r='no reply, still open after %.0fs'%hold
except Exception as e: r='<%s>'%e
print(time.strftime('%H:%M:%S'), 'auth=%s lines=%d:'%(auth,len(lines)), r); s.close()
