# OTTO

<p align="center">
  <img src="assets/otto-logo.png" alt="OTTO 像素风 Logo：轮椅冲刺并发出单轮对话气泡" width="240">
</p>

<p align="center">
  <strong>One-time.Talk once</strong><br>
  面向 CLI 环境的单轮大模型问答工具
</p>

## 项目简介

OTTO 是一个使用 C11 与 libcurl 实现的命令行大模型问答工具，面向可以通过一次请求直接完成的简单问题。

项目名称采用品牌化表达，强调“一次提问、一次完成”的使用方式。OTTO 不维护跨请求的会话历史，每次调用都独立生成回答。

## 核心特性

| 能力 | 说明 |
| --- | --- |
| 简洁调用 | 直接使用 `otto <问题内容...>` 发起单轮问答。 |
| 参数拼接 | 程序会将问题参数按单个空格重新拼接，包含空格的问题通常不需要加引号。 |
| 流式输出 | 默认使用 OpenAI Chat Completions 兼容接口的 SSE 流式响应，生成内容会即时输出。 |
| 服务兼容 | 支持 OpenAI Chat Completions 兼容服务，包括 OpenAI、SiliconFlow 等。 |
| 提示词模式 | 每次必加载统一系统提示词，并可附加一个可持久化的语气模式。 |
| 用户级安装 | 提供不修改 Shell 配置、保护已有用户配置的安装与卸载脚本。 |

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

## 快速开始

完成依赖安装后，可以直接编译、配置并发起第一条请求：

```bash
make
./otto --config
./otto 你好
```

如果已经完成当前用户安装，将上面的 `./otto` 替换为 `otto` 即可。

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

## 构建环境

- C11 编译器
- libcurl 开发库
- pkg-config

Ubuntu/Debian 示例：

```bash
sudo apt install build-essential pkg-config libcurl4-openssl-dev
```

## 构建与测试

```bash
make
make test
```

## 安装与卸载

推荐使用脚本安装到当前用户，不需要 `sudo`：

```bash
./scripts/install.sh
```

默认安装位置：

```text
可执行文件：~/.local/bin/otto
提示词配置：~/.config/otto/
安装状态：~/.local/state/otto/install.manifest
```

安装脚本会自动编译缺失的 `otto` 可执行文件；已有 API 配置、已有模式文件和当前激活模式都不会被覆盖。它也不会修改 `.bashrc` 等 Shell 配置文件，如果 `~/.local/bin` 不在 `PATH` 中，脚本只会给出提示。

卸载时执行：

```bash
./scripts/uninstall.sh
```

卸载脚本只处理安装清单中由安装脚本创建、且内容没有被修改的可执行文件和提示词。用户后来修改过的文件、已有的 `config` API 配置文件和 `active_mode` 会被保留；因此卸载不会破坏现有环境。

安装和卸载都支持自定义状态目录。安装时如果使用了 `--state-dir`，卸载时需要使用同一个路径：

```bash
./scripts/install.sh --prefix ~/.local --config-dir ~/.config/otto --state-dir ~/.local/state/otto
./scripts/uninstall.sh --state-dir ~/.local/state/otto
```

安装到 `/usr/local/bin`：

```bash
sudo make install
```

也可以指定安装目录：

```bash
make install PREFIX="$HOME/.local"
```

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
├── src/                  C 源码
├── include/otto/         公共头文件
├── tests/                功能与安装脚本测试
├── scripts/              当前用户安装与卸载脚本
├── assets/               项目 Logo 等静态资源
├── system.md             统一系统提示词
├── otto.md               OTTO 语气模式
├── jarvis.md             JARVIS 语气模式
├── Makefile              编译、测试与系统级安装入口
└── README.md             项目说明
```

## 路线图

- [ ] 添加 Agent 工具调用能力：在回答前根据问题调用合适的工具，逐步扩展为支持生成文件等任务的单轮 Agent 会话。
- [ ] 添加音频模式：支持语音输出，输出 OTTO 的“活字印刷”语音。
