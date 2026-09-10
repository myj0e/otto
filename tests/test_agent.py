#!/usr/bin/env python3
"""Exercise the Rust Agent tool-call loop against a local mock endpoint."""

import json
import os
import pathlib
import pty
import select
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def sse_response(handler, events):
    handler.send_response(200)
    handler.send_header("Content-Type", "text/event-stream")
    handler.send_header("Cache-Control", "no-cache")
    handler.end_headers()
    for event in events:
        encoded = ("data: " + json.dumps(event, ensure_ascii=False) + "\n\n").encode()
        for offset in range(0, len(encoded), 5):
            handler.wfile.write(encoded[offset : offset + 5])
            handler.wfile.flush()
    handler.wfile.write(b"data: [DONE]\n\n")
    handler.wfile.flush()


def build_handler(requests):
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
    server = ThreadingHTTPServer(("127.0.0.1", 0), build_handler(requests))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    try:
        with tempfile.TemporaryDirectory(prefix="otto-agent-test-") as directory:
            workspace = pathlib.Path(directory)
            (workspace / "note.txt").write_text("agent secret\n", encoding="utf-8")
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
            authorized = False
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
                        not authorized
                        and "授权选择 [1/2/3]: ".encode() in output
                    ):
                        os.write(master, b"2\n")
                        authorized = True
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=2)
                os.close(master)

            if process.returncode != 0:
                raise AssertionError(
                    f"agent exited with {process.returncode}: {output!r}"
                )
            if not authorized:
                raise AssertionError(f"read authorization prompt was not shown: {output!r}")
            if b"mock agent final" not in output:
                raise AssertionError(f"final answer was not printed: {output!r}")

            if len(requests) != 2:
                raise AssertionError(f"expected two Agent requests, got {len(requests)}")
            first, second = requests
            if not first.get("tools") or first.get("tool_choice") != "auto":
                raise AssertionError("first request did not advertise Agent tools")
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
