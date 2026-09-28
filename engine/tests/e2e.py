#!/usr/bin/env python3
"""End-to-end test of a debug sjtu-canvas-engine over JSON-RPC.

    cargo build --manifest-path engine/Cargo.toml
    python engine/tests/e2e.py --engine engine/target/debug/sjtu-canvas-engine[.exe]

Runs against the demo school (SJTU_CANVAS_FAKE_SCHOOL) and a local media
server, so no SJTU account is needed: login, courses, lessons, sizes,
downloads with pause/resume and cancel, settings, and a restart in the
middle of a transfer (the saved login and the partial file carry over).
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import queue
import secrets
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mock_media  # noqa: E402


class EngineError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(f"{code}: {message}")
        self.code = code


class Engine:
    def __init__(self, executable: Path, data_dir: Path, env: dict[str, str]):
        self.process = subprocess.Popen(
            [str(executable), "--data-dir", str(data_dir)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
        )
        self.next_id = 0
        self.pending: dict[int, queue.Queue] = {}
        self.notifications: "queue.Queue[dict]" = queue.Queue()
        self.lock = threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        assert self.process.stdout is not None
        for raw in self.process.stdout:
            message = json.loads(raw.decode("utf-8"))
            if "id" in message and message["id"] is not None:
                with self.lock:
                    waiting = self.pending.pop(message["id"], None)
                if waiting:
                    waiting.put(message)
            else:
                self.notifications.put(message)

    def call(self, method: str, params: dict | None = None, timeout: float = 30):
        with self.lock:
            self.next_id += 1
            request_id = self.next_id
            answer: queue.Queue = queue.Queue()
            self.pending[request_id] = answer
        line = json.dumps({"id": request_id, "method": method, "params": params or {}}, ensure_ascii=False)
        assert self.process.stdin is not None
        self.process.stdin.write(line.encode("utf-8") + b"\n")
        self.process.stdin.flush()
        message = answer.get(timeout=timeout)
        if "error" in message:
            raise EngineError(message["error"]["code"], message["error"]["message"])
        return message["result"]

    def wait(self, method: str, check=lambda params: True, timeout: float = 30) -> dict:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                message = self.notifications.get(timeout=max(0.05, deadline - time.monotonic()))
            except queue.Empty:
                break
            if message.get("method") == method and check(message.get("params")):
                return message["params"]
        raise AssertionError(f"no {method} notification within {timeout} s")

    def close(self) -> None:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.process.kill()
            raise


def expect(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)
    print(f"  ok  {message}")


def wait_status(engine: Engine, ids: set[str], status: str, timeout: float = 60) -> dict[str, dict]:
    found: dict[str, dict] = {}
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline and len(found) < len(ids):
        for item in engine.call("downloads.list")["items"]:
            if item["id"] in ids and item["status"] == status:
                found[item["id"]] = item
            if item["id"] in ids and item["status"] == "failed":
                raise AssertionError(f"download failed: {item['error']}")
        time.sleep(0.2)
    if len(found) < len(ids):
        raise AssertionError(f"downloads did not reach {status}: {engine.call('downloads.list')['items']}")
    return found


def check_file(path: str, size: int) -> None:
    data = Path(path).read_bytes()
    expect(len(data) == size, f"{Path(path).name} has {size} bytes")
    expect(data == mock_media.pattern(0, size), f"{Path(path).name} has the served content")
    expect(not Path(path + ".part").exists(), f"{Path(path).name} left no partial file")


def main() -> None:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path)
    args = parser.parse_args()

    work = args.work_dir or Path(tempfile.mkdtemp(prefix="sjtu-canvas-e2e-"))
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    data_dir = work / "数据"
    downloads_folder = work / "Downloads"
    media = mock_media.start(rate=6_000_000)
    env = dict(
        os.environ,
        SJTU_CANVAS_TEST_MODE="1",
        SJTU_CANVAS_FAKE_SCHOOL="1",
        SJTU_CANVAS_FAKE_MEDIA=f"http://127.0.0.1:{media.server_address[1]}",
        SJTU_CANVAS_FAKE_LOGIN_SECONDS="1",
    )
    key = base64.b64encode(secrets.token_bytes(32)).decode()
    initialize = {"session_key": key, "downloads_folder": str(downloads_folder)}

    print("engine start")
    engine = Engine(args.engine, data_dir, env)
    result = engine.call("engine.initialize", initialize)
    expect(result["protocol"] == 1, "protocol version 1")
    expect(result["settings"]["fake_school"], "demo school is active")
    expect(not result["account"]["authenticated"], "starts signed out")
    expect(result["settings"]["preferences"]["download_dir"] == str(downloads_folder / "SJTU Canvas"),
           "downloads default to <Downloads>/SJTU Canvas")
    try:
        engine.call("courses.list")
        raise AssertionError("courses.list worked without a login")
    except EngineError as error:
        expect(error.code == "unauthorized", "courses need a login")

    print("login")
    status = engine.call("login.start")
    expect(status["state"] == "preparing", "login starts")
    waiting = engine.wait("login.status", lambda params: params["state"] == "waiting")
    expect(base64.b64decode(waiting["qr_png"]).startswith(b"\x89PNG"), "a QR image is pushed")
    account = engine.wait("account.changed", lambda params: params["authenticated"])
    expect(account["profile"]["name"] == "演示同学", "signed in as the demo student")
    expect(account["persisted"], "the login is saved with the host key")
    engine.wait("login.status", lambda params: params["state"] == "authorized")

    print("courses")
    courses = engine.call("courses.list")["courses"]
    expect(len(courses) == 6, "six demo courses")
    lessons = engine.call("courses.lessons", {"course_id": "87954"})["lessons"]
    expect(len(lessons) == 10 and not lessons[-1]["available"], "ten lessons, the newest not open yet")
    files = engine.call("courses.files", {"course_id": "87954"})["files"]
    expect(len(files) == 5, "five course files")
    sizes = engine.call("lessons.sizes", {"course_id": "87954", "lesson_id": lessons[0]["video_id"],
                                          "tracks": ["slides", "composite"]})
    expect(sizes["tracks"]["slides"]["status"] == "ready", "slides size is known")
    expect(sizes["tracks"]["composite"]["status"] == "missing", "no composite view")

    print("downloads")
    lesson = lessons[0]
    created = engine.call("downloads.create", {"items": [
        {"kind": "video", "course_id": "87954", "course_name": "机器学习与数据挖掘（2026 秋）",
         "lesson_id": lesson["video_id"], "title": lesson["title"], "begin_time": lesson["begin_time"],
         "track": track, "size": sizes["tracks"]["slides"]["size"] if track == "slides" else None}
        for track in ("slides", "teacher")
    ] + [
        {"kind": "file", "course_id": "87954", "course_name": "机器学习与数据挖掘（2026 秋）",
         "file_id": file["id"], "title": file["display_name"], "size": file["size"]}
        for file in files[:2]
    ]})
    ids = {item["id"] for item in created["created"]}
    expect(len(ids) == 4 and not created["skipped"], "four downloads queued")
    video_id = created["created"][0]["id"]
    engine.wait("download.progress", lambda params: params["id"] == video_id and params["received"] > 0)
    paused = engine.call("downloads.pause", {"id": video_id})
    expect(paused["status"] == "paused", "a running download pauses")
    time.sleep(0.5)
    before = engine.call("downloads.get", {"id": video_id})
    expect(0 < before["received"] < (before["total"] or 1), "the paused download kept its progress")
    engine.call("downloads.resume", {"id": video_id})
    done = wait_status(engine, ids, "completed", timeout=90)
    for item in done.values():
        check_file(item["file_path"], item["total"])
    expect(any(value and value.startswith("bytes=") for value in media.ranges), "the resume used a Range request")
    counts = engine.call("downloads.list", {"filter": "completed"})["counts"]
    expect(counts["completed"] == 4 and counts["active"] == 0, "counts show four completed")
    syllabus = {"kind": "file", "course_id": "87954", "course_name": "机器学习与数据挖掘（2026 秋）",
                "file_id": files[0]["id"], "title": files[0]["display_name"]}
    again = engine.call("downloads.create", {"items": [syllabus]})
    expect(not again["created"] and again["skipped"][0]["reason"] == "downloaded",
           "a finished download is not repeated")
    first = next(item for item in done.values()
                 if item["kind"] == "file" and item["resource_id"] == str(files[0]["id"]))
    engine.call("downloads.remove", {"id": first["id"]})
    again = engine.call("downloads.create", {"items": [syllabus]})
    second = wait_status(engine, {again["created"][0]["id"]}, "completed")
    expect(next(iter(second.values()))["file_path"].endswith("课程大纲 (2).pdf"),
           "a removed task downloaded again gets a numbered copy")

    print("cancel")
    cancelled = engine.call("downloads.create", {"items": [
        {"kind": "video", "course_id": "87954", "course_name": "机器学习与数据挖掘（2026 秋）",
         "lesson_id": lessons[1]["video_id"], "title": lessons[1]["title"], "track": "slides"}]})
    cancel_id = cancelled["created"][0]["id"]
    engine.wait("download.progress", lambda params: params["id"] == cancel_id)
    part = engine.call("downloads.get", {"id": cancel_id})["file_path"] + ".part"
    info = engine.call("downloads.cancel", {"id": cancel_id})
    expect(info["status"] == "cancelled" and info["file_path"] is None, "a download cancels")
    time.sleep(0.5)
    expect(not Path(part).exists(), "the partial file of a cancelled download is deleted")
    engine.call("downloads.remove", {"id": cancel_id})

    print("settings")
    settings = engine.call("settings.get")
    preferences = dict(settings["preferences"], concurrency=5, proxy={"mode": "direct"},
                       default_tracks=["teacher"])
    updated = engine.call("settings.update", {"preferences": preferences})
    expect(updated["preferences"]["concurrency"] == 5, "concurrency saved")
    expect(updated["preferences"]["proxy"] == {"mode": "direct"}, "proxy saved")
    try:
        engine.call("settings.update", {"preferences": dict(preferences, concurrency=99)})
        raise AssertionError("an invalid concurrency was accepted")
    except EngineError as error:
        expect(error.code == "user_error", "invalid settings are refused")

    print("restart during a transfer")
    long_download = engine.call("downloads.create", {"items": [
        {"kind": "video", "course_id": "88148", "course_name": "无线通信原理",
         "lesson_id": "demo-88148-03", "title": "第 03 讲", "begin_time": "2026-09-03 08:00:00",
         "track": "teacher"}]})
    long_id = long_download["created"][0]["id"]
    engine.wait("download.progress", lambda params: params["id"] == long_id and params["received"] > 2_000_000)
    ranges_before = len(media.ranges)
    engine.close()
    expect(engine.process.returncode == 0, "the engine exits when stdin closes")

    engine = Engine(args.engine, data_dir, env)
    result = engine.call("engine.initialize", initialize)
    expect(result["account"]["authenticated"], "the saved login is restored")
    expect(result["settings"]["preferences"]["concurrency"] == 5, "settings survive a restart")
    done = wait_status(engine, {long_id}, "completed", timeout=90)
    check_file(done[long_id]["file_path"], done[long_id]["total"])
    resumed = [value for value in media.ranges[ranges_before:] if value]
    expect(bool(resumed) and resumed[0] != "bytes=0-", "the interrupted download resumed from its partial file")

    print("logout")
    engine.call("account.logout")
    account = engine.wait("account.changed", lambda params: not params["authenticated"])
    expect(not account["authenticated"], "signed out")
    engine.close()

    other_key = Engine(args.engine, data_dir, env)
    result = other_key.call("engine.initialize", {"session_key": base64.b64encode(secrets.token_bytes(32)).decode()})
    expect(not result["account"]["authenticated"], "a logout is not undone by a restart")
    other_key.close()
    media.shutdown()
    if not args.work_dir:
        shutil.rmtree(work, ignore_errors=True)
    print("e2e: all checks passed")


if __name__ == "__main__":
    main()
