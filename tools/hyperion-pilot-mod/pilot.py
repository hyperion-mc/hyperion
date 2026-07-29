#!/usr/bin/env python3
"""Drive the operator's live Minecraft client through the hyperion-pilot socket.

The mod writes its chosen endpoint to ~/.hyperion-pilot/endpoint.txt, so this
finds the socket without being told. Every subcommand sends one JSON request and
prints the one JSON response. An agent's loop is: drive (hold/look/use), read
(state/screenshot), decide, repeat.

Examples:
    ./pilot.py ping
    ./pilot.py hold --forward --sprint          # start running forward
    ./pilot.py look --yaw 90 --pitch 0           # turn to face +X, level
    ./pilot.py stop                              # release everything
    ./pilot.py use --hold                        # start drawing the bow
    ./pilot.py stop                              # release use -> fires the bow
    ./pilot.py screenshot                        # -> {"ok":true,"path":"..."}
    ./pilot.py state --radius 40                 # world snapshot incl. arrows
    ./pilot.py send '{"cmd":"hold","left":true}' # raw
"""
from __future__ import annotations

import argparse
import json
import os
import socket
import sys
from pathlib import Path

HOME = Path(os.path.expanduser("~")) / ".hyperion-pilot"
ENDPOINT_FILE = HOME / "endpoint.txt"


def read_endpoint() -> str:
    if ENDPOINT_FILE.exists():
        return ENDPOINT_FILE.read_text().strip()
    # default assumption if the file is not there yet
    return f"unix:{HOME / 'control.sock'}"


def connect(endpoint: str) -> socket.socket:
    if endpoint.startswith("unix:"):
        path = endpoint[len("unix:"):]
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(path)
        return s
    if endpoint.startswith("tcp:"):
        _, host, port = endpoint.split(":")
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.connect((host, int(port)))
        return s
    raise SystemExit(f"unrecognised endpoint: {endpoint}")


def rpc(obj: dict) -> dict:
    endpoint = read_endpoint()
    s = connect(endpoint)
    try:
        s.sendall((json.dumps(obj) + "\n").encode("utf-8"))
        buf = b""
        while b"\n" not in buf:
            chunk = s.recv(65536)
            if not chunk:
                break
            buf += chunk
        line = buf.split(b"\n", 1)[0]
        return json.loads(line.decode("utf-8")) if line else {}
    finally:
        s.close()


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    sub.add_parser("ping")

    h = sub.add_parser("hold", help="set held inputs (only the flags you pass change)")
    for flag in ["forward", "back", "left", "right", "jump", "sneak", "sprint", "use", "attack"]:
        h.add_argument(f"--{flag}", action="store_true")
        h.add_argument(f"--no-{flag}", dest=f"no_{flag}", action="store_true")

    sub.add_parser("stop", help="release all held inputs")

    lk = sub.add_parser("look")
    lk.add_argument("--yaw", type=float)
    lk.add_argument("--pitch", type=float)
    lk.add_argument("--dyaw", type=float)
    lk.add_argument("--dpitch", type=float)
    lk.add_argument("--step", type=float, help="max degrees per tick")
    lk.add_argument("--instant", action="store_true")

    at = sub.add_parser("attack")
    at.add_argument("--hold", action="store_true", help="hold instead of single click")
    us = sub.add_parser("use")
    us.add_argument("--hold", action="store_true", help="hold (draw bow); stop releases/fires")

    sl = sub.add_parser("slot")
    sl.add_argument("index", type=int)

    dr = sub.add_parser("drop")
    dr.add_argument("--all", action="store_true")

    ch = sub.add_parser("chat")
    ch.add_argument("message")
    cm = sub.add_parser("command")
    cm.add_argument("command")

    sub.add_parser("screenshot")

    st = sub.add_parser("state")
    st.add_argument("--radius", type=float, default=32.0)

    rc = sub.add_parser("recent")
    rc.add_argument("--n", type=int, default=50)
    sub.add_parser("rotate")

    lg = sub.add_parser("log")
    lg.add_argument("--enabled", choices=["true", "false"])
    lg.add_argument("--inbound", choices=["true", "false"])
    lg.add_argument("--outbound", choices=["true", "false"])

    sn = sub.add_parser("send", help="raw JSON request")
    sn.add_argument("json")

    args = p.parse_args()

    if args.cmd == "hold":
        req = {"cmd": "hold"}
        for flag in ["forward", "back", "left", "right", "jump", "sneak", "sprint", "use", "attack"]:
            if getattr(args, flag):
                req[flag] = True
            if getattr(args, f"no_{flag}"):
                req[flag] = False
    elif args.cmd == "look":
        req = {"cmd": "look"}
        for k in ["yaw", "pitch", "dyaw", "dpitch", "step"]:
            v = getattr(args, k)
            if v is not None:
                req[k] = v
        if args.instant:
            req["instant"] = True
    elif args.cmd == "attack":
        req = {"cmd": "hold", "attack": True} if args.hold else {"cmd": "attack"}
    elif args.cmd == "use":
        req = {"cmd": "hold", "use": True} if args.hold else {"cmd": "use"}
    elif args.cmd == "slot":
        req = {"cmd": "slot", "index": args.index}
    elif args.cmd == "drop":
        req = {"cmd": "drop", "all": args.all}
    elif args.cmd == "chat":
        req = {"cmd": "chat", "message": args.message}
    elif args.cmd == "command":
        req = {"cmd": "command", "command": args.command}
    elif args.cmd == "state":
        req = {"cmd": "state", "radius": args.radius}
    elif args.cmd == "recent":
        req = {"cmd": "recent", "n": args.n}
    elif args.cmd == "log":
        req = {"cmd": "log"}
        for k in ["enabled", "inbound", "outbound"]:
            v = getattr(args, k)
            if v is not None:
                req[k] = (v == "true")
    elif args.cmd == "send":
        req = json.loads(args.json)
    else:
        req = {"cmd": args.cmd}

    print(json.dumps(rpc(req)))


if __name__ == "__main__":
    main()
