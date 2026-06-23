# Copyright Spack Project Developers. See COPYRIGHT file for details.
#
# SPDX-License-Identifier: (Apache-2.0 OR MIT)

"""Tests for the single-node installer CI backend (`spack ci run`/`server`)."""

import json
import socket
import sys
import threading
import time
import types

import pytest

if sys.platform == "win32":
    pytest.skip("No Windows support", allow_module_level=True)

import spack.ci.run as ci_run
import spack.ci.server as ci_server
import spack.concretize
from spack.new_installer import BuildStatus, InstallEventSink


class RecordingSink(InstallEventSink):
    def __init__(self):
        self.added = []
        self.changed = []

    def on_build_added(self, build_id, info):
        self.added.append((build_id, info.state))

    def on_state_change(self, build_id, info):
        self.changed.append((build_id, info.state))


def _concrete(name="trivial-install-test-package"):
    return spack.concretize.concretize_one(name)


# ---------------------------------------------------------------------------
# Installer event hook
# ---------------------------------------------------------------------------


def test_build_status_emits_events(config, mock_packages):
    sink = RecordingSink()
    status = BuildStatus(total=1, event_sink=sink)
    status.headless = True  # skip terminal print path
    spec = _concrete()

    status.add_build(spec, explicit=True)
    status.update_state(spec.dag_hash(), "configure")
    status.update_state(spec.dag_hash(), "finished")

    assert sink.added == [(spec.dag_hash(), "starting")]
    assert sink.changed == [(spec.dag_hash(), "configure"), (spec.dag_hash(), "finished")]


def test_build_status_default_sink_is_noop(config, mock_packages):
    # No event_sink: a no-op InstallEventSink is used and nothing raises.
    status = BuildStatus(total=1)
    status.headless = True
    spec = _concrete()
    status.add_build(spec, explicit=True)
    status.update_state(spec.dag_hash(), "finished")
    assert isinstance(status.event_sink, InstallEventSink)


# ---------------------------------------------------------------------------
# Dashboard server: db + hub (no sockets)
# ---------------------------------------------------------------------------


def _payload():
    return {
        "commit": "c1",
        "ref": "r1",
        "log_base": "http://build:9000",
        "nodes": [
            {"dag_hash": "h1", "name": "zlib", "version": "1.3"},
            {"dag_hash": "h2", "name": "cmake", "version": "3.2"},
        ],
        "edges": [["h2", "h1"]],
    }


def test_db_register_and_events():
    db = ci_server.Db(":memory:")
    pid = db.register_pipeline(_payload(), now=10.0)
    assert {b["dag_hash"]: b["status"] for b in db.builds(pid)} == {
        "h1": "pending",
        "h2": "pending",
    }
    assert [list(e) for e in [(e["parent_hash"], e["child_hash"]) for e in db.edges(pid)]] == [
        ["h2", "h1"]
    ]

    db.apply_event(pid, {"dag_hash": "h1", "state": "configure", "ts": 11.0})
    db.apply_event(pid, {"dag_hash": "h1", "state": "finished", "ts": 12.0})
    h1 = db.build(pid, "h1")
    assert h1["status"] == "finished"
    assert h1["start_ts"] == 11.0 and h1["end_ts"] == 12.0


def test_hub_log_target():
    hub = ci_server.Hub(ci_server.Db(":memory:"), time_fn=lambda: 0.0)
    pid = hub.register_pipeline(_payload())

    # running build -> live stream from the build machine
    assert hub.log_target(pid, "h2") == ("live", "http://build:9000/log/h2")

    # finished but not archived -> no log yet
    hub.ingest_events(pid, [{"dag_hash": "h1", "state": "finished"}])
    assert hub.log_target(pid, "h1") is None

    # archived -> serve from the mirror url
    hub.db.set_log_url(pid, "h1", "http://mirror/h1.gz")
    assert hub.log_target(pid, "h1") == ("archived", "http://mirror/h1.gz")


# ---------------------------------------------------------------------------
# Dashboard server: full non-blocking HTTP/SSE loop over a real socket
# ---------------------------------------------------------------------------


class RunningServer:
    def __init__(self):
        self.hub = ci_server.Hub(ci_server.Db(":memory:"))
        self.server = ci_server.Server(self.hub, "127.0.0.1", 0)
        self.host, self.port = self.server.host, self.server.port
        self._stop = False
        self._thread = threading.Thread(target=self._loop, daemon=True)

    def _loop(self):
        while not self._stop:
            self.server.poll(timeout=0.02)

    def __enter__(self):
        self._thread.start()
        return self

    def __exit__(self, *exc):
        self._stop = True
        self._thread.join(timeout=1)

    def request(self, method, path, body=None):
        data = body.encode() if body else b""
        sock = socket.create_connection((self.host, self.port))
        sock.settimeout(2)
        sock.sendall(
            (
                f"{method} {path} HTTP/1.1\r\nHost: x\r\n"
                f"Content-Length: {len(data)}\r\nConnection: close\r\n\r\n"
            ).encode()
            + data
        )
        chunks = b""
        try:
            while True:
                piece = sock.recv(4096)
                if not piece:
                    break
                chunks += piece
        except socket.timeout:
            pass
        sock.close()
        head, _, payload = chunks.partition(b"\r\n\r\n")
        return head.split(b"\r\n")[0].decode(), payload


def test_server_register_events_and_sse():
    with RunningServer() as srv:
        status, body = srv.request("POST", "/api/pipelines", json.dumps(_payload()))
        assert status.endswith("200 OK")
        pid = json.loads(body)["pipeline_id"]

        # open an SSE subscription before posting events
        sse = socket.create_connection((srv.host, srv.port))
        sse.sendall(f"GET /pipeline/{pid}/events HTTP/1.1\r\nHost: x\r\n\r\n".encode())
        time.sleep(0.1)

        srv.request(
            "POST",
            f"/api/pipelines/{pid}/events",
            json.dumps({"events": [{"dag_hash": "h1", "state": "finished", "ts": 1.0}]}),
        )

        sse.settimeout(1)
        got = b""
        try:
            while b"data:" not in got:
                got += sse.recv(4096)
        except socket.timeout:
            pass
        sse.close()
        assert b'"dag_hash": "h1"' in got and b'"state": "finished"' in got

        # the JSON view reflects the new state
        _, body = srv.request("GET", f"/api/pipeline/{pid}")
        nodes = {n["dag_hash"]: n["status"] for n in json.loads(body)["nodes"]}
        assert nodes["h1"] == "finished"


def test_server_log_redirects():
    with RunningServer() as srv:
        _, body = srv.request("POST", "/api/pipelines", json.dumps(_payload()))
        pid = json.loads(body)["pipeline_id"]

        # running build redirects to the live endpoint
        status, _ = srv.request("GET", f"/build/{pid}/h2/log")
        assert status.endswith("302 Found")

        # archived build redirects to the mirror url
        srv.request(
            "POST",
            f"/api/pipelines/{pid}/builds/h1/log_url",
            json.dumps({"url": "http://mirror/h1.gz"}),
        )
        status, _ = srv.request("GET", f"/build/{pid}/h1/log")
        assert status.endswith("302 Found")


def test_server_pipeline_page_wires_iframe():
    with RunningServer() as srv:
        _, body = srv.request("POST", "/api/pipelines", json.dumps(_payload()))
        pid = json.loads(body)["pipeline_id"]

        # the pipeline page hosts an iframe; node links navigate it straight to /log so the
        # browser loads the log natively (no SSE/JS viewer)
        _, page = srv.request("GET", f"/pipeline/{pid}")
        assert b'name="logframe"' in page
        assert b"a.target='logframe'" in page
        assert b"'/build/'+PID+'/'+h+'/log'" in page
        # nodes are topologically ordered with a static name@version /shorthash label
        assert b"function topo(" in page
        assert b"n.name+'@'+n.version+' /'+h.slice(0,7)" in page

        # a build with no log yet renders a plain placeholder, not a redirect
        status, page = srv.request("GET", f"/build/{pid}/nope/log")
        assert status.endswith("200 OK")
        assert b"no log available yet" in page


# ---------------------------------------------------------------------------
# Build-machine event sink
# ---------------------------------------------------------------------------


class FakeClient:
    def __init__(self):
        self.events = []
        self.log_urls = []

    def post_events(self, pipeline_id, events):
        self.events.extend(events)

    def set_log_url(self, pipeline_id, dag_hash, url):
        self.log_urls.append((dag_hash, url))


def _info(state, log_path=None):
    return types.SimpleNamespace(state=state, log_path=log_path)


def test_event_sink_is_non_blocking_until_flush():
    client = FakeClient()
    sink = ci_run.CIEventSink(client, pipeline_id=1, time_fn=lambda: 0.0)

    sink.on_build_added("h1", _info("staging", "/tmp/h1.log"))
    sink.on_state_change("h1", _info("build", "/tmp/h1.log"))

    # nothing posted inline (no network on the installer hot loop)
    assert client.events == []
    assert sink.log_paths["h1"] == "/tmp/h1.log"
    assert sink.states["h1"] == "build"

    sink.flush()
    assert [(e["dag_hash"], e["state"]) for e in client.events] == [
        ("h1", "staging"),
        ("h1", "build"),
    ]


def test_event_sink_archives_on_finish(tmp_path):
    log = tmp_path / "h1.log"
    log.write_text("output\n")
    archived = []

    def archive(dag_hash, log_path):
        archived.append((dag_hash, log_path))
        return f"http://mirror/{dag_hash}.gz"

    client = FakeClient()
    sink = ci_run.CIEventSink(client, pipeline_id=1, archive=archive, time_fn=lambda: 0.0)
    sink.on_build_added("h1", _info("staging", str(log)))
    sink.on_state_change("h1", _info("finished", str(log)))
    sink.flush()

    assert archived == [("h1", str(log))]
    assert client.log_urls == [("h1", "http://mirror/h1.gz")]


def test_directory_archiver(tmp_path):
    src = tmp_path / "build.log"
    src.write_text("hello\n")
    dest_dir = tmp_path / "archive"
    archive = ci_run.make_directory_archiver(str(dest_dir), "http://logs.example")

    url = archive("abc", str(src))
    assert url == "http://logs.example/abc.log"
    assert (dest_dir / "abc.log").read_text() == "hello\n"


def test_dag_payload():
    # Fake the small slice of the spec API that dag_payload uses.
    class FakeSpec:
        def __init__(self, h, name, version, deps=()):
            self._h, self.name, self._deps = h, name, deps
            self.version = version

        def dag_hash(self, length=None):
            return self._h

        def dependencies(self):
            return list(self._deps)

        def traverse(self):
            seen = {}
            stack = [self]
            while stack:
                s = stack.pop()
                if s._h not in seen:
                    seen[s._h] = s
                    stack.extend(s._deps)
            return seen.values()

    child = FakeSpec("h1", "zlib", "1.3")
    root = FakeSpec("h2", "cmake", "3.2", deps=[child])
    nodes, edges = ci_run.dag_payload([root])

    assert {n["dag_hash"] for n in nodes} == {"h1", "h2"}
    assert edges == [["h2", "h1"]]


def test_html_escape():
    assert ci_run._html_escape(b"a<b>&c") == b"a&lt;b&gt;&amp;c"


def test_log_server_streams_native_html(tmp_path):
    log = tmp_path / "h1.log"
    log.write_bytes(b"first line\n")

    client = FakeClient()
    sink = ci_run.CIEventSink(client, pipeline_id=1, time_fn=lambda: 0.0)
    sink.log_paths["h1"] = str(log)
    sink.states["h1"] = "build"

    server = ci_run.LogServer(sink, host="127.0.0.1", port=0)
    server.start()
    try:
        conn = socket.create_connection(("127.0.0.1", server.port))
        conn.settimeout(2)
        conn.sendall(b"GET /log/h1 HTTP/1.1\r\nHost: x\r\n\r\n")
        time.sleep(0.2)
        # a line split across two writes plus an HTML-significant char, then finish
        log.open("ab").write(b"part ")
        time.sleep(0.3)
        with log.open("ab") as f:
            f.write(b"<end>\nlast\n")
        time.sleep(0.2)
        sink.states["h1"] = "finished"
        time.sleep(0.7)

        data = b""
        try:
            while True:
                piece = conn.recv(4096)
                if not piece:
                    break
                data += piece
        except socket.timeout:
            pass
        conn.close()
    finally:
        server.stop()

    head, _, body = data.partition(b"\r\n\r\n")
    assert b"text/html" in head
    assert b"<pre>" in body  # native streaming page, browser renders incrementally
    # raw bytes forwarded as-is (escaped); the split line is not broken by an extra newline
    assert b"first line\npart &lt;end&gt;\nlast\n" in body
