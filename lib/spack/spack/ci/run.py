# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Build-machine side of the installer CI backend (``spack ci run``).

Instead of generating one GitLab job per package build, this concretizes an environment to a
lockfile and installs the whole DAG on a single machine under the new installer's jobserver
(``spack.new_installer``). It reports tiny build state events to the dashboard server
(``spack.ci.server``) and serves live logs straight from the local log files, so log bytes
never round-trip through the dashboard. Finished logs are archived once and served from there.

The split that keeps the dashboard cheap:

- control plane: small state events (``staging`` -> phase -> ``finished``/``failed``) posted to
  the server. Emitted via :class:`spack.new_installer.InstallEventSink` and batched off the
  installer's hot loop by a background thread.
- data plane: log bytes stay on this machine. A small HTTP endpoint tails the local log file as
  Server-Sent Events for builds a human is actually watching; nothing is streamed otherwise.
"""

import json
import os
import socket
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Callable, Dict, List, Optional, Tuple

import spack.installer_dispatch
import spack.new_installer

#: how often the background thread flushes batched events to the server
FLUSH_INTERVAL = 0.5

#: states that mean a build no longer produces log output
TERMINAL_STATES = ("finished", "failed")


class ServerClient:
    """Minimal JSON-over-HTTP client for the dashboard server."""

    def __init__(self, base_url: str, timeout: float = 10.0) -> None:
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout

    def _post(self, path: str, payload: dict) -> dict:
        data = json.dumps(payload).encode()
        request = urllib.request.Request(
            self.base_url + path, data=data, headers={"Content-Type": "application/json"}
        )
        with urllib.request.urlopen(request, timeout=self.timeout) as response:
            body = response.read()
        return json.loads(body) if body else {}

    def register_pipeline(self, payload: dict) -> int:
        return self._post("/api/pipelines", payload)["pipeline_id"]

    def post_events(self, pipeline_id: int, events: List[dict]) -> None:
        self._post(f"/api/pipelines/{pipeline_id}/events", {"events": events})

    def set_log_url(self, pipeline_id: int, dag_hash: str, url: str) -> None:
        self._post(f"/api/pipelines/{pipeline_id}/builds/{dag_hash}/log_url", {"url": url})

    def finish(self, pipeline_id: int) -> None:
        self._post(f"/api/pipelines/{pipeline_id}/done", {})


#: archives a finished build's compressed log and returns a URL to it, or None for live-only.
ArchiveFn = Callable[[str, str], Optional[str]]


class CIEventSink(spack.new_installer.InstallEventSink):
    """Funnels installer build events to the dashboard server without blocking the build loop.

    Events are buffered under a lock and posted by a background thread, so the installer's hot
    loop only ever does a cheap append. The sink also tracks each build's log path and state so
    the local log endpoint knows what file to tail and when a build is done.
    """

    def __init__(
        self,
        client: ServerClient,
        pipeline_id: int,
        archive: Optional[ArchiveFn] = None,
        time_fn: Callable[[], float] = time.time,
    ) -> None:
        self.client = client
        self.pipeline_id = pipeline_id
        self.archive = archive
        self.time = time_fn
        self._lock = threading.Lock()
        self._pending: List[dict] = []
        #: dag_hash -> local log file path (read by the log endpoint)
        self.log_paths: Dict[str, str] = {}
        #: dag_hash -> latest state (read by the log endpoint to know when to stop tailing)
        self.states: Dict[str, str] = {}
        self._stop = threading.Event()
        self._thread: Optional[threading.Thread] = None

    # -- InstallEventSink (called from the installer loop; must not block) --
    def on_build_added(self, build_id: str, info: "spack.new_installer.BuildInfo") -> None:
        if info.log_path:
            self.log_paths[build_id] = info.log_path
        self._record(build_id, info.state)

    def on_state_change(self, build_id: str, info: "spack.new_installer.BuildInfo") -> None:
        if info.log_path:
            self.log_paths[build_id] = info.log_path
        self._record(build_id, info.state)

    def _record(self, build_id: str, state: str) -> None:
        # Called on the installer's hot loop: only cheap, non-blocking work here. Network POSTs
        # happen on the background flush thread, never inline.
        event = {"dag_hash": build_id, "state": state, "ts": self.time()}
        self.states[build_id] = state
        with self._lock:
            self._pending.append(event)

    # -- background flushing ----------------------------------------------
    def start(self) -> None:
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=2 * FLUSH_INTERVAL)
        self.flush()

    def _run(self) -> None:
        while not self._stop.wait(FLUSH_INTERVAL):
            self.flush()

    def flush(self) -> None:
        """Post buffered events, then archive logs of any builds that have just finished."""
        with self._lock:
            events, self._pending = self._pending, []
        if events:
            try:
                self.client.post_events(self.pipeline_id, events)
            except OSError:
                # transient server hiccup; drop rather than wedge the build
                pass
        if self.archive is not None:
            for event in events:
                if event["state"] in TERMINAL_STATES:
                    self._archive(event["dag_hash"])

    def _archive(self, build_id: str) -> None:
        log_path = self.log_paths.get(build_id)
        if not log_path:
            return
        try:
            url = self.archive(build_id, log_path)  # type: ignore[misc]
        except OSError:
            return
        if url:
            try:
                self.client.set_log_url(self.pipeline_id, build_id, url)
            except OSError:
                pass


#: HTML head streamed before the log body. The browser renders the <pre> natively as bytes
#: arrive (no JS). Sticky-bottom scrolling is pure CSS: a `flex-direction: column-reverse`
#: scroll container has its scroll origin at the bottom, so it stays pinned to the newest
#: output as it streams and only releases when the user scrolls up.
LOG_PAGE_HEAD = b"""<!doctype html><html><head><meta charset="utf-8"><style>
 html,body{margin:0;height:100%;background:#111;color:#ddd}
 #s{height:100%;overflow:auto;display:flex;flex-direction:column-reverse}
 pre{margin:0;padding:6px;font:12px/1.4 monospace;white-space:pre-wrap;word-break:break-all}
</style></head><body><div id="s"><pre>"""


def _html_escape(data: bytes) -> bytes:
    # Escape only the three HTML-significant ASCII bytes. They never occur inside multi-byte
    # UTF-8 sequences, so this is safe to apply chunk-by-chunk to a raw byte stream.
    return data.replace(b"&", b"&amp;").replace(b"<", b"&lt;").replace(b">", b"&gt;")


class _LogHandler(BaseHTTPRequestHandler):
    """Serves ``GET /log/<dag_hash>`` as a streaming HTML page of the build's local log file.

    The log is forwarded as raw bytes (HTML-escaped) into a <pre>; the browser streams and
    renders it natively. No SSE, no client-side DOM construction, and -- because the iframe
    merely navigates here -- no CORS.
    """

    def log_message(self, *args) -> None:  # silence default stderr logging
        pass

    def do_GET(self) -> None:
        parts = [p for p in self.path.split("?")[0].strip("/").split("/") if p]
        if parts[:1] != ["log"] or len(parts) != 2:
            self.send_error(404)
            return
        sink: CIEventSink = self.server.sink  # type: ignore[attr-defined]
        dag_hash = parts[1]
        log_path = sink.log_paths.get(dag_hash)
        if not log_path:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()
        self.wfile.write(LOG_PAGE_HEAD)
        self._stream(sink, dag_hash, log_path)

    def _stream(self, sink: CIEventSink, dag_hash: str, log_path: str) -> None:
        deadline_after_done = 0.5
        try:
            with open(log_path, "rb") as f:
                while True:
                    chunk = f.read(65536)
                    if chunk:
                        self.wfile.write(_html_escape(chunk))
                        self.wfile.flush()
                        continue
                    if sink.states.get(dag_hash) in TERMINAL_STATES:
                        time.sleep(deadline_after_done)  # let final bytes land
                        self.wfile.write(_html_escape(f.read()))
                        self.wfile.write(b"</pre></div></body></html>")
                        self.wfile.flush()
                        return
                    time.sleep(0.2)
        except (BrokenPipeError, ConnectionError):
            return  # viewer closed the tab


class LogServer:
    """Build-machine HTTP endpoint that tails local log files (thread-per-watcher).

    Run on the build node, which has CPU to spare; the count of live connections equals the
    number of humans actively watching, which is what keeps this cheap.
    """

    def __init__(self, sink: CIEventSink, host: str = "0.0.0.0", port: int = 0) -> None:
        self.httpd = ThreadingHTTPServer((host, port), _LogHandler)
        self.httpd.sink = sink  # type: ignore[attr-defined]
        self.host, self.port = self.httpd.server_address[0], self.httpd.server_address[1]
        self._thread: Optional[threading.Thread] = None

    def start(self) -> None:
        self._thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()


def dag_payload(roots) -> Tuple[List[dict], List[List[str]]]:
    """Build (nodes, edges) for the dashboard from concrete root specs.

    Edges are ``[parent_hash, child_hash]`` meaning "parent depends on child".
    """
    nodes: Dict[str, dict] = {}
    edges = set()
    for root in roots:
        for spec in root.traverse():
            nodes[spec.dag_hash()] = {
                "dag_hash": spec.dag_hash(),
                "name": spec.name,
                "version": str(spec.version),
            }
            for dep in spec.dependencies():
                edges.add((spec.dag_hash(), dep.dag_hash()))
    return list(nodes.values()), [list(e) for e in edges]


def make_directory_archiver(directory: str, base_url: str) -> ArchiveFn:
    """Archive logs by copying the compressed log into ``directory``, served at ``base_url``.

    A stand-in for pushing to a binary mirror / object store; swap in an S3/mirror uploader for
    production. Returns None when no compressed log exists yet (live-only).
    """
    os.makedirs(directory, exist_ok=True)

    def archive(dag_hash: str, log_path: str) -> Optional[str]:
        # Prefer the compressed, installed log next to the metadata if present.
        gz = log_path + ".gz"
        source = gz if os.path.exists(gz) else log_path
        if not os.path.exists(source):
            return None
        dest_name = f"{dag_hash}{os.path.splitext(source)[1]}"
        with open(source, "rb") as src, open(os.path.join(directory, dest_name), "wb") as dst:
            dst.write(src.read())
        return f"{base_url.rstrip('/')}/{dest_name}"

    return archive


def run(
    env,
    server_url: str,
    *,
    commit: str = "",
    ref: str = "",
    log_host: Optional[str] = None,
    log_port: int = 0,
    archive: Optional[ArchiveFn] = None,
) -> int:
    """Install the active environment's DAG on this machine, reporting to the dashboard server.

    Returns the number of failed builds.
    """
    roots = [concrete for _, concrete in env.concretized_specs()]
    if not roots:
        raise ValueError("environment has no concretized specs; run `spack concretize` first")

    nodes, edges = dag_payload(roots)
    client = ServerClient(server_url)

    sink = CIEventSink(client, pipeline_id=0, archive=archive)
    log_server = LogServer(sink, port=log_port)
    log_base = f"http://{log_host or socket.gethostname()}:{log_server.port}"

    sink.pipeline_id = client.register_pipeline(
        {"commit": commit, "ref": ref, "log_base": log_base, "nodes": nodes, "edges": edges}
    )

    log_server.start()
    sink.start()
    try:
        installs = [spec.package for spec in roots]
        builder = spack.installer_dispatch.create_installer(
            installs, explicit={spec.dag_hash() for spec in roots}, event_sink=sink
        )
        builder.install()
    finally:
        sink.stop()
        try:
            client.finish(sink.pipeline_id)
        except OSError:
            pass
        log_server.stop()

    return sum(1 for state in sink.states.values() if state == "failed")
