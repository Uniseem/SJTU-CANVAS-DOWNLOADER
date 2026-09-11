#!/usr/bin/env python3
"""A local media server for the demo school (SJTU_CANVAS_FAKE_MEDIA).

    python engine/tests/mock_media.py 8765 [--rate BYTES_PER_SECOND]

GET /media/<name>?size=N[&rate=R] returns N bytes of a fixed pattern with
Range, ETag and Content-Length, throttled to R bytes per second (default
--rate), so downloads take long enough to pause, cancel or watch.
"""

from __future__ import annotations

import argparse
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

CHUNK = 64 * 1024


PERIOD = bytes((index * 31 + 7) % 251 for index in range(251))


def pattern(start: int, end: int) -> bytes:
    """Bytes start..end of the pattern ((i * 31 + 7) % 251) the tests check."""
    length = max(0, end - start)
    offset = start % len(PERIOD)
    repeats = (offset + length) // len(PERIOD) + 1
    return (PERIOD * repeats)[offset:offset + length]


class MediaServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, port: int, rate: int):
        super().__init__(("127.0.0.1", port), Handler)
        self.rate = rate
        self.ranges: list[str | None] = []
        self.lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    server: MediaServer

    def log_message(self, format: str, *args) -> None:  # noqa: A002 - quiet
        pass

    def do_GET(self) -> None:  # noqa: N802
        url = urlparse(self.path)
        match = re.fullmatch(r"/media/([A-Za-z0-9_.-]+)", url.path)
        query = parse_qs(url.query)
        if not match or "size" not in query:
            self.send_error(404)
            return
        size = int(query["size"][0])
        rate = int(query.get("rate", [self.server.rate])[0])
        range_header = self.headers.get("Range")
        with self.server.lock:
            self.server.ranges.append(range_header)
        start = 0
        if range_header:
            requested = re.fullmatch(r"bytes=(\d+)-", range_header)
            if requested:
                start = int(requested.group(1))
        if start >= size and size > 0:
            self.send_response(416)
            self.send_header("Content-Range", f"bytes */{size}")
            self.end_headers()
            return
        self.send_response(206 if start else 200)
        self.send_header("Content-Type", "video/mp4")
        self.send_header("Content-Length", str(size - start))
        self.send_header("Accept-Ranges", "bytes")
        self.send_header("ETag", f'"{match.group(1)}-{size}"')
        if start:
            self.send_header("Content-Range", f"bytes {start}-{size - 1}/{size}")
        self.end_headers()
        offset = start
        began = time.monotonic()
        try:
            while offset < size:
                end = min(offset + CHUNK, size)
                self.wfile.write(pattern(offset, end))
                offset = end
                if rate > 0:
                    ahead = (offset - start) / rate - (time.monotonic() - began)
                    if ahead > 0:
                        time.sleep(ahead)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass


def start(port: int = 0, rate: int = 4_000_000) -> MediaServer:
    server = MediaServer(port, rate)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("port", type=int, nargs="?", default=8765)
    parser.add_argument("--rate", type=int, default=4_000_000, help="bytes per second (0 = unthrottled)")
    args = parser.parse_args()
    server = MediaServer(args.port, args.rate)
    print(f"mock media on http://127.0.0.1:{server.server_address[1]} ({args.rate} B/s)", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        sys.exit(0)


if __name__ == "__main__":
    main()
