#!/usr/bin/env python3
"""Verify that OTTO follows the system proxy settings."""

import argparse
import os
import pathlib
import select
import socket
import socketserver
import subprocess
import sys
import tempfile
import threading
import time


class Socks5Handler(socketserver.BaseRequestHandler):
    def recv_exact(self, length):
        data = b""
        while len(data) < length:
            chunk = self.request.recv(length - len(data))
            if not chunk:
                raise ConnectionError("SOCKS5 client disconnected")
            data += chunk
        return data

    def handle(self):  # noqa: D102
        version, methods = self.recv_exact(2)
        if version != 5:
            return
        self.recv_exact(methods)
        self.request.sendall(b"\x05\x00")

        version, command, _reserved, address_type = self.recv_exact(4)
        if version != 5 or command != 1:
            self.request.sendall(b"\x05\x07\x00\x01\x00\x00\x00\x00\x00\x00")
            return

        if address_type == 1:
            host = socket.inet_ntoa(self.recv_exact(4))
        elif address_type == 3:
            host_length = self.recv_exact(1)[0]
            host = self.recv_exact(host_length).decode("idna")
        elif address_type == 4:
            host = socket.inet_ntop(socket.AF_INET6, self.recv_exact(16))
        else:
            self.request.sendall(b"\x05\x08\x00\x01\x00\x00\x00\x00\x00\x00")
            return
        port = int.from_bytes(self.recv_exact(2), "big")

        try:
            target = socket.create_connection((host, port), timeout=5)
        except OSError:
            self.request.sendall(b"\x05\x05\x00\x01\x00\x00\x00\x00\x00\x00")
            return

        self.server.connections.append((host, port))
        with target:
            bound_host, bound_port = target.getsockname()[:2]
            self.request.sendall(
                b"\x05\x00\x00\x01"
                + socket.inet_aton(bound_host)
                + bound_port.to_bytes(2, "big")
            )
            self.relay(target)

    def relay(self, target):
        sockets = [self.request, target]
        while True:
            readable, _writable, _exceptional = select.select(sockets, [], [], 5)
            if not readable:
                return
            for source in readable:
                data = source.recv(64 * 1024)
                if not data:
                    return
                destination = target if source is self.request else self.request
                destination.sendall(data)


class Socks5Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self):
        super().__init__(("127.0.0.1", 0), Socks5Handler)
        self.connections = []


def wait_for_port(path):
    for _attempt in range(100):
        if path.exists() and path.read_text(encoding="ascii").strip():
            return int(path.read_text(encoding="ascii"))
        time.sleep(0.01)
    raise AssertionError("mock server did not start")


def write_fake_gsettings(directory, mode, proxy_port):
    script = directory / "gsettings"
    script.write_text(
        "#!/bin/sh\n"
        "case \"$3\" in\n"
        f"  mode) printf \"'{mode}'\\n\" ;;\n"
        "  use-same-proxy) printf 'false\\n' ;;\n"
        "  ignore-hosts) printf '[]\\n' ;;\n"
        "  host)\n"
        "    case \"$2\" in\n"
        "      org.gnome.system.proxy.socks) printf \"'127.0.0.1'\\n\" ;;\n"
        "      *) printf \"''\\n\" ;;\n"
        "    esac\n"
        "    ;;\n"
        "  port)\n"
        "    case \"$2\" in\n"
        f"      org.gnome.system.proxy.socks) printf 'uint32 {proxy_port}\\n' ;;\n"
        "      *) printf 'uint32 0\\n' ;;\n"
        "    esac\n"
        "    ;;\n"
        "  *) exit 1 ;;\n"
        "esac\n",
        encoding="utf-8",
    )
    script.chmod(0o700)


def run_otto(binary, workspace, environment, question):
    result = subprocess.run(
        [binary, "--no-agent", question],
        cwd=workspace,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=15,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"otto exited with {result.returncode}: "
            f"stdout={result.stdout!r} stderr={result.stderr!r}"
        )
    expected = f"mock: {question}\n".encode()
    if result.stdout != expected:
        raise AssertionError(f"unexpected response: {result.stdout!r}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    args = parser.parse_args()
    binary = str(pathlib.Path(args.binary).resolve())
    root = pathlib.Path(__file__).resolve().parents[1]
    if sys.platform != "linux":
        print("system proxy test skipped outside Linux")
        return

    with tempfile.TemporaryDirectory(prefix="otto-proxy-test-") as directory:
        workspace = pathlib.Path(directory)
        port_file = workspace / "port"
        capture_file = workspace / "request.json"
        mock = subprocess.Popen(
            [
                sys.executable,
                str(root / "tests/mock_server.py"),
                "--port-file",
                str(port_file),
                "--capture",
                str(capture_file),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        proxy = Socks5Server()
        proxy_thread = threading.Thread(target=proxy.serve_forever, daemon=True)
        proxy_thread.start()

        try:
            mock_port = wait_for_port(port_file)
            fake_bin = workspace / "bin"
            fake_bin.mkdir()
            write_fake_gsettings(fake_bin, "manual", proxy.server_address[1])
            config = workspace / "config"
            config.write_text(
                "name=Mock\n"
                f"baseurl=http://127.0.0.1:{mock_port}/v1/chat/completions\n"
                "apikey=test-key\n"
                "model=test-model\n",
                encoding="utf-8",
            )
            environment = os.environ.copy()
            for key in (
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "http_proxy",
                "https_proxy",
                "NO_PROXY",
                "no_proxy",
            ):
                environment.pop(key, None)
            environment.update(
                {
                    "ALL_PROXY": "",
                    "all_proxy": "",
                    "OTTO_CONFIG": str(config),
                    "OTTO_MODE_DIR": str(root / "prompts"),
                    "PATH": f"{fake_bin}{os.pathsep}{environment.get('PATH', '')}",
                }
            )
            run_otto(binary, workspace, environment, "system proxy works")
            if not proxy.connections:
                raise AssertionError("request did not pass through the SOCKS5 proxy")

            connections_before_disable = len(proxy.connections)
            write_fake_gsettings(fake_bin, "none", proxy.server_address[1])
            environment["ALL_PROXY"] = "socks5h://127.0.0.1:1"
            run_otto(binary, workspace, environment, "system proxy disabled")
            if len(proxy.connections) != connections_before_disable:
                raise AssertionError("disabled system proxy should not be used")
        finally:
            proxy.shutdown()
            proxy.server_close()
            proxy_thread.join(timeout=2)
            mock.terminate()
            try:
                mock.wait(timeout=2)
            except subprocess.TimeoutExpired:
                mock.kill()
                mock.wait()

    print("system proxy follow/disable support passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:  # pragma: no cover - test harness output
        print(f"system proxy support failed: {error}", file=sys.stderr)
        sys.exit(1)
