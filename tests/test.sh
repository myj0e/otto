#!/bin/sh
set -eu

binary=${1:-./otto}
test_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/otto-test.XXXXXX")
server_pid=""

cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$temporary_directory"
}
trap cleanup EXIT INT TERM

help_output=$($binary --help)
printf '%s\n' "$help_output" | rg -q -- 'OTTO \(One-time.Talk once\)'
printf '%s\n' "$help_output" | rg -q -- --config
if "$binary" -mode otto >/dev/null 2>&1; then
    echo "single-dash -mode should not be accepted" >&2
    exit 1
fi

version_output=$($binary --version)
printf '%s\n' "$version_output" | rg -q '^otto [0-9]+\.[0-9]+\.[0-9]+$'

python3 "$test_root/tests/test_interactive.py" "$binary"

port_file="$temporary_directory/port"
capture_file="$temporary_directory/request.json"
server_log="$temporary_directory/server.log"
python3 "$test_root/tests/mock_server.py" \
    --port-file "$port_file" \
    --capture "$capture_file" \
    >"$server_log" 2>&1 &
server_pid=$!

attempt=0
while [ ! -s "$port_file" ]; do
    attempt=$((attempt + 1))
    if [ "$attempt" -gt 100 ]; then
        echo "mock server did not start" >&2
        exit 1
    fi
    sleep 0.01
done
port=$(sed -n '1p' "$port_file")

config_file="$temporary_directory/config"
OTTO_CONFIG="$config_file" "$binary" --config \
    --name SiliconFlow \
    --baseurl "http://127.0.0.1:$port/v1/chat/completions" \
    --apikey test-key \
    --model test-model \
    >"$temporary_directory/config-output"

test "$(stat -c '%a' "$config_file")" = 600
rg -q '^name=SiliconFlow$' "$config_file"
rg -q '^model=test-model$' "$config_file"
if rg -q 'test-key' "$temporary_directory/config-output"; then
    echo "API key leaked to stdout" >&2
    exit 1
fi

answer=$(env \
    HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
    http_proxy= https_proxy= all_proxy= \
    NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
    OTTO_CONFIG="$config_file" "$binary" 你好 世界)
test "$answer" = 'mock: 你好 世界'
rg -q '你好 世界' "$capture_file"
rg -q 'test-model' "$capture_file"
rg -q '"stream":true' "$capture_file"
rg -q '所有请求都会加载' "$capture_file"

mode_directory="$temporary_directory/modes"
mkdir -p "$mode_directory"
cp "$test_root/system.md" "$mode_directory/system.md"
cp "$test_root/otto.md" "$mode_directory/otto.md"
printf '%s\n' 'test-mode-system-prompt' >"$mode_directory/test.md"

otto_mode_answer=$(env \
    HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
    http_proxy= https_proxy= all_proxy= \
    NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
    OTTO_CONFIG="$config_file" OTTO_MODE_DIR="$mode_directory" \
    "$binary" --mode otto)
rg -q '^otto$' "$temporary_directory/active_mode"

otto_answer=$(env \
    HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
    http_proxy= https_proxy= all_proxy= \
    NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
    OTTO_CONFIG="$config_file" OTTO_MODE_DIR="$mode_directory" \
    "$binary" 你好 世界)
test "$otto_answer" = 'mock: 你好 世界'
rg -q '所有请求都会加载' "$capture_file"
rg -q '嘴臭' "$capture_file"

test_mode_answer=$(env \
    HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
    http_proxy= https_proxy= all_proxy= \
    NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
    OTTO_CONFIG="$config_file" OTTO_MODE_DIR="$mode_directory" \
    "$binary" --mode test)
rg -q '^test$' "$temporary_directory/active_mode"

mode_answer=$(env \
    HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
    http_proxy= https_proxy= all_proxy= \
    NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
    OTTO_CONFIG="$config_file" OTTO_MODE_DIR="$mode_directory" \
    "$binary" 你好 世界)
test "$mode_answer" = 'mock: 你好 世界'
rg -q '所有请求都会加载' "$capture_file"
rg -q 'test-mode-system-prompt' "$capture_file"

raw_answer=$(env \
    HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
    http_proxy= https_proxy= all_proxy= \
    NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
    OTTO_CONFIG="$config_file" OTTO_MODE_DIR="$mode_directory" \
    "$binary" --mode -- 你好 世界)
test "$raw_answer" = 'mock: 你好 世界'
if rg -q 'test-mode-system-prompt' "$capture_file"; then
    echo "one-shot mode without an optional mode should omit the mode prompt" >&2
    exit 1
fi
rg -q '所有请求都会加载' "$capture_file"

clear_mode_answer=$(env \
    OTTO_CONFIG="$config_file" "$binary" --mode)
test -n "$clear_mode_answer"
test ! -s "$temporary_directory/active_mode"

bare_answer=$(env \
    HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
    http_proxy= https_proxy= all_proxy= \
    NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
    OTTO_CONFIG="$config_file" OTTO_MODE_DIR="$mode_directory" \
    "$binary" 你好 世界)
test "$bare_answer" = 'mock: 你好 世界'
rg -q '所有请求都会加载' "$capture_file"
if rg -q 'test-mode-system-prompt' "$capture_file"; then
    echo "cleared mode should omit the optional mode prompt" >&2
    exit 1
fi

if env \
    HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= \
    http_proxy= https_proxy= all_proxy= \
    NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost \
    OTTO_CONFIG="$config_file" OTTO_MODE_DIR="$mode_directory" \
    "$binary" --mode missing 你好 \
    >"$temporary_directory/missing-mode-output" 2>/dev/null; then
    echo "missing named mode should fail" >&2
    exit 1
fi

if OTTO_CONFIG="$temporary_directory/missing-config" "$binary" 你好 \
    >"$temporary_directory/missing-output" 2>/dev/null; then
    echo "missing configuration should fail" >&2
    exit 1
fi

echo "all tests passed"
