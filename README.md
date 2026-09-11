# OTTO

<p align="center">
  <img src="assets/otto-logo.png" alt="OTTO 像素风 Logo：轮椅冲刺并发出单轮对话气泡" width="240">
</p>

<p align="center">
  <strong>One-time.Talk once</strong><br>
  面向 CLI 环境的单轮大模型问答工具
</p>

## 项目简介

OTTO 是一个面向 CLI 环境的单轮大模型 Agent 工具，适合用一句话解决一个明确问题。
当前正式实现使用 Rust，网络层采用 reqwest；早期的 C11/libcurl 实现作为 v0.1 基线保留。

项目名称采用品牌化表达，强调“一次提问、一次完成”的使用方式。OTTO 不维护跨请求的会话历史，每次调用都独立生成回答。

## 当前版本

当前正式版本为 Rust `1.0.0`：普通问答默认进入 Agent loop，模型可以按需调用本地
文件工具、websearch 和 webfetch。C11/libcurl 的 v0.1 基线通过
[`docs/legacy/c-v0.1.md`](docs/legacy/c-v0.1.md)、`c-v0.1.0` tag 和
`legacy/c-v0.1` 分支留档；迁移记录见 [`docs/migration/rust.md`](docs/migration/rust.md)。

正式构建入口：

```bash
make
target/release/otto --help
target/release/otto 你好
```

C 基线仍可以显式构建和回归测试，不会覆盖 Rust 的 `otto`：

```bash
make c-build
build/c/otto-c --help
make c-test
```

## 核心特性

正式 Rust 版本的 Agent 默认隐式启用；C v0.1 基线只保留单轮 Chat 能力。

| 能力 | 说明 |
| --- | --- |
| 简洁调用 | 直接使用 `otto <问题内容...>` 发起单轮问答。 |
| 参数拼接 | 程序会将问题参数按单个空格重新拼接，包含空格的问题通常不需要加引号。 |
| 流式输出 | 默认使用 OpenAI Chat Completions 兼容接口的 SSE 流式响应，生成内容会即时输出。 |
| 服务兼容 | 支持 OpenAI Chat Completions 兼容服务，包括 OpenAI、SiliconFlow 等。 |
| 提示词模式 | 每次必加载统一系统提示词，并可附加一个可持久化的语气模式。 |
| Agent 工具 | 默认按需调用 Glob、Grep、Read、Edit、Write、websearch 和 webfetch。 |
| 用户级安装 | 通过 Makefile 统一安装和卸载，不修改 Shell 配置并保护已有用户配置。 |

## 命令速查

| 命令 | 作用 |
| --- | --- |
| `otto <问题内容...>` | 使用当前已保存模式发起问答。 |
| `otto --help` | 显示命令帮助。 |
| `otto --version` | 显示版本号。 |
| `otto --config` | 进入交互式配置界面。 |
| `otto --config --name ... --baseurl ... --apikey ...` | 使用命令行参数保存服务配置。 |
| `otto --mode <模式>` | 设置并保存默认模式。 |
| `otto --mode` | 清除默认模式，恢复裸生成模式。 |
| `otto --mode <模式> <问题内容...>` | 仅本次请求附加指定模式，不修改默认模式。 |
| `otto --mode -- <问题内容...>` | 仅本次请求不附加可选模式。 |
| `otto --root <目录> <问题内容...>` | 设置本轮 Agent 的 workspace 根目录。 |
| `otto --no-agent <问题内容...>` | 本次请求跳过工具调用，只发送普通 Chat 请求。 |

## 快速开始

完成依赖安装后，可以直接编译、配置并发起第一条请求：

```bash
make
target/release/otto --config
target/release/otto 你好
```

如果已经完成当前用户安装，将上面的 `target/release/otto` 替换为 `otto` 即可。

## 提示词与模式

所有请求都会加载 `system.md` 作为统一系统提示词；文件缺失时请求会失败。模式提示词是可选附加项，每次最多选择一个。

最终发送给模型的系统提示词按以下顺序组合：

```text
system.md
+ <模式>.md（如果选择了模式）
```

源码目录中提供了 `system.md`、`otto.md` 与 `jarvis.md` 示例。直接从源码目录运行时，如果配置目录没有 `system.md` 或 `otto.md`，程序会回退读取项目根目录中的对应文件；安装后建议将提示词复制到配置目录：

```bash
mkdir -p ~/.config/otto
cp system.md ~/.config/otto/system.md
cp otto.md ~/.config/otto/otto.md
cp jarvis.md ~/.config/otto/jarvis.md
```

默认情况下，模式文件、统一系统提示词和当前模式状态位于同一个目录：

```text
~/.config/otto/system.md
~/.config/otto/otto.md
~/.config/otto/test.md
~/.config/otto/active_mode
```

其中 `system.md` 是必需文件，`otto.md`、`jarvis.md`、`test.md` 等是可选模式文件。新增模式时，只需在该目录创建同名的 `<模式>.md` 文件：

```bash
vi ~/.config/otto/test.md
otto --mode test
```

模式名称仅支持字母、数字、`-`、`_` 和 `.`。

先选择模式，选择结果会保存下来，之后普通的 `otto <问题>` 会自动使用它：

```bash
otto --mode otto
otto 你好
```

`otto --mode otto` 会保存 `otto` 模式；之后每次普通请求都会把 `system.md` 与 `otto.md` 合并后作为 system prompt。其他模式同理：

```bash
otto --mode test
otto 你好
```

项目中也提供了 JARVIS 模式，复制后即可启用：

```bash
cp jarvis.md ~/.config/otto/jarvis.md
otto --mode jarvis
otto 你好
```

清除已保存的附加模式，只保留统一 `system.md`：

```bash
otto --mode
otto 你好
```

也可以只对当前请求临时指定模式或不附加模式，且不会改变已保存的模式：

```bash
otto --mode test 你好
otto --mode -- 你好
```

上面的 `otto --mode -- 你好` 仍然会加载 `system.md`，只是本次不附加任何模式提示词。

通过 `OTTO_MODE_DIR` 可以指定提示词文件的读取目录。当前激活模式 `active_mode` 仍保存在配置目录中。

## API 与流式输出

程序默认发送 `"stream": true`，并解析 `data:` SSE 事件。兼容 OpenAI Chat Completions 流式接口的服务可以直接使用，例如 SiliconFlow，不需要增加命令行参数。

## Agent 与工具

OTTO 默认使用 Agent loop，可使用以下工具：glob 查找 workspace 内的文件，grep 搜索
workspace 内的文本，read 读取 UTF-8 文本，edit 精确替换文本并返回 diff，write
创建或覆盖文本文件，websearch 检索互联网信息，webfetch 抓取公开网页并提取正文。

本地文件工具只能访问当前 workspace 的相对路径，拒绝路径穿越和逃逸到 workspace 外的
符号链接。read、grep、glob 在访问本地内容前会请求读取授权；edit 和 write 会请求
写入授权。授权选择为：1 仅此次、2 本轮同类操作总是允许、3 拒绝。授权提示会根据
当前 mode 使用对应语气。

websearch 不会默认调用 Google，而是通过 provider 适配层工作。配置会由真正的
`otto` 原生读取 `$XDG_CONFIG_HOME/otto/search.env`（默认
`~/.config/otto/search.env`），不需要修改 `~/.bashrc`，也不需要额外的启动器。
当前支持 Brave Search、SearXNG 和 Tavily：

    # Tavily（默认使用 basic 搜索深度以节省额度）
    OTTO_SEARCH_PROVIDER=tavily
    OTTO_TAVILY_API_KEY=...

    # Brave Search
    OTTO_SEARCH_PROVIDER=brave
    OTTO_BRAVE_API_KEY=...

    # 或自建/可信的 SearXNG
    OTTO_SEARCH_PROVIDER=searxng
    OTTO_SEARCH_URL=https://your-searxng.example/search

将需要的配置写入 `search.env`，并限制文件权限：

    chmod 600 ~/.config/otto/search.env

`OTTO_SEARCH_CONFIG` 可以临时指定另一份配置文件；为兼容旧用法，进程环境变量只会作为
配置文件中未设置字段的回退。旧启动器使用的 `OTTO_BIN` 不再参与配置解析。

webfetch 只允许 HTTP/HTTPS，限制响应大小和重定向次数，并拒绝本地、内网和解析到内网
地址的主机；网页内容会以不可信工具数据交给模型。

## 构建环境

- Rust 工具链（正式构建需要）
- C11 编译器、libcurl 开发库和 pkg-config（仅 `c-build` / `c-test` 需要）

Ubuntu/Debian 示例：

```bash
sudo apt install build-essential pkg-config libcurl4-openssl-dev
```

## 构建与测试

```bash
make
make test
```

`make test` 会先测试正式 Rust 版本，再运行 C 基线回归；也可以分别执行
`make rust-test`、`make rust-functional-test` 和 `make c-test`。

## 安装与卸载

推荐使用 Makefile 安装到当前用户，不需要 `sudo`：

```bash
make install
```

默认安装位置：

```text
可执行文件：~/.local/bin/otto
提示词配置：~/.config/otto/
安装状态：~/.local/state/otto/install.manifest
```

安装脚本会自动编译缺失的 Rust release 可执行文件；已有 API 配置、已有模式文件和当前激活模式都不会被覆盖。它也不会修改 `.bashrc` 等 Shell 配置文件，如果 `~/.local/bin` 不在 `PATH` 中，脚本只会给出提示。

卸载时执行：

```bash
make uninstall
```

`make install` 和 `make uninstall` 是正式生命周期入口。原有的
`scripts/install.sh`、`scripts/uninstall.sh` 仍然保留，供旧脚本和自动化流程直接调用。

卸载脚本只处理安装清单中由安装脚本创建、且内容没有被修改的可执行文件和提示词。用户后来修改过的文件、已有的 `config` API 配置文件和 `active_mode` 会被保留；因此卸载不会破坏现有环境。

安装和卸载都支持通过 Make 变量自定义路径。安装时如果使用了 `STATE_DIR`，卸载时需要使用同一个路径：

```bash
make install PREFIX="$HOME/.local" CONFIG_DIR="$HOME/.config/otto" STATE_DIR="$HOME/.local/state/otto"
make uninstall STATE_DIR="$HOME/.local/state/otto"
```

不设置 `PREFIX`、`CONFIG_DIR`、`STATE_DIR` 时，安装脚本分别使用当前用户的
`~/.local`、XDG 配置目录和 XDG 状态目录；不会修改 `.bashrc` 等 Shell 配置，也不会
覆盖已有 API 配置、active_mode 或用户修改过的提示词。安装清单 v2 兼容旧 v1 清单，
升级时会先校验原文件并支持失败回滚。

## 服务配置

推荐使用交互式配置：

```bash
otto --config
```

程序会引导输入服务名称、Base URL、API Key 和模型名称。输入 API Key 时终端不会回显。

配置文件默认位于：

```text
$XDG_CONFIG_HOME/otto/config
```

如果没有设置 `XDG_CONFIG_HOME`，则使用：

```text
~/.config/otto/config
```

配置文件路径的优先级如下：

1. 设置 `OTTO_CONFIG` 时，直接使用该变量指定的配置文件。
2. 未设置 `OTTO_CONFIG` 时，优先使用 `$XDG_CONFIG_HOME/otto/config`。
3. 未设置 `XDG_CONFIG_HOME` 时，使用 `~/.config/otto/config`。

如果通过 `OTTO_CONFIG` 指定了配置文件，提示词目录默认取该配置文件所在目录。

配置文件权限为 `0600`。也可以使用参数式配置，适合自动化环境：

```bash
otto --config \
  --name Openai \
  --baseurl https://api.openai.com \
  --apikey sk-xxxx \
  --model gpt-4o-mini
```

例如使用 SiliconFlow：

```bash
otto --config \
  --name SiliconFlow \
  --baseurl https://api.siliconflow.cn/v1/chat/completions \
  --apikey sk-xxxx \
  --model your-siliconflow-model
```

不同服务支持的模型名称不同，请填写对应服务提供的实际模型名。

参数式配置会将 API Key 暴露给 Shell 历史，生产环境推荐使用交互式配置。

配置服务使用 OpenAI Chat Completions 兼容接口。`name` 只是服务名称，不限制为 OpenAI，SiliconFlow、其他兼容服务都可以使用。Base URL 可以填写完整地址，也可以填写服务根地址：

```text
https://api.openai.com
https://api.openai.com/v1
https://api.openai.com/v1/chat/completions
```

## 管道与脚本集成

回答输出到 stdout，错误输出到 stderr：

```bash
otto 总结这段文字 | tee answer.txt
```

如果问题以短横线开头，可以使用 `--`：

```bash
otto -- --help 是什么意思
```

## 项目结构

```text
.
├── src/                  C v0.1 基线源码
├── include/otto/         C 基线公共头文件
├── build/c/              C 基线构建产物（忽略）
├── tests/                功能与安装脚本测试
├── scripts/              安装与卸载兼容入口
├── rust/otto/            正式 Rust CLI 与 Agent
├── docs/                 C 基线和 Rust 迁移记录
├── assets/               项目 Logo 等静态资源
├── system.md             统一系统提示词
├── otto.md               OTTO 语气模式
├── jarvis.md             JARVIS 语气模式
├── Makefile              编译、测试与系统级安装入口
└── README.md             项目说明
```

## 路线图

- [x] Rust 正式入口、默认 Agent loop、文件工具、websearch 和 webfetch。
- [x] Makefile 统一安装、卸载和 v1 → v2 安全升级路径。
- [ ] 添加音频模式：支持语音输出，输出 OTTO 的“活字印刷”语音。
