#!/bin/sh
set -eu

test_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/otto-install-test.XXXXXX")

cleanup() {
    rm -rf "$temporary_directory"
}
trap cleanup EXIT INT TERM

install_script="$test_root/scripts/install.sh"
uninstall_script="$test_root/scripts/uninstall.sh"

binary=${1:-"$test_root/target/release/otto"}
fail() {
    printf 'install script test: %s\n' "$*" >&2
    exit 1
}

case "$binary" in
    /*) binary_path=$binary ;;
    *) binary_path="$test_root/$binary" ;;
esac

[ -x "$binary_path" ] || fail 'Rust release 可执行文件不存在，请先运行 make'

prefix="$temporary_directory/prefix"
config_dir="$temporary_directory/config"
state_dir="$temporary_directory/state"
mkdir -p "$config_dir"

printf '%s\n' 'user-api-config' >"$config_dir/config"
printf '%s\n' 'user-owned-system-prompt' >"$config_dir/system.md"
printf '%s\n' 'otto' >"$config_dir/active_mode"

OTTO_INSTALL_BINARY="$binary_path" "$install_script" \
    --prefix "$prefix" \
    --config-dir "$config_dir" \
    --state-dir "$state_dir" \
    >"$temporary_directory/install-output"

test -x "$prefix/bin/otto" || fail '可执行文件未安装'
package_version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$test_root/Cargo.toml" | head -n 1)
test "$("$prefix/bin/otto" --version)" = "otto $package_version" || fail '安装的不是正式 Rust 版本'
test -f "$config_dir/otto.md" || fail 'otto.md 未安装'
test -f "$config_dir/jarvis.md" || fail 'jarvis.md 未安装'
test -f "$config_dir/system.md" || fail '已有 system.md 被删除'
test -f "$config_dir/config" || fail '已有 API 配置被删除'
test -f "$config_dir/active_mode" || fail '已有 active_mode 被删除'
test "$(cat "$config_dir/system.md")" = 'user-owned-system-prompt'
test "$(cat "$config_dir/config")" = 'user-api-config'
test "$(cat "$config_dir/active_mode")" = 'otto'
test -f "$state_dir/install.manifest" || fail '安装状态未记录'
rg -q '^version=2$' "$state_dir/install.manifest" || fail '安装状态不是 v2'

# A v1 manifest from an older installer must be accepted and upgraded in
# place. The paths and ownership records use the same fields.
sed -i 's/^version=2$/version=1/' "$state_dir/install.manifest"
OTTO_INSTALL_BINARY="$binary_path" "$install_script" \
    --prefix "$prefix" \
    --config-dir "$config_dir" \
    --state-dir "$state_dir" \
    >"$temporary_directory/upgrade-output"
rg -q '^version=2$' "$state_dir/install.manifest" || fail 'v1 清单未升级为 v2'

"$uninstall_script" --state-dir "$state_dir" >"$temporary_directory/uninstall-output"

test ! -e "$prefix/bin/otto" || fail '可执行文件未卸载'
test ! -e "$config_dir/otto.md" || fail '安装的 otto.md 未卸载'
test ! -e "$config_dir/jarvis.md" || fail '安装的 jarvis.md 未卸载'
test -f "$config_dir/system.md" || fail '已有 system.md 被误删'
test -f "$config_dir/config" || fail '已有 API 配置被误删'
test -f "$config_dir/active_mode" || fail '已有 active_mode 被误删'
test ! -e "$state_dir/install.manifest" || fail '安装状态未清理'

modified_prefix="$temporary_directory/modified-prefix"
modified_config="$temporary_directory/modified-config"
modified_state="$temporary_directory/modified-state"
OTTO_INSTALL_BINARY="$binary_path" "$install_script" \
    --prefix "$modified_prefix" \
    --config-dir "$modified_config" \
    --state-dir "$modified_state" \
    >/dev/null
printf '%s\n' 'user customization' >>"$modified_config/jarvis.md"
printf '%s\n' 'otto' >"$modified_config/active_mode"

if "$uninstall_script" --state-dir "$modified_state" \
    >"$temporary_directory/modified-uninstall-output" 2>&1; then
    fail '卸载修改过的文件时应该报告未完全卸载'
fi

test ! -e "$modified_prefix/bin/otto" || fail '修改配置场景下可执行文件未卸载'
test ! -e "$modified_config/system.md" || fail '未修改的 system.md 未卸载'
test ! -e "$modified_config/otto.md" || fail '未修改的 otto.md 未卸载'
test -f "$modified_config/jarvis.md" || fail '用户修改的 jarvis.md 被误删'
test -f "$modified_config/active_mode" || fail 'active_mode 被误删'
test -f "$modified_state/install.manifest" || fail '部分卸载时安装状态未保留'

collision_prefix="$temporary_directory/collision-prefix"
collision_config="$temporary_directory/collision-config"
collision_state="$temporary_directory/collision-state"
mkdir -p "$collision_prefix/bin"
printf '%s\n' 'pre-existing-otto' >"$collision_prefix/bin/otto"

if OTTO_INSTALL_BINARY="$binary_path" "$install_script" \
    --prefix "$collision_prefix" \
    --config-dir "$collision_config" \
    --state-dir "$collision_state" \
    >"$temporary_directory/collision-output" 2>&1; then
    fail '目标已有未知可执行文件时应该停止安装'
fi

test "$(cat "$collision_prefix/bin/otto")" = 'pre-existing-otto'
test ! -e "$collision_config/system.md" || fail '碰撞安装不应创建提示词'
test ! -e "$collision_state/install.manifest" || fail '碰撞安装不应创建状态清单'

printf '%s\n' 'install and uninstall tests passed'
