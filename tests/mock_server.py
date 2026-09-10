#!/usr/bin/env python3
"""Small local OpenAI-compatible endpoint used by make test."""

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def build_handler(capture_path):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):  # noqa: N802
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length)
            if capture_path:
                with open(capture_path, "wb") as capture:
                    capture.write(body)

            if self.path != "/v1/chat/completions":
                self.send_error(404)
                return

            if self.headers.get("Authorization") != "Bearer test-key":
                self.send_error(401)
                return

            try:
                request = json.loads(body.decode("utf-8"))
                messages = request["messages"]
                prompt = next(
                    message["content"]
                    for message in reversed(messages)
                    if message.get("role") == "user"
                )
            except (UnicodeDecodeError, json.JSONDecodeError, KeyError, IndexError, TypeError):
                self.send_error(400)
                return
            except StopIteration:
                self.send_error(400)
                return

            response = {
                "choices": [
                    {"message": {"role": "assistant", "content": f"mock: {prompt}"}}
                ]
            }
            encoded = json.dumps(response, ensure_ascii=False).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def log_message(self, _format, *_args):
            return

    return Handler


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port-file", required=True)
    parser.add_argument("--capture", required=True)
    args = parser.parse_args()

    server = ThreadingHTTPServer(
        ("127.0.0.1", 0),
        build_handler(args.capture),
    )
    with open(args.port_file, "w", encoding="ascii") as port_file:
        port_file.write(str(server.server_port))
        port_file.flush()
    server.serve_forever()


if __name__ == "__main__":
    main()
