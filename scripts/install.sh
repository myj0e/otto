#!/bin/sh
#
# Install the maintained Rust OTTO binary for the current user.
# Existing API configuration and user-owned prompt files are never overwritten.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
project_root=$(CDPATH= cd -- "$script_dir/.." && pwd -P)
launch_dir=$(pwd -P)

die() {
    printf 'otto install: %s\n' "$*" >&2
    exit 1
}

info() {
    printf 'otto install: %s\n' "$*"
}

usage() {
    cat <<'EOF'
用法：
  scripts/install.sh [选项]

选项：
  --prefix DIR       安装前缀，默认 ~/.local
  --config-dir DIR   配置目录，默认 $XDG_CONFIG_HOME/otto 或 ~/.config/otto
  --state-dir DIR    安装状态目录，默认 $XDG_STATE_HOME/otto 或 ~/.local/state/otto
  -h, --help         显示帮助

也可以使用 OTTO_INSTALL_PREFIX、OTTO_INSTALL_CONFIG_DIR、
OTTO_INSTALL_STATE_DIR 环境变量指定路径。

默认安装 target/release/otto；也可以通过 OTTO_INSTALL_BINARY 指定已经构建的
兼容二进制。推荐使用 make install 调用此脚本。
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

if command -v sha256sum >/dev/null 2>&1; then
    checksum_algorithm=sha256
elif command -v cksum >/dev/null 2>&1; then
    checksum_algorithm=cksum
else
    die '找不到 sha256sum 或 cksum，无法安全记录文件完整性'
fi

file_checksum() {
    if [ "$checksum_algorithm" = sha256 ]; then
        sha256sum -- "$1" | awk '{ print $1 }'
    else
        cksum "$1" | awk '{ print $1 ":" $2 }'
    fi
}

rollback_prompt() {
    prompt_name=$1
    expected_checksum=$2
    prompt_path="$config_dir/$prompt_name.md"

    if [ -n "$expected_checksum" ] && [ -f "$prompt_path" ] &&
        [ ! -L "$prompt_path" ]; then
        current_checksum=$(file_checksum "$prompt_path" 2>/dev/null || printf '')
        if [ "$current_checksum" = "$expected_checksum" ]; then
            rm -f -- "$prompt_path" 2>/dev/null || true
        fi
    fi
}

cleanup() {
    exit_code=$?

    if [ "$exit_code" -ne 0 ]; then
        if [ -n "$temporary_binary" ]; then
            rm -f -- "$temporary_binary" 2>/dev/null || true
        fi
        if [ -n "$temporary_prompt" ]; then
            rm -f -- "$temporary_prompt" 2>/dev/null || true
        fi
        if [ -n "$temporary_manifest" ]; then
            rm -f -- "$temporary_manifest" 2>/dev/null || true
        fi

        if [ "$binary_replaced" -eq 1 ] && [ -n "$binary_backup" ] &&
            [ -f "$binary_backup" ]; then
            if mv -f -- "$binary_backup" "$binary" 2>/dev/null; then
                binary_backup=''
            else
                printf 'otto install: 无法回滚可执行文件，备份仍保留在：%s\n' \
                    "$binary_backup" >&2
            fi
        elif [ "$binary_installed" -eq 1 ] && [ -f "$binary" ] &&
            [ ! -L "$binary" ] && [ -n "$binary_checksum" ]; then
            current_checksum=$(file_checksum "$binary" 2>/dev/null || printf '')
            if [ "$current_checksum" = "$binary_checksum" ]; then
                rm -f -- "$binary" 2>/dev/null || true
            fi
        fi

        if [ "$binary_replaced" -eq 0 ] && [ -n "$binary_backup" ]; then
            rm -f -- "$binary_backup" 2>/dev/null || true
        fi

        for prompt_name in $created_prompt_names; do
            case "$prompt_name" in
                system) rollback_prompt system "$created_system_checksum" ;;
                otto) rollback_prompt otto "$created_otto_checksum" ;;
                jarvis) rollback_prompt jarvis "$created_jarvis_checksum" ;;
            esac
        done

        if [ "$config_dir_created" -eq 1 ]; then
            rmdir "$config_dir" 2>/dev/null || true
        fi
        if [ "$bin_dir_created" -eq 1 ]; then
            rmdir "$bin_dir" 2>/dev/null || true
        fi
        if [ "$state_dir_created" -eq 1 ]; then
            rmdir "$state_dir" 2>/dev/null || true
        fi
    fi

    exit "$exit_code"
}

user_home=${HOME-}
if [ -z "$user_home" ]; then
    user_home=$(getent passwd "$(id -u)" 2>/dev/null | cut -d: -f6 || true)
fi
[ -n "$user_home" ] || die '无法确定当前用户的 home 目录'

install_prefix=${OTTO_INSTALL_PREFIX-}
[ -n "$install_prefix" ] || install_prefix="$user_home/.local"

xdg_config_home=${XDG_CONFIG_HOME-}
if [ -n "$xdg_config_home" ]; then
    install_config_dir=${OTTO_INSTALL_CONFIG_DIR-}
    [ -n "$install_config_dir" ] || install_config_dir="$xdg_config_home/otto"
else
    install_config_dir=${OTTO_INSTALL_CONFIG_DIR-}
    [ -n "$install_config_dir" ] || install_config_dir="$user_home/.config/otto"
fi

xdg_state_home=${XDG_STATE_HOME-}
if [ -n "$xdg_state_home" ]; then
    install_state_dir=${OTTO_INSTALL_STATE_DIR-}
    [ -n "$install_state_dir" ] || install_state_dir="$xdg_state_home/otto"
else
    install_state_dir=${OTTO_INSTALL_STATE_DIR-}
    [ -n "$install_state_dir" ] || install_state_dir="$user_home/.local/state/otto"
fi

while [ "$#" -gt 0 ]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --prefix)
            [ "$#" -ge 2 ] || die '选项 --prefix 缺少值'
            install_prefix=$2
            shift 2
            ;;
        --prefix=*)
            install_prefix=${1#*=}
            [ -n "$install_prefix" ] || die '选项 --prefix 的值不能为空'
            shift
            ;;
        --config-dir)
            [ "$#" -ge 2 ] || die '选项 --config-dir 缺少值'
            install_config_dir=$2
            shift 2
            ;;
        --config-dir=*)
            install_config_dir=${1#*=}
            [ -n "$install_config_dir" ] || die '选项 --config-dir 的值不能为空'
            shift
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

prefix=$(absolute_path "$install_prefix")
config_dir=$(absolute_path "$install_config_dir")
state_dir=$(absolute_path "$install_state_dir")
bin_dir="$prefix/bin"
binary="$bin_dir/otto"
manifest="$state_dir/install.manifest"

binary_source=${OTTO_INSTALL_BINARY-}
if [ -z "$binary_source" ]; then
    binary_source="$project_root/target/release/otto"
else
    binary_source=$(absolute_path "$binary_source")
fi
prompt_source_dir="$project_root/prompts"

[ -n "$prefix" ] || die '安装前缀不能为空'
[ -n "$config_dir" ] || die '配置目录不能为空'
[ -n "$state_dir" ] || die '状态目录不能为空'
[ "$prefix" != '/' ] || die '拒绝把安装前缀设为 /'
[ "$config_dir" != '/' ] || die '拒绝把配置目录设为 /'
[ "$state_dir" != '/' ] || die '拒绝把状态目录设为 /'

temporary_binary=''
temporary_prompt=''
temporary_manifest=''
binary_backup=''
binary_checksum=''
binary_replaced=0
binary_installed=0
created_prompt_names=''
created_system_checksum=''
created_otto_checksum=''
created_jarvis_checksum=''
config_dir_created=0
bin_dir_created=0
state_dir_created=0
manifest_exists=0

trap cleanup EXIT

if [ ! -x "$binary_source" ]; then
    command -v make >/dev/null 2>&1 || die '找不到可执行文件，也找不到 make'
    info '未找到 Rust release 构建结果，正在编译 OTTO'
    make -C "$project_root" rust-build || die '编译失败'
fi
[ -x "$binary_source" ] || die "找不到可执行文件：$binary_source"
[ -f "$binary_source" ] || die "可执行文件不是普通文件：$binary_source"
[ ! -L "$binary_source" ] || die "可执行文件是符号链接：$binary_source"

for prompt_name in system otto jarvis; do
    [ -f "$prompt_source_dir/$prompt_name.md" ] ||
        die "找不到提示词文件：$prompt_source_dir/$prompt_name.md"
done

if [ -L "$state_dir" ]; then
    die "状态目录是符号链接，为安全起见停止安装：$state_dir"
elif [ -e "$state_dir" ]; then
    [ -d "$state_dir" ] || die "状态目录不是目录：$state_dir"
else
    mkdir -p -m 700 -- "$state_dir" || die "无法创建状态目录：$state_dir"
    state_dir_created=1
fi

if [ -L "$manifest" ]; then
    die "安装状态文件是符号链接，为安全起见停止安装：$manifest"
elif [ -e "$manifest" ]; then
    [ -f "$manifest" ] || die "安装状态文件不是普通文件：$manifest"
    [ -r "$manifest" ] || die "无法读取安装状态文件：$manifest"
    manifest_exists=1
    manifest_version=$(state_value version)
    [ "$manifest_version" = 1 ] || [ "$manifest_version" = 2 ] ||
        die "无法识别安装状态文件：$manifest"
    [ "$(state_value binary)" = "$binary" ] ||
        die '安装路径与已有安装状态不一致，请使用原路径卸载或显式指定相同路径'
    [ "$(state_value config_dir)" = "$config_dir" ] ||
        die '配置路径与已有安装状态不一致，请使用原路径卸载或显式指定相同路径'
    [ "$(state_value state_dir)" = "$state_dir" ] ||
        die '状态路径与已有安装状态不一致，请使用原路径卸载或显式指定相同路径'
    [ "$(state_value checksum_algorithm)" = "$checksum_algorithm" ] ||
        die '当前环境的文件校验方式与已有安装状态不一致'
fi

if [ -L "$config_dir" ]; then
    die "配置目录是符号链接，为安全起见停止安装：$config_dir"
elif [ -e "$config_dir" ]; then
    [ -d "$config_dir" ] || die "配置目录不是目录：$config_dir"
else
    mkdir -p -m 700 -- "$config_dir" || die "无法创建配置目录：$config_dir"
    config_dir_created=1
fi

if [ -L "$bin_dir" ]; then
    die "bin 目录是符号链接，为安全起见停止安装：$bin_dir"
elif [ -e "$bin_dir" ]; then
    [ -d "$bin_dir" ] || die "bin 路径不是目录：$bin_dir"
else
    mkdir -p -m 755 -- "$bin_dir" || die "无法创建 bin 目录：$bin_dir"
    bin_dir_created=1
fi

if [ -L "$binary" ]; then
    die "目标可执行文件是符号链接，为安全起见不覆盖：$binary"
elif [ -e "$binary" ]; then
    [ "$manifest_exists" -eq 1 ] ||
        die "目标已存在且不属于 OTTO 安装记录，为安全起见不覆盖：$binary"
    [ -f "$binary" ] || die "目标不是普通文件，为安全起见不覆盖：$binary"
    expected_binary_checksum=$(state_value binary_checksum)
    current_binary_checksum=$(file_checksum "$binary")
    [ "$current_binary_checksum" = "$expected_binary_checksum" ] ||
        die "目标可执行文件已被修改，为安全起见不覆盖：$binary"
    binary_backup=$(mktemp "$state_dir/.otto-binary-backup.XXXXXX") ||
        die '无法创建可执行文件备份'
    cp -p -- "$binary" "$binary_backup" || die '无法备份现有可执行文件'
fi

temporary_binary=$(mktemp "$bin_dir/.otto-install.XXXXXX") ||
    die '无法创建可执行文件临时文件'
install -m 0755 -- "$binary_source" "$temporary_binary" ||
    die '无法准备可执行文件'
binary_checksum=$(file_checksum "$temporary_binary")
mv -f -- "$temporary_binary" "$binary" || die '无法原子安装可执行文件'
temporary_binary=''
if [ -n "$binary_backup" ]; then
    binary_replaced=1
else
    binary_installed=1
fi

system_managed=0
system_checksum=''
otto_managed=0
otto_checksum=''
jarvis_managed=0
jarvis_checksum=''

for prompt_name in system otto jarvis; do
    source_prompt="$prompt_source_dir/$prompt_name.md"
    destination_prompt="$config_dir/$prompt_name.md"
    managed=0
    expected_checksum=''

    if [ "$manifest_exists" -eq 1 ]; then
        managed=$(state_value "$prompt_name""_managed")
        expected_checksum=$(state_value "$prompt_name""_checksum")
        [ "$managed" = 0 ] || [ "$managed" = 1 ] ||
            die "无法识别提示词安装状态：$prompt_name"
    fi

    if [ -e "$destination_prompt" ] || [ -L "$destination_prompt" ]; then
        if [ "$managed" -eq 1 ] && [ -f "$destination_prompt" ] &&
            [ ! -L "$destination_prompt" ]; then
            current_prompt_checksum=$(file_checksum "$destination_prompt")
            if [ "$current_prompt_checksum" != "$expected_checksum" ]; then
                info "保留已修改的提示词，不覆盖：$destination_prompt"
            else
                info "保留已安装提示词：$destination_prompt"
            fi
        else
            info "保留已有配置，不覆盖：$destination_prompt"
        fi
    else
        temporary_prompt=$(mktemp "$config_dir/.otto-prompt.XXXXXX") ||
            die "无法创建提示词临时文件：$prompt_name.md"
        install -m 0600 -- "$source_prompt" "$temporary_prompt" ||
            die "无法准备提示词：$prompt_name.md"
        mv -f -- "$temporary_prompt" "$destination_prompt" ||
            die "无法安装提示词：$prompt_name.md"
        temporary_prompt=''
        managed=1
        expected_checksum=$(file_checksum "$destination_prompt")
        created_prompt_names="$created_prompt_names $prompt_name"
        case "$prompt_name" in
            system) created_system_checksum=$expected_checksum ;;
            otto) created_otto_checksum=$expected_checksum ;;
            jarvis) created_jarvis_checksum=$expected_checksum ;;
        esac
        info "已安装提示词：$destination_prompt"
    fi

    case "$prompt_name" in
        system)
            system_managed=$managed
            system_checksum=$expected_checksum
            ;;
        otto)
            otto_managed=$managed
            otto_checksum=$expected_checksum
            ;;
        jarvis)
            jarvis_managed=$managed
            jarvis_checksum=$expected_checksum
            ;;
    esac
done

temporary_manifest=$(mktemp "$state_dir/.install-manifest.XXXXXX") ||
    die '无法创建安装状态临时文件'
{
    printf '%s\n' '# OTTO user installation manifest v2'
    printf 'version=2\n'
    printf 'binary_kind=rust-release\n'
    printf 'binary=%s\n' "$binary"
    printf 'config_dir=%s\n' "$config_dir"
    printf 'state_dir=%s\n' "$state_dir"
    printf 'checksum_algorithm=%s\n' "$checksum_algorithm"
    printf 'binary_checksum=%s\n' "$binary_checksum"
    printf 'system_managed=%s\n' "$system_managed"
    printf 'system_checksum=%s\n' "$system_checksum"
    printf 'otto_managed=%s\n' "$otto_managed"
    printf 'otto_checksum=%s\n' "$otto_checksum"
    printf 'jarvis_managed=%s\n' "$jarvis_managed"
    printf 'jarvis_checksum=%s\n' "$jarvis_checksum"
} >"$temporary_manifest" || die '无法写入安装状态'
chmod 0600 -- "$temporary_manifest" || die '无法设置安装状态权限'
mv -f -- "$temporary_manifest" "$manifest" || die '无法提交安装状态'
temporary_manifest=''

if [ -n "$binary_backup" ]; then
    rm -f -- "$binary_backup" 2>/dev/null ||
        info "无法清理临时备份，请手动删除：$binary_backup"
    binary_backup=''
fi

trap - EXIT
info "已安装可执行文件：$binary"
info 'API 配置文件未创建、未覆盖；已有配置和模式文件均受到保护。'
case ":${PATH-}:" in
    *":$bin_dir:") ;;
    *) printf '提示：%s 不在 PATH 中，可执行：export PATH="%s:$PATH"\n' "$bin_dir" "$bin_dir" ;;
esac
