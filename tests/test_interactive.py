#!/usr/bin/env python3
"""Exercise the interactive configuration flow through a pseudo-terminal."""

import os
import pathlib
import pty
import select
import subprocess
import sys
import tempfile
import time


def main():
    binary = sys.argv[1]
    with tempfile.TemporaryDirectory(prefix="otto-interactive-test-") as directory:
        config_path = pathlib.Path(directory) / "config"
        environment = os.environ.copy()
        environment["OTTO_CONFIG"] = str(config_path)
        # LeakSanitizer cannot inspect a process attached to a pseudo-terminal
        # on some systems. The non-interactive tests still run with leak checks.
        environment.setdefault("ASAN_OPTIONS", "detect_leaks=0")

        master, slave = pty.openpty()
        process = subprocess.Popen(
            [binary, "--config"],
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=environment,
            close_fds=True,
        )
        os.close(slave)

        responses = [
            (b"Name [Openai]: ", b"SiliconFlow\n"),
            (b"Base URL [https://api.openai.com]: ", b"http://127.0.0.1:1\n"),
            (b"API Key: ", b"test-key\n"),
            (b"Model [gpt-4o-mini]: ", b"test-model\n"),
            ("保存配置 [Y/n]: ".encode(), b"y\n"),
        ]
        output = bytearray()
        response_index = 0
        deadline = time.monotonic() + 5

        while process.poll() is None and time.monotonic() < deadline:
            readable, _, _ = select.select([master], [], [], 0.1)
            if not readable:
                continue
            try:
                output.extend(os.read(master, 4096))
            except OSError:
                break

            while response_index < len(responses) and responses[response_index][0] in output:
                os.write(master, responses[response_index][1])
                response_index += 1

        if process.poll() is None:
            process.terminate()
            process.wait(timeout=2)

        try:
            os.close(master)
        except OSError:
            pass

        if process.returncode != 0:
            raise AssertionError(f"interactive config exited with {process.returncode}: {output!r}")
        if response_index != len(responses):
            raise AssertionError(f"interactive prompts were incomplete: {output!r}")
        if b"test-key" in output:
            raise AssertionError("API key was echoed by the terminal")

        content = config_path.read_text(encoding="utf-8")
        assert "name=SiliconFlow\n" in content
        assert "baseurl=http://127.0.0.1:1\n" in content
        assert "apikey=test-key\n" in content
        assert "model=test-model\n" in content

    print("interactive configuration passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:  # pragma: no cover - test harness output
        print(f"interactive configuration failed: {error}", file=sys.stderr)
        sys.exit(1)
