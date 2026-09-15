#!/usr/bin/env python3
"""Exercise native web search routing and the third-party fallback."""

import json
import os
import pathlib
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse


def sse_response(handler, event):
    handler.send_response(200)
    handler.send_header("Content-Type", "text/event-stream")
    handler.send_header("Cache-Control", "no-cache")
    handler.end_headers()
    encoded = ("data: " + json.dumps(event) + "\n\n").encode()
    handler.wfile.write(encoded)
    handler.wfile.write(b"data: [DONE]\n\n")
    handler.wfile.flush()


def build_handler(requests):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            parsed = urlparse(self.path)
            if parsed.path != "/fallback":
                self.send_error(404)
                return
            request = {
                "method": "GET",
                "path": parsed.path,
                "query": parse_qs(parsed.query),
            }
            requests.append(request)
            body = {
                "results": [
                    {
                        "title": "Rust fallback",
                        "url": "https://www.rust-lang.org/",
                        "content": "fallback result",
                        "engine": "mock",
                    }
                ]
            }
            encoded = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def do_POST(self):  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length)
            try:
                request = json.loads(body.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError):
                self.send_error(400)
                return

            requests.append({"method": "POST", "path": self.path, "body": request})
            if self.path != "/v1/chat/completions":
                self.send_error(404)
                return
            if self.headers.get("Authorization") != "Bearer test-key":
                self.send_error(401)
                return

            # The mock model service does not implement native search. This
            # must cause the application to use the configured SearXNG route.
            if "web_search_options" in request:
                encoded = json.dumps(
                    {"error": {"message": "web_search_options is unsupported"}}
                ).encode()
                self.send_response(400)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)
                return

            has_tool_result = any(
                message.get("role") == "tool"
                for message in request.get("messages", [])
            )
            if has_tool_result:
                sse_response(
                    self,
                    {
                        "choices": [
                            {
                                "index": 0,
                                "delta": {"role": "assistant", "content": "fallback final"},
                            }
                        ]
                    },
                )
            else:
                sse_response(
                    self,
                    {
                        "choices": [
                            {
                                "index": 0,
                                "delta": {
                                    "tool_calls": [
                                        {
                                            "index": 0,
                                            "id": "call-search",
                                            "type": "function",
                                            "function": {
                                                "name": "websearch",
                                                "arguments": '{"query":"Rust native search","count":3}',
                                            },
                                        }
                                    ]
                                },
                            }
                        ]
                    },
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
        with tempfile.TemporaryDirectory(prefix="otto-websearch-test-") as directory:
            workspace = pathlib.Path(directory)
            prompt_dir = workspace / "prompts"
            prompt_dir.mkdir()
            (prompt_dir / "system.md").write_text("websearch test system\n", encoding="utf-8")

            config = workspace / "config"
            config.write_text(
                "name=Mock\n"
                f"baseurl=http://127.0.0.1:{server.server_port}/v1/chat/completions\n"
                "apikey=test-key\n"
                "model=test-model\n",
                encoding="utf-8",
            )
            search_config = workspace / "search.env"
            search_config.write_text(
                "OTTO_SEARCH_PROVIDER=searxng\n"
                f"OTTO_SEARCH_URL=http://127.0.0.1:{server.server_port}/fallback\n",
                encoding="utf-8",
            )

            environment = os.environ.copy()
            environment.update(
                {
                    "OTTO_CONFIG": str(config),
                    "OTTO_MODE_DIR": str(prompt_dir),
                    "OTTO_SEARCH_CONFIG": str(search_config),
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
            for key in (
                "OTTO_SEARCH_MODE",
                "OTTO_NATIVE_SEARCH",
                "OTTO_NATIVE_SEARCH_PROTOCOL",
                "OTTO_NATIVE_SEARCH_PROVIDER",
            ):
                environment.pop(key, None)

            process = subprocess.run(
                [binary, "搜索 Rust native search"],
                cwd=workspace,
                env=environment,
                input=b"",
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=15,
                check=False,
            )
            if process.returncode != 0:
                raise AssertionError(
                    f"websearch exited with {process.returncode}: "
                    f"stdout={process.stdout!r} stderr={process.stderr!r}"
                )
            if b"fallback final" not in process.stdout:
                raise AssertionError(f"final answer missing: {process.stdout!r}")
            if "[otto] ⚙ 工具调用：T-websearch".encode() not in process.stderr:
                raise AssertionError(
                    f"search route notice was not printed: {process.stderr!r}"
                )

            native_requests = [
                item
                for item in requests
                if item["method"] == "POST"
                and "web_search_options" in item["body"]
            ]
            if len(native_requests) != 1:
                raise AssertionError(f"expected one native search attempt: {requests!r}")

            fallback_requests = [
                item for item in requests if item["method"] == "GET" and item["path"] == "/fallback"
            ]
            if len(fallback_requests) != 1:
                raise AssertionError(f"expected one third-party fallback: {requests!r}")
            if fallback_requests[0]["query"].get("q") != ["Rust native search"]:
                raise AssertionError(f"fallback query was not forwarded: {requests!r}")
    finally:
        server.shutdown()
        thread.join(timeout=2)

    print("websearch native-first fallback passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:  # pragma: no cover - test harness output
        print(f"websearch native-first fallback failed: {error}", file=sys.stderr)
        sys.exit(1)
