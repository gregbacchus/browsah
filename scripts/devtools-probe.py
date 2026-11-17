import json, socket, base64, os, struct, sys, time, urllib.request

def targets(port):
    return json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/json", timeout=3))

def ws_eval(url, expr):
    host, rest = url[5:].split("/", 1)
    h, p = host.split(":")
    s = socket.create_connection((h, int(p)), timeout=5)
    key = base64.b64encode(os.urandom(16)).decode()
    s.sendall((f"GET /{rest} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nOrigin: http://{host}\r\n\r\n").encode())
    buf = b""
    while b"\r\n\r\n" not in buf:
        buf += s.recv(4096)
    msg = json.dumps({"id": 1, "method": "Runtime.evaluate", "params": {"expression": expr, "returnByValue": True}}).encode()
    mask = os.urandom(4)
    hdr = bytes([0x81])
    n = len(msg)
    hdr += bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + struct.pack(">H", n)
    s.sendall(hdr + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(msg)))
    data = b""
    deadline = time.time() + 5
    while time.time() < deadline:
        chunk = s.recv(65536)
        data += chunk
        # parse a single unfragmented text frame
        if len(data) >= 2:
            l = data[1] & 0x7F
            off = 2
            if l == 126: l = struct.unpack(">H", data[2:4])[0]; off = 4
            elif l == 127: l = struct.unpack(">Q", data[2:10])[0]; off = 10
            if len(data) >= off + l:
                return json.loads(data[off:off + l])
    return None

port = int(sys.argv[1])
deadline = time.time() + 20
while time.time() < deadline:
    try:
        ts = targets(port)
    except Exception:
        time.sleep(0.3); continue
    dev = [t for t in ts if "devtools_app" in t.get("url", "") or t.get("url", "").startswith("devtools://")]
    if dev:
        for t in dev[:1]:
            print("target:", t.get("title"), t.get("url")[:80])
            r = ws_eval(t["webSocketDebuggerUrl"], "JSON.stringify({dpr: devicePixelRatio, innerW: innerWidth, innerH: innerHeight, screenW: screen.width, outerW: outerWidth, fontPx: getComputedStyle(document.body).fontSize})")
            print("devtools frontend:", r["result"]["result"]["value"] if r else r)
        break
    time.sleep(0.3)
else:
    print("no devtools target found", [t.get("url") for t in ts][:5])
