#!/bin/sh
#
# Remove only resources recorded by scripts/install.sh or `make install`.
# User API configuration, active mode state, and modified files are preserved.
set -eu

launch_dir=$(pwd -P)

die() {
    printf 'otto uninstall: %s\n' "$*" >&2
    exit 1
}

info() {
    printf 'otto uninstall: %s\n' "$*"
}

usage() {
    cat <<'EOF'
用法：
  scripts/uninstall.sh [选项]

选项：
  --state-dir DIR    安装状态目录，默认 $XDG_STATE_HOME/otto 或 ~/.local/state/otto
  -h, --help         显示帮助

如果安装时使用了自定义状态目录，卸载时请使用相同的 --state-dir，
或设置 OTTO_INSTALL_STATE_DIR 环境变量。
EOF
}

absolute_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        *) printf '%s/%s\n' "$launch_dir" "$1" ;;
    esac
}

state_value() {
    awk -v key="$1" \
        'index($0, key "=") == 1 { print substr($0, length(key) + 2); exit }' \
        "$manifest"
}

install_state_dir=${OTTO_INSTALL_STATE_DIR-}
if [ -z "$install_state_dir" ]; then
    xdg_state_home=${XDG_STATE_HOME-}
    if [ -n "$xdg_state_home" ]; then
        install_state_dir="$xdg_state_home/otto"
    else
        user_home=${HOME-}
        if [ -z "$user_home" ]; then
            user_home=$(getent passwd "$(id -u)" 2>/dev/null | cut -d: -f6 || true)
        fi
        [ -n "$user_home" ] || die '无法确定当前用户的 home 目录'
        install_state_dir="$user_home/.local/state/otto"
    fi
fi

while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --state-dir)
            [ "$#" -ge 2 ] || die '选项 --state-dir 缺少值'
            install_state_dir=$2
            shift 2
            ;;
        --state-dir=*)
            install_state_dir=${1#*=}
            [ -n "$install_state_dir" ] || die '选项 --state-dir 的值不能为空'
            shift
            ;;
        *)
            die "未知选项：$1"
            ;;
    esac
done

state_dir=$(absolute_path "$install_state_dir")
manifest="$state_dir/install.manifest"

[ "$state_dir" != '/' ] || die '拒绝把状态目录设为 /'

if [ ! -e "$manifest" ] && [ ! -L "$manifest" ]; then
    info "没有找到 OTTO 安装状态，不删除任何文件：$manifest"
    exit 0
fi
[ ! -L "$manifest" ] || die "安装状态文件是符号链接，为安全起见停止卸载：$manifest"
[ -f "$manifest" ] || die "安装状态文件不是普通文件：$manifest"
[ -r "$manifest" ] || die "无法读取安装状态文件：$manifest"

manifest_version=$(state_value version)
[ "$manifest_version" = 1 ] || [ "$manifest_version" = 2 ] ||
    die "无法识别安装状态文件：$manifest"
recorded_state_dir=$(state_value state_dir)
[ "$recorded_state_dir" = "$state_dir" ] ||
    die '状态路径与安装记录不一致，请使用安装时的原路径'

binary=$(state_value binary)
config_dir=$(state_value config_dir)
checksum_algorithm=$(state_value checksum_algorithm)
[ -n "$binary" ] || die '安装状态缺少可执行文件路径'
[ -n "$config_dir" ] || die '安装状态缺少配置目录路径'
[ "$binary" != '/' ] || die '安装状态中的可执行文件路径不安全'
[ "$config_dir" != '/' ] || die '安装状态中的配置目录路径不安全'

case "$checksum_algorithm" in
    sha256)
        command -v sha256sum >/dev/null 2>&1 ||
            die '找不到 sha256sum，无法安全校验安装文件'
        ;;
    cksum)
        command -v cksum >/dev/null 2>&1 ||
            die '找不到 cksum，无法安全校验安装文件'
        ;;
    *)
        die '安装状态中的文件校验方式无效'
        ;;
esac

file_checksum() {
    if [ "$checksum_algorithm" = sha256 ]; then
        sha256sum -- "$1" | awk '{ print $1 }'
    else
        cksum "$1" | awk '{ print $1 ":" $2 }'
    fi
}

preserved_count=0

remove_owned_file() {
    target=$1
    expected_checksum=$2
    description=$3

    if [ ! -e "$target" ] && [ ! -L "$target" ]; then
        return
    fi

    if [ -L "$target" ] || [ ! -f "$target" ]; then
        info "保留非普通文件，不删除：$target"
        preserved_count=$((preserved_count + 1))
        return
    fi

    current_checksum=$(file_checksum "$target")
    if [ -z "$expected_checksum" ] || [ "$current_checksum" != "$expected_checksum" ]; then
        info "保留已修改的$description：$target"
        preserved_count=$((preserved_count + 1))
        return
    fi

    rm -f -- "$target"
    info "已卸载$description：$target"
}

remove_owned_file "$binary" "$(state_value binary_checksum)" '可执行文件'

for prompt_name in system otto jarvis; do
    managed=$(state_value "$prompt_name""_managed")
    [ "$managed" = 0 ] || [ "$managed" = 1 ] ||
        die "无法识别提示词安装状态：$prompt_name"
    if [ "$managed" -eq 1 ]; then
        remove_owned_file \
            "$config_dir/$prompt_name.md" \
            "$(state_value "$prompt_name""_checksum")" \
            "提示词 $prompt_name.md"
    fi
done

if [ "$preserved_count" -ne 0 ]; then
    info "为保护用户环境，仍有资源未删除；安装状态已保留：$manifest"
    exit 1
fi

rm -f -- "$manifest"
if rmdir "$state_dir" 2>/dev/null; then
    info "已清理空的安装状态目录：$state_dir"
else
    info "状态目录包含其他文件，已保留：$state_dir"
fi
info '卸载完成；API 配置文件和 active_mode 未被删除。'
