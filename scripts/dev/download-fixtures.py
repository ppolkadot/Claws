"""Local download fixtures with controlled faults, for tests and benchmarks.

Run:  python scripts/dev/download-fixtures.py [--port 0] [--size-mib 8] [--rate-kib 0]

Everything is served from 127.0.0.1 only and every byte is generated on the fly
(a repeating pattern derived from the offset), so even multi-GiB payloads never sit
in memory or on disk. Routes:

  /hls/list.m3u8           three-segment HLS playlist; segment 1 (the middle one) is 404
  /hls-ok/list.m3u8        the same playlist with every segment present
  /range/file.bin          honours Range requests (206), so downloads can resume
  /no-range/file.bin       ignores Range and always sends the whole body (200)
  /unstable/file.bin       answers 503 to the first N requests, then serves normally
  /reset/file.bin          drops the connection halfway through the first response
  /expired/file.bin        always 403, like an expired signed media link
  /slow/file.bin           serves at a capped rate (--rate-kib, default 256 KiB/s)
  /sha256/<route-name>     hex SHA-256 of what that route serves, for integrity checks
  /faults                  JSON of the active fault settings and request counters

The base URL and fault settings are printed on startup as one JSON line, so test
harnesses can read them from stdout.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CHUNK = 64 * 1024
SEGMENTS = 3


BLOCK = 4096
# Byte i is (i * 31 + i // 4096) & 255. Every 4 KiB block is the same base pattern shifted
# by its block number, so the 256 possible blocks are built once and sliced from.
_BASE = [(i * 31) & 0xFF for i in range(BLOCK)]
_BLOCKS = [bytes((x + shift) & 0xFF for x in _BASE) for shift in range(256)]


def pattern_bytes(offset: int, length: int) -> bytes:
    """Deterministic bytes for [offset, offset + length)."""
    parts = []
    position, end = offset, offset + length
    while position < end:
        block, within = divmod(position, BLOCK)
        take = min(BLOCK - within, end - position)
        parts.append(_BLOCKS[block & 0xFF][within:within + take])
        position += take
    return b"".join(parts)


def pattern_digest(size: int) -> str:
    digest = hashlib.sha256()
    for start in range(0, size, CHUNK):
        digest.update(pattern_bytes(start, min(CHUNK, size - start)))
    return digest.hexdigest()


class Fixtures:
    def __init__(self, size: int, segment_size: int, rate: int, unstable_failures: int):
        self.size = size
        self.segment_size = segment_size
        self.rate = rate
        self.unstable_failures = unstable_failures
        self.lock = threading.Lock()
        self.counters: dict[str, int] = {}
        self._digests: dict[int, str] = {}

    def count(self, route: str) -> int:
        with self.lock:
            self.counters[route] = self.counters.get(route, 0) + 1
            return self.counters[route]

    def digest(self, size: int) -> str:
        with self.lock:
            if size not in self._digests:
                self._digests[size] = pattern_digest(size)
            return self._digests[size]

    def describe(self) -> dict:
        with self.lock:
            return {
                "sizeBytes": self.size,
                "segmentBytes": self.segment_size,
                "slowRateBytesPerSecond": self.rate,
                "unstableFailures": self.unstable_failures,
                "missingHlsSegment": 1,
                "counters": dict(self.counters),
            }


def make_handler(fx: Fixtures):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_args):  # keep test output quiet
            pass

        def send_plain(self, status: int, body: bytes, content_type: str = "text/plain"):
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if self.command != "HEAD":
                self.wfile.write(body)

        def stream(self, start: int, end: int, rate: int = 0, reset_at: int | None = None):
            sent = 0
            began = time.monotonic()
            position = start
            while position < end:
                length = min(CHUNK, end - position)
                if reset_at is not None and position + length > reset_at:
                    self.wfile.write(pattern_bytes(position, max(0, reset_at - position)))
                    self.wfile.flush()
                    self.close_connection = True
                    self.connection.shutdown(2)
                    return
                self.wfile.write(pattern_bytes(position, length))
                position += length
                sent += length
                if rate > 0:
                    ahead = sent / rate - (time.monotonic() - began)
                    if ahead > 0:
                        time.sleep(ahead)

        def serve_file(self, honour_range: bool, rate: int = 0, reset_at: int | None = None):
            size = fx.size
            header = self.headers.get("Range")
            match = re.fullmatch(r"bytes=(\d+)-(\d*)", header or "")
            if honour_range and match:
                start = int(match.group(1))
                end = int(match.group(2)) + 1 if match.group(2) else size
                end = min(end, size)
                if start >= size:
                    self.send_response(416)
                    self.send_header("Content-Range", f"bytes */{size}")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                self.send_response(206)
                self.send_header("Content-Range", f"bytes {start}-{end - 1}/{size}")
            else:
                start, end = 0, size
                self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(end - start))
            if honour_range:
                self.send_header("Accept-Ranges", "bytes")
            self.end_headers()
            if self.command != "HEAD":
                self.stream(start, end, rate, reset_at)

        def playlist(self, prefix: str):
            lines = ["#EXTM3U", "#EXT-X-VERSION:3", "#EXT-X-TARGETDURATION:4", "#EXT-X-MEDIA-SEQUENCE:0"]
            for index in range(SEGMENTS):
                lines += ["#EXTINF:4.0,", f"/{prefix}/seg{index}.ts"]
            lines.append("#EXT-X-ENDLIST")
            self.send_plain(200, ("\n".join(lines) + "\n").encode(), "application/vnd.apple.mpegurl")

        def segment(self, index: int, missing: bool):
            if missing and index == 1:
                return self.send_plain(404, b"missing segment")
            size = fx.segment_size
            self.send_response(200)
            self.send_header("Content-Type", "video/mp2t")
            self.send_header("Content-Length", str(size))
            self.end_headers()
            if self.command != "HEAD":
                self.stream(index * size, index * size + size)

        def do_HEAD(self):
            self.do_GET()

        def do_GET(self):
            path = self.path.split("?", 1)[0]
            fx.count(path)
            if path == "/faults":
                return self.send_plain(200, json.dumps(fx.describe()).encode(), "application/json")
            if path in ("/hls/list.m3u8", "/hls-ok/list.m3u8"):
                return self.playlist(path.split("/")[1])
            seg = re.fullmatch(r"/(hls|hls-ok)/seg(\d+)\.ts", path)
            if seg:
                return self.segment(int(seg.group(2)), missing=seg.group(1) == "hls")
            # Any file name works under the file routes (/range/a.bin, /range/b.bin, ...),
            # so tests can produce distinct downloads of the same bytes.
            route = re.fullmatch(r"/([a-z-]+)/[A-Za-z0-9_.-]+\.bin", path)
            kind = route.group(1) if route else None
            if kind == "range":
                return self.serve_file(True)
            if kind == "no-range":
                return self.serve_file(False)
            if kind == "slow":
                return self.serve_file(True, rate=fx.rate)
            if kind == "unstable":
                if fx.count("unstable-attempts") <= fx.unstable_failures:
                    return self.send_plain(503, b"try again later")
                return self.serve_file(True)
            if kind == "reset":
                first = fx.count("reset-attempts") == 1
                return self.serve_file(True, reset_at=fx.size // 2 if first else None)
            if kind == "expired":
                return self.send_plain(403, b"link expired")
            digest = re.fullmatch(r"/sha256/([a-z-]+)", path)
            if digest:
                size = fx.segment_size * SEGMENTS if digest.group(1) == "hls-ok" else fx.size
                return self.send_plain(200, fx.digest(size).encode())
            self.send_plain(404, b"unknown fixture route")

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=0, help="0 picks a free port")
    parser.add_argument("--size-mib", type=float, default=8, help="payload size of the file routes")
    parser.add_argument("--segment-kib", type=int, default=256, help="size of each HLS segment")
    parser.add_argument("--rate-kib", type=int, default=256, help="rate cap of /slow, KiB/s")
    parser.add_argument("--unstable-failures", type=int, default=2, help="503s before /unstable succeeds")
    args = parser.parse_args()

    fx = Fixtures(int(args.size_mib * 1024 * 1024), args.segment_kib * 1024, args.rate_kib * 1024, args.unstable_failures)
    class QuietServer(ThreadingHTTPServer):
        # Clients closing early (cancel, the reset fixture) are expected, not errors.
        def handle_error(self, request, client_address):
            pass

    server = QuietServer(("127.0.0.1", args.port), make_handler(fx))
    server.daemon_threads = True
    base = f"http://127.0.0.1:{server.server_address[1]}"
    print(json.dumps({"baseUrl": base, "faults": fx.describe()}), flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
