#!/usr/bin/env python3
"""Show how long the app takes to answer the compositor's pings (what "Not Responding" is about).

Run the browser with Wayland's client debugging and pipe both streams in; only the late answers are printed:

    WAYLAND_DEBUG=client browsah --profile=home 2>&1 | scripts/ping-watch.py [seconds]

(`browsah` must not already be running for that profile: one process per profile.) A line is printed for every `xdg_wm_base`
ping that was answered later than `seconds` (default 1), or has not been answered by the time later traffic shows the
connection alive. The time is the client's own clock in the debug lines, so it shows when the ping *reached* the client's
dispatch and when the pong left, which separates "the compositor sent it late" from "we answered it late".
"""
import re
import sys

limit = float(sys.argv[1]) * 1000 if len(sys.argv) > 1 else 1000.0
STAMP = re.compile(r"^\[\s*(\d+\.\d+)\]")
PING = re.compile(r"\s(->\s+)?xdg_wm_base[#@]\d+\.(ping|pong)\((\d+)\)")
pending = {}  # serial -> time the ping was received (ms)
reported = set()


def report(serial, since, now, state):
    print(f"ping {serial}: {state} after {now - since:.0f} ms (received at {since:.3f}, clock in ms)", flush=True)


for line in sys.stdin:
    t = STAMP.match(line)
    if not t:
        continue
    now = float(t.group(1))
    m = PING.search(line)
    if m:
        out, kind, serial = m.group(1), m.group(2), m.group(3)
        if kind == "ping" and not out:
            pending[serial] = now
        elif kind == "pong" and out:
            since = pending.pop(serial, None)
            if since is not None and now - since > limit:
                report(serial, since, now, "answered")
    # Any traffic shows the connection is alive: flag pings that are still waiting.
    for s, since in list(pending.items()):
        if now - since > limit and s not in reported:
            reported.add(s)
            report(s, since, now, "still unanswered")
