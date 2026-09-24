#!/usr/bin/env python3
"""Exercise the Rust Agent tool-call loop against a local mock endpoint."""

import json
import fcntl
import os
import pathlib
import pty
import select
import struct
import subprocess
import sys
import tempfile
import threading
import time
import termios
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def sse_response(handler, events, first_event_sent=None, continue_event=None):
    handler.send_response(200)
    handler.send_header("Content-Type", "text/event-stream")
    handler.send_header("Cache-Control", "no-cache")
    handler.end_headers()
    for index, event in enumerate(events):
        encoded = ("data: " + json.dumps(event, ensure_ascii=False) + "\n\n").encode()
        for offset in range(0, len(encoded), 5):
            handler.wfile.write(encoded[offset : offset + 5])
            handler.wfile.flush()
        if index == 0 and first_event_sent is not None:
            first_event_sent.set()
            if continue_event is not None and not continue_event.wait(timeout=5):
                raise RuntimeError("timed out waiting for streamed output")
    handler.wfile.write(b"data: [DONE]\n\n")
    handler.wfile.flush()


def build_handler(requests, first_event_sent, continue_event):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length)
            try:
                request = json.loads(body.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError):
                self.send_error(400)
                return

            requests.append(request)
            if self.path != "/v1/chat/completions":
                self.send_error(404)
                return
            if self.headers.get("Authorization") != "Bearer test-key":
                self.send_error(401)
                return
            if request.get("stream") is not True:
                self.send_error(400)
                return

            has_tool_result = any(
                message.get("role") == "tool" for message in request.get("messages", [])
            )
            if not has_tool_result:
                sse_response(
                    self,
                    [
                        {
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "role": "assistant",
                                        "content": "先读取这个文件，再根据内容回答。",
                                        "tool_calls": [
                                            {
                                                "index": 0,
                                                "id": "call_",
                                                "type": "function",
                                                "function": {
                                                    "name": "re",
                                                    "arguments": '{"pa',
                                                },
                                            }
                                        ],
                                    },
                                }
                            ]
                        },
                        {
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "tool_calls": [
                                            {
                                                "index": 0,
                                                "id": "ad",
                                                "function": {
                                                    "name": "ad",
                                                    "arguments": 'th":"note.txt"}',
                                                },
                                            }
                                        ]
                                    },
                                }
                            ]
                        },
                    ],
                    first_event_sent,
                    continue_event,
                )
            else:
                sse_response(
                    self,
                    [
                        {
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "role": "assistant",
                                        "content": "mock agent final",
                                    },
                                }
                            ]
                        }
                    ],
                )

        def log_message(self, _format, *_args):
            return

    return Handler


def main():
    binary = str(pathlib.Path(sys.argv[1]).resolve())
    requests = []
    first_event_sent = threading.Event()
    continue_event = threading.Event()
    server = ThreadingHTTPServer(
        ("127.0.0.1", 0),
        build_handler(requests, first_event_sent, continue_event),
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    try:
        with tempfile.TemporaryDirectory(prefix="otto-agent-test-") as directory:
            workspace = pathlib.Path(directory)
            (workspace / "note.txt").write_text("agent secret\n", encoding="utf-8")
            (workspace / "AGENT.md").write_text(
                "workspace instruction\n", encoding="utf-8"
            )
            prompt_dir = workspace / "prompts"
            prompt_dir.mkdir()
            (prompt_dir / "system.md").write_text("agent test system\n", encoding="utf-8")
            config = workspace / "config"
            config.write_text(
                "name=Mock\n"
                f"baseurl=http://127.0.0.1:{server.server_port}/v1/chat/completions\n"
                "apikey=test-key\n"
                "model=test-model\n",
                encoding="utf-8",
            )
            config.chmod(0o600)

            environment = os.environ.copy()
            environment.update(
                {
                    "OTTO_CONFIG": str(config),
                    "OTTO_MODE_DIR": str(prompt_dir),
                    "HTTP_PROXY": "",
                    "HTTPS_PROXY": "",
                    "ALL_PROXY": "",
                    "http_proxy": "",
                    "https_proxy": "",
                    "all_proxy": "",
                    "NO_PROXY": "127.0.0.1,localhost",
                    "no_proxy": "127.0.0.1,localhost",
                }
            )

            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
            process = subprocess.Popen(
                [binary, "读取 note.txt"],
                stdin=slave,
                stdout=slave,
                stderr=slave,
                cwd=workspace,
                env=environment,
                close_fds=True,
            )
            os.close(slave)

            output = bytearray()
            streamed_before_first_response_finished = False
            deadline = time.monotonic() + 10
            try:
                while process.poll() is None and time.monotonic() < deadline:
                    readable, _, _ = select.select([master], [], [], 0.1)
                    if readable:
                        try:
                            output.extend(os.read(master, 4096))
                        except OSError:
                            break
                    if (
                        first_event_sent.is_set()
                        and not streamed_before_first_response_finished
                        and "先读取这个文件，再根据内容回答。".encode() in output
                    ):
                        streamed_before_first_response_finished = True
                        continue_event.set()
            finally:
                continue_event.set()
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=2)
                os.close(master)

            if process.returncode != 0:
                raise AssertionError(
                    f"agent exited with {process.returncode}: {output!r}"
                )
            if "授权选择 [1/2/3]: ".encode() in output:
                raise AssertionError(
                    f"read-only tool unexpectedly requested authorization: {output!r}"
                )
            if b"mock agent final" not in output:
                raise AssertionError(f"final answer was not printed: {output!r}")
            if not streamed_before_first_response_finished:
                raise AssertionError(
                    "model text was not streamed before the first SSE response ended"
                )
            if "先读取这个文件，再根据内容回答。".encode() not in output:
                raise AssertionError(f"streamed model text was not printed: {output!r}")
            if "工具活动".encode() not in output or b"read" not in output:
                raise AssertionError(f"tool summary was not printed: {output!r}")
            if output.count("先读取这个文件，再根据内容回答。".encode()) != 1:
                raise AssertionError(f"model progress text was not shown exactly once: {output!r}")

            if len(requests) != 2:
                raise AssertionError(f"expected two Agent requests, got {len(requests)}")
            first, second = requests
            if not first.get("tools") or first.get("tool_choice") != "auto":
                raise AssertionError("first request did not advertise Agent tools")
            system_message = next(
                message
                for message in first["messages"]
                if message.get("role") == "system"
            )
            if "workspace instruction" not in system_message.get("content", ""):
                raise AssertionError("AGENT.md instructions were not sent to the model")
            assistant = next(
                message
                for message in second["messages"]
                if message.get("role") == "assistant"
            )
            tool_result = next(
                message for message in second["messages"] if message.get("role") == "tool"
            )
            assert assistant["tool_calls"][0]["function"]["name"] == "read"
            assert assistant["tool_calls"][0]["function"]["arguments"] == (
                '{"path":"note.txt"}'
            )
            assert "agent secret" in tool_result["content"]

            limited_environment = environment.copy()
            limited_environment["OTTO_MAX_AGENT_ROUNDS"] = "1"
            limited = subprocess.run(
                [binary, "读取 note.txt"],
                cwd=workspace,
                env=limited_environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            if limited.returncode != 5:
                raise AssertionError(
                    f"configured Agent round limit was not enforced: {limited.returncode}, "
                    f"stderr={limited.stderr!r}"
                )
            if "最大轮数".encode() not in limited.stderr:
                raise AssertionError(
                    f"configured Agent round limit returned the wrong error: {limited.stderr!r}"
                )
            if len(requests) != 3:
                raise AssertionError(
                    f"one-round Agent should make one request, got {len(requests)} total"
                )

            unlimited_environment = environment.copy()
            unlimited_environment["OTTO_MAX_AGENT_ROUNDS"] = "0"
            unlimited = subprocess.run(
                [binary, "读取 note.txt"],
                cwd=workspace,
                env=unlimited_environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            if unlimited.returncode != 0:
                raise AssertionError(
                    f"zero Agent round limit should mean unlimited: {unlimited.returncode}, "
                    f"stderr={unlimited.stderr!r}"
                )
            if b"mock agent final" not in unlimited.stdout:
                raise AssertionError(
                    f"unlimited Agent did not finish: {unlimited.stdout!r}"
                )
            if len(requests) != 5:
                raise AssertionError(
                    f"unlimited Agent should make two requests, got {len(requests) - 3}"
                )
    finally:
        server.shutdown()
        thread.join(timeout=2)

    print("agent tool loop passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:  # pragma: no cover - test harness output
        print(f"agent tool loop failed: {error}", file=sys.stderr)
        sys.exit(1)
