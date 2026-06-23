# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Tiny CI dashboard server for the installer CI backend.

This is the "VPS" side of the installer CI backend (see ``spack ci run`` for the build-machine
side). It is deliberately small enough to host on a tiny machine:

- It only ever sees the *control plane*: a per-commit pipeline DAG plus a stream of small build
  state events. Log *bytes* never traverse this server; ``/build/.../log`` redirects the browser
  to the build machine (live) or to the archived object in a mirror (finished).
- IO is a single-threaded non-blocking event loop on the stdlib ``selectors`` module
  (epoll on Linux, kqueue on macOS), mirroring ``spack.new_installer``. No thread-per-connection,
  no third-party async framework. Idle Server-Sent-Events (SSE) connections cost a file
  descriptor and a buffer, not a thread.

History (pipelines, builds, timings, log locations) is kept in a single sqlite file.
"""

import html
import json
import selectors
import socket
import sqlite3
import time
from typing import Dict, List, Optional, Set, Tuple

#: states that mean a build no longer produces log output
TERMINAL_STATES = ("finished", "failed")

SCHEMA = """
CREATE TABLE IF NOT EXISTS pipelines (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    commit_sha TEXT,
    ref        TEXT,
    log_base   TEXT,
    created_at REAL,
    status     TEXT
);
CREATE TABLE IF NOT EXISTS builds (
    pipeline_id INTEGER,
    dag_hash    TEXT,
    name        TEXT,
    version     TEXT,
    status      TEXT,
    start_ts    REAL,
    end_ts      REAL,
    log_url     TEXT,
    PRIMARY KEY (pipeline_id, dag_hash)
);
CREATE TABLE IF NOT EXISTS edges (
    pipeline_id  INTEGER,
    parent_hash  TEXT,
    child_hash   TEXT
);
"""


class Db:
    """Thin sqlite wrapper holding the pipeline/build history."""

    def __init__(self, path: str) -> None:
        # The server uses one thread; check_same_thread=False only eases embedding/testing.
        self.conn = sqlite3.connect(path, check_same_thread=False)
        self.conn.row_factory = sqlite3.Row
        self.conn.executescript(SCHEMA)
        self.conn.commit()

    def register_pipeline(self, payload: dict, now: float) -> int:
        """Insert a pipeline plus its DAG nodes and edges, returning the pipeline id."""
        cur = self.conn.execute(
            "INSERT INTO pipelines (commit_sha, ref, log_base, created_at, status) "
            "VALUES (?, ?, ?, ?, 'running')",
            (payload.get("commit"), payload.get("ref"), payload.get("log_base"), now),
        )
        pipeline_id = cur.lastrowid
        assert pipeline_id is not None
        self.conn.executemany(
            "INSERT OR REPLACE INTO builds "
            "(pipeline_id, dag_hash, name, version, status) VALUES (?, ?, ?, ?, 'pending')",
            [
                (pipeline_id, n["dag_hash"], n["name"], n.get("version", ""))
                for n in payload.get("nodes", [])
            ],
        )
        self.conn.executemany(
            "INSERT INTO edges (pipeline_id, parent_hash, child_hash) VALUES (?, ?, ?)",
            [(pipeline_id, parent, child) for parent, child in payload.get("edges", [])],
        )
        self.conn.commit()
        return pipeline_id

    def apply_event(self, pipeline_id: int, event: dict) -> None:
        """Update a single build's state/timestamps from an event."""
        dag_hash, state, ts = event["dag_hash"], event["state"], event.get("ts", time.time())
        if state in TERMINAL_STATES:
            self.conn.execute(
                "UPDATE builds SET status = ?, end_ts = ? WHERE pipeline_id = ? AND dag_hash = ?",
                (state, ts, pipeline_id, dag_hash),
            )
        else:
            # First non-terminal state for a build is its start.
            self.conn.execute(
                "UPDATE builds SET status = ?, start_ts = COALESCE(start_ts, ?) "
                "WHERE pipeline_id = ? AND dag_hash = ?",
                (state, ts, pipeline_id, dag_hash),
            )
        self.conn.commit()

    def set_log_url(self, pipeline_id: int, dag_hash: str, log_url: str) -> None:
        self.conn.execute(
            "UPDATE builds SET log_url = ? WHERE pipeline_id = ? AND dag_hash = ?",
            (log_url, pipeline_id, dag_hash),
        )
        self.conn.commit()

    def finish_pipeline(self, pipeline_id: int) -> None:
        self.conn.execute("UPDATE pipelines SET status = 'done' WHERE id = ?", (pipeline_id,))
        self.conn.commit()

    def pipelines(self) -> List[sqlite3.Row]:
        return list(self.conn.execute("SELECT * FROM pipelines ORDER BY id DESC"))

    def pipeline(self, pipeline_id: int) -> Optional[sqlite3.Row]:
        return self.conn.execute("SELECT * FROM pipelines WHERE id = ?", (pipeline_id,)).fetchone()

    def builds(self, pipeline_id: int) -> List[sqlite3.Row]:
        return list(
            self.conn.execute("SELECT * FROM builds WHERE pipeline_id = ?", (pipeline_id,))
        )

    def edges(self, pipeline_id: int) -> List[sqlite3.Row]:
        return list(self.conn.execute("SELECT * FROM edges WHERE pipeline_id = ?", (pipeline_id,)))

    def build(self, pipeline_id: int, dag_hash: str) -> Optional[sqlite3.Row]:
        return self.conn.execute(
            "SELECT * FROM builds WHERE pipeline_id = ? AND dag_hash = ?", (pipeline_id, dag_hash)
        ).fetchone()


class Hub:
    """Pure (socket-free) state: owns the Db and the set of live SSE subscribers per pipeline.

    Ingest methods return the list of SSE payload strings to broadcast, so this layer is testable
    without any networking.
    """

    def __init__(self, db: Db, time_fn=time.time) -> None:
        self.db = db
        self.time = time_fn
        #: pipeline_id -> set of subscriber connection ids (filled in by the Server)
        self.subscribers: Dict[int, Set[int]] = {}

    def register_pipeline(self, payload: dict) -> int:
        return self.db.register_pipeline(payload, self.time())

    def ingest_events(self, pipeline_id: int, events: List[dict]) -> List[str]:
        """Apply events to the db and return SSE ``data:`` payloads to broadcast."""
        messages = []
        for event in events:
            self.db.apply_event(pipeline_id, event)
            messages.append(
                json.dumps(
                    {
                        "dag_hash": event["dag_hash"],
                        "state": event["state"],
                        "ts": event.get("ts", self.time()),
                    }
                )
            )
        return messages

    def log_target(self, pipeline_id: int, dag_hash: str) -> Optional[Tuple[str, str]]:
        """Return ``(kind, url)`` to reach a build's log, or None if unknown.

        ``kind`` is ``"archived"`` (object in a mirror, prefer it once present) or ``"live"``
        (stream from the build machine while the build is still running).
        """
        build = self.db.build(pipeline_id, dag_hash)
        if build is None:
            return None
        if build["log_url"]:
            return "archived", build["log_url"]
        pipeline = self.db.pipeline(pipeline_id)
        if pipeline is None or not pipeline["log_base"]:
            return None
        if build["status"] in TERMINAL_STATES:
            return None  # finished but not yet archived
        return "live", f"{pipeline['log_base'].rstrip('/')}/log/{dag_hash}"


# ---------------------------------------------------------------------------
# Minimal non-blocking HTTP/1.1 + SSE on top of the Hub.
# ---------------------------------------------------------------------------

RECV_SIZE = 65536

SSE_PREAMBLE = (
    b"HTTP/1.1 200 OK\r\n"
    b"Content-Type: text/event-stream\r\n"
    b"Cache-Control: no-cache\r\n"
    b"Connection: keep-alive\r\n"
    b"\r\n"
    # tell EventSource to wait 2s before reconnecting
    b"retry: 2000\n\n"
)


class Request:
    __slots__ = ("method", "path", "headers", "body")

    def __init__(self, method: str, path: str, headers: Dict[str, str], body: bytes) -> None:
        self.method = method
        self.path = path
        self.headers = headers
        self.body = body


def parse_request(data: bytes) -> Optional[Tuple[Request, int]]:
    """Parse a buffered HTTP request. Returns (request, bytes_consumed) or None if incomplete."""
    idx = data.find(b"\r\n\r\n")
    if idx == -1:
        return None
    head = data[:idx].decode("latin-1").split("\r\n")
    try:
        method, path, _ = head[0].split(" ", 2)
    except ValueError:
        method, path = "GET", "/"
    headers: Dict[str, str] = {}
    for line in head[1:]:
        if ":" in line:
            key, value = line.split(":", 1)
            headers[key.strip().lower()] = value.strip()
    body_start = idx + 4
    length = int(headers.get("content-length", "0"))
    if len(data) - body_start < length:
        return None  # body still incoming
    return Request(
        method, path, headers, data[body_start : body_start + length]
    ), body_start + length


def http_response(
    status: str, body: bytes, content_type: str = "text/html; charset=utf-8"
) -> bytes:
    return (
        f"HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n"
        f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
    ).encode("latin-1") + body


def redirect(url: str) -> bytes:
    return (
        f"HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    ).encode("latin-1")


def sse_frame(data: str) -> bytes:
    return b"data: " + data.encode("utf-8") + b"\n\n"


class Conn:
    """A non-blocking client connection. Either a one-shot request or a long-lived SSE stream."""

    __slots__ = ("sock", "cid", "inbuf", "outbuf", "sse_pipeline", "close_when_drained")

    def __init__(self, sock: socket.socket, cid: int) -> None:
        self.sock = sock
        self.cid = cid
        self.inbuf = bytearray()
        self.outbuf = bytearray()
        self.sse_pipeline: Optional[int] = None
        self.close_when_drained = False


class Server:
    """Single-threaded, non-blocking dashboard server driven by a ``selectors`` event loop."""

    def __init__(self, hub: Hub, host: str = "127.0.0.1", port: int = 8080) -> None:
        self.hub = hub
        self.selector = selectors.DefaultSelector()
        self.conns: Dict[int, Conn] = {}
        self._next_cid = 0
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind((host, port))
        self.listener.listen(128)
        self.listener.setblocking(False)
        self.host, self.port = self.listener.getsockname()
        self.selector.register(self.listener, selectors.EVENT_READ, None)

    # -- event loop -------------------------------------------------------
    def serve_forever(self) -> None:
        while True:
            self.poll(timeout=None)

    def poll(self, timeout: Optional[float]) -> None:
        for key, _ in self.selector.select(timeout=timeout):
            if key.data is None:
                self._accept()
            else:
                self._on_ready(key.data, key.events)

    def _accept(self) -> None:
        try:
            sock, _ = self.listener.accept()
        except BlockingIOError:
            return
        sock.setblocking(False)
        conn = Conn(sock, self._next_cid)
        self._next_cid += 1
        self.conns[conn.cid] = conn
        self.selector.register(sock, selectors.EVENT_READ, conn)

    def _on_ready(self, conn: Conn, events: int) -> None:
        if events & selectors.EVENT_READ:
            self._on_read(conn)
        if conn.cid in self.conns and events & selectors.EVENT_WRITE:
            self._on_write(conn)

    def _on_read(self, conn: Conn) -> None:
        try:
            data = conn.sock.recv(RECV_SIZE)
        except (BlockingIOError, ConnectionError):
            return
        if not data:
            # client closed (e.g. browser navigated away from an SSE stream)
            self._close(conn)
            return
        if conn.sse_pipeline is not None:
            return  # SSE clients don't send more requests
        conn.inbuf += data
        parsed = parse_request(bytes(conn.inbuf))
        if parsed is None:
            return
        request, consumed = parsed
        del conn.inbuf[:consumed]
        self._dispatch(conn, request)

    def _on_write(self, conn: Conn) -> None:
        if conn.outbuf:
            try:
                sent = conn.sock.send(conn.outbuf)
            except (BlockingIOError, ConnectionError):
                return
            del conn.outbuf[:sent]
        if not conn.outbuf and conn.close_when_drained:
            self._close(conn)
        else:
            self._update_interest(conn)

    def _update_interest(self, conn: Conn) -> None:
        events = selectors.EVENT_READ
        if conn.outbuf:
            events |= selectors.EVENT_WRITE
        try:
            self.selector.modify(conn.sock, events, conn)
        except KeyError:
            pass

    def _close(self, conn: Conn) -> None:
        if conn.sse_pipeline is not None:
            self.hub.subscribers.get(conn.sse_pipeline, set()).discard(conn.cid)
        try:
            self.selector.unregister(conn.sock)
        except KeyError:
            pass
        conn.sock.close()
        self.conns.pop(conn.cid, None)

    def _send(self, conn: Conn, data: bytes, *, close: bool = True) -> None:
        conn.outbuf += data
        conn.close_when_drained = close
        self._update_interest(conn)

    # -- broadcasting -----------------------------------------------------
    def broadcast(self, pipeline_id: int, message: str) -> None:
        frame = sse_frame(message)
        for cid in list(self.hub.subscribers.get(pipeline_id, ())):
            conn = self.conns.get(cid)
            if conn is not None:
                conn.outbuf += frame
                self._update_interest(conn)

    # -- routing ----------------------------------------------------------
    def _dispatch(self, conn: Conn, request: Request) -> None:
        try:
            self._route(conn, request)
        except Exception as e:  # never let one bad request kill the loop
            self._send(conn, http_response("500 Internal Server Error", str(e).encode()))

    def _route(self, conn: Conn, request: Request) -> None:
        method, path = request.method, request.path
        parts = [p for p in path.split("?")[0].strip("/").split("/") if p]

        if method == "POST" and parts == ["api", "pipelines"]:
            pid = self.hub.register_pipeline(json.loads(request.body or b"{}"))
            return self._send(conn, self._json({"pipeline_id": pid}))

        if method == "POST" and parts[:2] == ["api", "pipelines"] and parts[-1:] == ["events"]:
            pid = int(parts[2])
            for message in self.hub.ingest_events(pid, json.loads(request.body)["events"]):
                self.broadcast(pid, message)
            return self._send(conn, self._json({"ok": True}))

        if method == "POST" and parts[:2] == ["api", "pipelines"] and parts[-1:] == ["done"]:
            self.hub.db.finish_pipeline(int(parts[2]))
            return self._send(conn, self._json({"ok": True}))

        if method == "POST" and parts[:2] == ["api", "pipelines"] and parts[-1:] == ["log_url"]:
            # /api/pipelines/<id>/builds/<hash>/log_url
            self.hub.db.set_log_url(int(parts[2]), parts[4], json.loads(request.body)["url"])
            return self._send(conn, self._json({"ok": True}))

        if method == "GET" and not parts:
            return self._send(conn, http_response("200 OK", self._index_html()))

        if method == "GET" and parts[:1] == ["api"] and parts[1:2] == ["pipeline"]:
            return self._send(conn, self._json(self._pipeline_json(int(parts[2]))))

        if method == "GET" and parts[:1] == ["pipeline"] and parts[-1:] == ["events"]:
            return self._subscribe(conn, int(parts[1]))

        if method == "GET" and parts[:1] == ["pipeline"]:
            return self._send(conn, http_response("200 OK", self._pipeline_html(int(parts[1]))))

        if method == "GET" and parts[:1] == ["build"] and parts[-1:] == ["log"]:
            target = self.hub.log_target(int(parts[1]), parts[2])
            if target is None:
                body = b"<pre>no log available yet</pre>"
                return self._send(conn, http_response("200 OK", body))
            return self._send(conn, redirect(target[1]))

        self._send(conn, http_response("404 Not Found", b"not found"))

    def _subscribe(self, conn: Conn, pipeline_id: int) -> None:
        conn.sse_pipeline = pipeline_id
        self.hub.subscribers.setdefault(pipeline_id, set()).add(conn.cid)
        self._send(conn, SSE_PREAMBLE, close=False)

    # -- views ------------------------------------------------------------
    def _json(self, obj) -> bytes:
        return http_response("200 OK", json.dumps(obj).encode(), "application/json")

    def _pipeline_json(self, pipeline_id: int) -> dict:
        return {
            "nodes": [
                {
                    "dag_hash": b["dag_hash"],
                    "name": b["name"],
                    "version": b["version"],
                    "status": b["status"],
                }
                for b in self.hub.db.builds(pipeline_id)
            ],
            "edges": [[e["parent_hash"], e["child_hash"]] for e in self.hub.db.edges(pipeline_id)],
        }

    def _index_html(self) -> bytes:
        rows = "".join(
            f'<li><a href="/pipeline/{p["id"]}">#{p["id"]} {html.escape(p["commit_sha"] or "")} '
            f"({html.escape(p['ref'] or '')})</a> - {p['status']}</li>"
            for p in self.hub.db.pipelines()
        )
        return f"<!doctype html><title>spack ci</title><h1>Pipelines</h1><ul>{rows}</ul>".encode()

    def _pipeline_html(self, pipeline_id: int) -> bytes:
        return PIPELINE_HTML.replace("__PID__", str(pipeline_id)).encode()


#: Single-page DAG view. Renders nodes colored by state and live-updates them over SSE.
PIPELINE_HTML = """<!doctype html><html><head><meta charset="utf-8">
<title>spack ci pipeline __PID__</title>
<style>
 /* Fill the viewport exactly: the iframe absorbs leftover space and scrolls internally, so
    the page itself never grows a scrollbar. The node list gets its own capped scroll area. */
 html,body{height:100%}
 body{margin:0;font-family:monospace;display:flex;flex-direction:column;height:100vh;overflow:hidden}
 h1{margin:8px;font-size:1.2rem}
 h1 a{color:#36c;text-decoration:none}
 h1 a:hover{text-decoration:underline}
 /* Column-major grid (topo order): nodes fill a few rows top-to-bottom, then add columns to
    the right, so horizontal space is used and overflow scrolls sideways instead of down. Static
    labels mean state changes only recolor the left bar -- columns never reflow. */
 #g{margin:0 8px 8px;flex:0 0 auto;overflow-x:auto;overflow-y:hidden;
    display:grid;grid-auto-flow:column;grid-template-rows:repeat(6,auto);
    grid-auto-columns:max-content;gap:3px 8px}
 .n{padding:0 8px;line-height:1.6em;white-space:nowrap;text-decoration:none;color:#fff;
    border-radius:3px;background:#888}
 /* Background carries state, so hover just brightens -- works on any state color. */
 .n:hover{filter:brightness(1.18)}
 .pending{background:#888}.staging{background:#36c}.running{background:#36c}
 .finished{background:#2a2}.failed{background:#c33}
 /* Fixed-width phase field: the item's width stays constant as the state text changes, so
    columns don't jump. Overlong states (e.g. "stopped before X") ellipsize instead. */
 .s{display:inline-block;width:13ch;overflow:hidden;text-overflow:ellipsis;
    vertical-align:bottom;color:rgba(255,255,255,.75)}
 #log{flex:1 1 auto;min-height:0;width:100%;border:0;border-top:1px solid #ccc}
</style></head><body>
<h1><a href="/">&larr; pipelines</a> / pipeline __PID__</h1><div id="g">loading...</div>
<iframe id="log" name="logframe" title="build log"></iframe>
<script>
const PID=__PID__;
const nodes={}, els={};
function cls(s){
 if(s==='pending'||s==='finished'||s==='failed')return s;
 return 'running';
}
// Topological order, dependencies first (matches the bottom-up build order). Edges are
// [parent, child] = "parent depends on child", so a node is emitted once all its children are.
function topo(d){
 const indeg={}, deps_of={};
 d.nodes.forEach(n=>{indeg[n.dag_hash]=0;deps_of[n.dag_hash]=[];});
 d.edges.forEach(e=>{const p=e[0],c=e[1];if(deps_of[c]){deps_of[c].push(p);indeg[p]++;}});
 const ready=d.nodes.map(n=>n.dag_hash).filter(h=>indeg[h]===0).sort();
 const out=[], seen=new Set();
 while(ready.length){
  const h=ready.shift();out.push(h);seen.add(h);
  deps_of[h].forEach(p=>{if(--indeg[p]===0)ready.push(p);});
 }
 d.nodes.forEach(n=>{if(!seen.has(n.dag_hash))out.push(n.dag_hash);});
 return out;
}
// Build each row once with a static label; updates only recolor and swap the phase text, so
// the layout never reflows.
function paint(h){
 const n=nodes[h];
 let a=els[h];
 if(!a){
  a=document.createElement('a');a.target='logframe';a.href='/build/'+PID+'/'+h+'/log';
  const id=document.createElement('span');
  id.textContent=n.name+'@'+n.version+' /'+h.slice(0,7)+' ';
  const s=document.createElement('span');s.className='s';
  a.appendChild(id);a.appendChild(s);
  els[h]=a;document.getElementById('g').appendChild(a);
 }
 a.className='n '+cls(n.status);
 a.lastChild.textContent='['+n.status+']';
}
// Coalesce bursts of events into a single repaint per animation frame.
const queued=new Set();
let scheduled=false;
function schedule(h){
 queued.add(h);
 if(scheduled)return;
 scheduled=true;
 requestAnimationFrame(()=>{scheduled=false;queued.forEach(paint);queued.clear();});
}
fetch('/api/pipeline/'+PID).then(r=>r.json()).then(d=>{
 d.nodes.forEach(n=>nodes[n.dag_hash]=n);
 document.getElementById('g').textContent='';
 topo(d).forEach(paint);
 const es=new EventSource('/pipeline/'+PID+'/events');
 es.onmessage=e=>{
  const m=JSON.parse(e.data);
  if(nodes[m.dag_hash]){nodes[m.dag_hash].status=m.state;schedule(m.dag_hash);}
 };
});
</script></body></html>"""
