#!/usr/bin/env python3
# A native messaging host for the self-test: replies {"echo": <message>} (messages are a 4-byte length, then JSON).
import json, struct, sys
n = struct.unpack('<I', sys.stdin.buffer.read(4))[0]
message = json.loads(sys.stdin.buffer.read(n))
reply = json.dumps({"echo": message}).encode()
sys.stdout.buffer.write(struct.pack('<I', len(reply)) + reply)
sys.stdout.buffer.flush()
