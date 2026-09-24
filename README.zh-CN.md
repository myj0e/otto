<p align="center">
  <img src="assets/otto-logo.png" alt="OTTO Logo" width="240">
</p>

[English README](README.md)


# One Time. Talk Once.

OTTO 是一个运行在终端里的 AI 助手。你可以直接用自然语言提问，让它回答问题、解释代码、分析当前项目、处理文件，并在配置后搜索和阅读网页。

它适合快速完成一个明确的任务：每次命令都是独立请求，不会自动混入上一次调用的内容。修改本地文件或执行 Bash 命令前，OTTO 都会请求授权；查找和读取文件默认允许执行。

> 一个睿智的轮椅人，从眼前冲刺而过，带来智慧的哲言。但是记性不好的他，~~太阳升起时就把昨天忘掉~~下次会忘记上次的对话。

## 你可以用 OTTO 做什么

- 解释报错、命令和代码，帮助定位问题。
- 阅读当前项目中的文件，查找相关代码并总结结构。
- 在每次获得授权后运行非交互式 Bash 脚本。
- 在获得授权后创建、修改或整理文件。
- 配置联网搜索，查找最新资料、文档和网页内容。
- 通过不同的模式调整回答风格，例如技术评审、简洁回答或角色化表达。
- 将回答直接交给其他命令或脚本继续处理。

## 快速开始

### 1. 安装依赖

OTTO 需要 Rust 工具链和 `make`。如果还没有 Rust，可以使用 rustup 安装：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
```

### 2. 编译并安装

在 OTTO 项目目录中执行：

```bash
make
make install
```

默认安装到当前用户目录，不需要 `sudo`：

```text
可执行文件：~/.local/bin/otto
配置目录：  ~/.config/otto/
```

如果终端提示找不到 `otto`，可以只为当前 shell 添加路径：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### 3. 配置模型服务

首次使用前运行：

```bash
otto --config
```

按照提示输入：

- 服务名称：任意便于识别的名称。
- Base URL：模型服务地址。
- API Key：对应服务的密钥，输入时不会在终端显示。
- 模型名称：服务提供的实际模型名。

OTTO 按厂商选择 API 适配器。DeepSeek 会根据服务名称或域名自动选择 Anthropic Messages（Claude Code 兼容）接口；其他已识别厂商使用各自适配器，未识别的服务走 OpenAI Chat Completions 兼容兜底。Base URL 可以填写服务根地址或完整接口地址。例如：

```text
https://api.openai.com
https://api.siliconflow.cn/v1
https://api.siliconflow.cn/v1/chat/completions
https://api.deepseek.com
```

DeepSeek 适配器会将根地址映射到 `https://api.deepseek.com/anthropic/v1/messages`，并使用同一接口执行原生 Web Search。也可以把 Base URL 写成 `https://api.deepseek.com/anthropic`。

如果当前网络需要代理，OTTO 默认跟随系统代理设置，不提供独立的代理开关。Linux
GNOME 下，系统代理为 `none` 时直连，为 `manual` 时使用系统配置的 HTTP/HTTPS/SOCKS
代理和绕过列表；没有可读取的桌面代理设置时，才遵循标准的 `HTTP_PROXY`、
`HTTPS_PROXY`、`ALL_PROXY` 和 `NO_PROXY` 环境变量。HTTP(S) 以及 `socks4`、`socks5`、
`socks5h` 代理均可使用。

### 4. 开始使用

```bash
otto "解释一下这个项目是做什么的"
otto "帮我分析当前项目最近的错误"
```

直接运行源码构建出的程序时，也可以使用：

```bash
target/release/otto "你好"
```

## 常用命令

| 命令 | 用途 |
| --- | --- |
| `otto "问题"` | 提问并完成任务。 |
| `otto -h` / `otto --help` | 查看完整帮助。 |
| `otto -V` / `otto --version` | 查看版本。 |
| `otto -A "问题"` / `otto --no-agent "问题"` | 只进行普通问答，不让程序操作工具。 |
| `otto -S "问题"` / `otto --no-stdin "问题"` | 忽略管道或重定向传入的标准输入。 |
| `otto -r DIR "问题"` / `otto --root DIR "问题"` | 指定本次任务可以访问的项目目录。 |
| `otto -m NAME` / `otto --mode NAME` | 设置默认回答模式。 |
| `otto -m` / `otto --mode` | 清除默认模式。 |
| `otto -m NAME "问题"` / `otto --mode NAME "问题"` | 只为本次请求使用指定模式。 |
| `otto -m -- "问题"` / `otto --mode -- "问题"` | 本次请求不使用可选模式。 |
| `otto -c` / `otto --config` | 进入模型服务配置流程。 |

配置流程中的参数也支持简写：`-n/--name`、`-b/--baseurl`（兼容 `--base-url`）、
`-k/--apikey`（兼容 `--api-key`）和 `-M/--model`。带值选项支持空格和等号两种写法，
例如 `-b https://example.test/v1` 或 `-b=https://example.test/v1`。

问题参数可以直接写成多个单词，OTTO 会自动用空格拼接：

```bash
otto 请解释这个函数为什么返回错误
```

命令行选项必须写在问题之前。解析器遇到第一个问题参数后，就会进入问题模式；该参数以及后续所有参数都会被当作问题内容，用空格拼接，不再解析为 OTTO 选项。因此，下面的写法会完整询问 `python -m` 的含义：

```bash
otto python -m 这个指令是什么意思
```

问题开始后，即使后续参数看起来像选项，也会保留为问题文本：

```bash
otto 解释这段命令 --no-agent
```

如果问题的第一个参数本身以 `-` 开头，请先使用 `--` 结束选项：

```bash
otto -- -m 这个参数是什么意思
```

## 让 OTTO 使用文件和项目

默认情况下，OTTO 将当前目录作为本次任务的工作区。你可以让它查找文件、阅读代码、总结项目，或在授权后编辑文件：

```bash
otto "找出所有处理用户登录的文件，并说明调用关系"
otto --root ~/projects/demo "检查这个项目的配置问题"
```

`glob`、`grep` 和 `read` 属于默认允许的只读工具，不会在执行前询问；`edit` 和 `write` 修改 workspace 普通文件时会请求授权，`.otto/` 内的普通文件操作例外，详见下文。会话缓存目录 `.otto/sessions/` 由 OTTO 内部管理，通用文件和存储工具不能读取或修改它。`bash` 每次调用都会重新请求授权，包括看起来只读的命令。Bash 的同一次 `tool_call` 会同时返回完整脚本和风险字段（`risk`、`reason`、`breakdown`），授权界面据此展示风险摘要和命令结构；模型判断为可能修改或无法确定时，会用柔和的黄色提示。字段缺失或格式无效时按“无法确定”处理。命令和判断由同一次模型响应生成，因此判断只是可能出错的提示，不能代替用户授权。

`otto_storage` 用于管理当前 workspace 根目录下的 `.otto/`，可创建目录、列出、读取、创建、更新和删除其中的普通文件，路径均相对于 `.otto/`。该目录用于 OTTO 后续功能保存 workspace 局部数据，例如项目概览；在 `.otto/` 内使用此工具或 `edit`、`write` 时默认允许。会话数据保存在 `.otto/sessions/`，由 OTTO 独占管理，并自动从 Git 跟踪中排除。

### 保存和继续会话

普通调用不会保存对话。使用 `--new-session` 显式创建并保存会话：

```bash
otto --new-session "梳理这个项目的模块结构"
```

OTTO 会在回答后显示 session ID 和基于首轮对话生成的一句话 description。列出现有会话：

```bash
otto --session-list
otto --session
```

使用完整 ID 或唯一前缀继续会话：

```bash
otto --session 8f41a2c0 "接着分析配置加载流程"
```

会话按 workspace 隔离。同一 session 同一时间只允许一个 OTTO 请求；并发请求会提示稍后重试。存档保留完整消息历史；历史较长时 OTTO 会单独生成滚动摘要，并将最近的完整回合与摘要一起提供给模型。

Bash 以非交互方式运行，工作目录设为 workspace，最长运行 120 秒，并限制捕获的输出大小。一次批准覆盖整段脚本，包括管道、条件命令和子进程。workspace 不是沙箱：Bash 使用 OTTO 当前操作系统账户的权限，也可以访问 workspace 外的路径。时间和输出限制不能隔离命令副作用；主动脱离进程组的后台进程可能在 Bash 调用结束后继续运行。命令输出会返回给模型；涉及敏感内容时，请先查看完整命令和授权提示。Bash 工具不支持交互式终端程序。

交互式终端使用统一的紧凑布局，分别呈现模型状态、执行说明、工具活动、授权、配置和最终答复。最终答复通过管道输出时保持纯文本；状态与授权信息留在标准错误。配色柔和，并遵循 `NO_COLOR` 和 `TERM=dumb`。

## 联网搜索

`websearch` 默认优先调用已匹配的厂商原生联网搜索适配器；适配器不可用、响应没有结构化来源，
或原生请求失败时，才会使用配置的第三方搜索服务兜底。DeepSeek 使用 Anthropic Messages
接口中的 `web_search_20250305` 服务端工具，因此不需要另配搜索 API Key；搜索调用会产生
DeepSeek API 的额外模型 Token 费用。OpenAI、OpenRouter 和阿里云/Qwen 使用各自的搜索请求格式。
未匹配的厂商使用 OpenAI-compatible 搜索格式作为通用适配器；如果服务不支持或没有返回结构化来源，
会继续尝试已配置的第三方搜索服务，不会把普通模型回复当作搜索成功。

配置保存在 `$XDG_CONFIG_HOME/otto/search.env`，未设置 `XDG_CONFIG_HOME` 时默认是 `~/.config/otto/search.env`。

如果模型服务本身支持原生搜索，可以不创建此文件。需要第三方兜底时，以下配置任选一种：

Tavily：

```env
OTTO_SEARCH_PROVIDER=tavily
OTTO_TAVILY_API_KEY=你的 Tavily API Key
```

Brave Search：

```env
OTTO_SEARCH_PROVIDER=brave
OTTO_BRAVE_API_KEY=你的 Brave API Key
```

自建或可信的 SearXNG：

```env
OTTO_SEARCH_PROVIDER=searxng
OTTO_SEARCH_URL=https://your-searxng.example/search
```

搜索策略默认是原生优先、第三方兜底。如需临时或永久只使用第三方服务：

```env
OTTO_SEARCH_MODE=third-party
```

自动识别失败或使用自定义网关时，可以显式指定原生协议：

```env
# deepseek-claude-code、openai-chat、openrouter 或 alibaba-chat
OTTO_NATIVE_SEARCH_PROTOCOL=openai-chat
```

保存包含密钥的文件后，建议限制权限：

```bash
chmod 600 ~/.config/otto/search.env
```

配置完成后，直接提问即可：

```bash
otto "搜索 Rust 官方文档中关于异步运行时的说明"
```

OTTO 会在需要时使用搜索和网页阅读能力；网页内容仅作为资料提供，不会被当作操作指令执行。

## 回答模式

所有请求都会加载统一提示词 `system.md`。模式是可选的附加提示词，用来固定回答风格。

仓库内置的统一提示词和示例模式模板集中放在 `prompts/` 目录中；安装时会将它们复制到
配置目录。运行时仍从 `~/.config/otto/`（或 `OTTO_MODE_DIR` 指定的目录）读取提示词文件。

设置一个默认模式：

```bash
otto --mode otto
otto "帮我写一个简洁的提交说明"
```

只使用一次模式，不改变默认设置：

```bash
otto --mode jarvis "用更有角色感的方式回答"
```

清除默认模式：

```bash
otto --mode
```

安装时会提供示例模式。你也可以在配置目录中创建自己的模式文件，例如：

```bash
$EDITOR ~/.config/otto/reviewer.md
otto --mode reviewer
```

模式文件的内容会作为额外提示词附加到 `system.md` 后面。

### 项目指令（AGENT.md）

每次请求还会从 workspace 根目录向当前目录逐级查找名为 `AGENT.md` 的项目指令文件，按“根目录在前、越靠近当前目录越后”的顺序拼接。越靠近当前目录的文件可以补充更具体的规则，但不能覆盖统一系统提示词、OTTO Agent 的内置运行规则或用户问题。

workspace 根目录默认为启动 Otto 时的当前目录，也可以通过 `--root DIR` 指定。搜索不会越过 workspace 根目录；如果从 workspace 外启动并使用 `--root`，只加载根目录中的 `AGENT.md`。`AGENT.md` 对 Agent 和 `--no-agent` 两种请求都生效，且只在当前请求读取，不会被复制到配置目录。

项目指令文件必须是 UTF-8 普通文件；单个文件最多 128 KiB，单次请求合计最多 256 KiB。Otto 不会执行其中的内容，也不会把它们当作用户问题。文件缺失是正常情况；符号链接只能指向 workspace 内部的文件。

最终提示词顺序为：统一系统提示词、`AGENT.md` 项目指令、可选模式提示词；用户问题始终作为独立的用户消息发送。

### 运行时环境变量

Agent 每次请求默认最多执行 8 轮模型响应和工具调用。可以通过
`OTTO_MAX_AGENT_ROUNDS` 调整，允许范围是 0–255；设置为 0 表示不限制轮数：

```bash
export OTTO_MAX_AGENT_ROUNDS=16
otto "检查这个项目的实现"
```

为方便管理，可以复制仓库中的 `.otto_profile.example` 为 `~/.otto_profile`，
集中放置 Otto 的运行时环境变量，再在 `~/.profile` 或 `~/.bashrc` 中引用整个文件：

```bash
[ -f "$HOME/.otto_profile" ] && . "$HOME/.otto_profile"
```

profile 文件只建议存放非敏感设置；API Key 继续使用 `otto --config` 和
`~/.config/otto/search.env` 管理。当前支持的路径覆盖变量包括 `OTTO_CONFIG`、
`OTTO_MODE_DIR` 和 `OTTO_SEARCH_CONFIG`。

## 配置文件

默认配置位于：

```text
~/.config/otto/config       模型服务配置
~/.config/otto/search.env   联网搜索配置
~/.config/otto/system.md    统一提示词
~/.config/otto/<mode>.md    自定义回答模式
```

项目内的 `AGENT.md` 不安装到配置目录，由项目自行维护。

如果设置了 `XDG_CONFIG_HOME`，上述路径会相应改为 `$XDG_CONFIG_HOME/otto/`。

自动化场景也可以使用参数配置，但 API Key 会进入 shell 历史记录，日常使用推荐交互式配置：

```bash
otto --config \
  --name Openai \
  --baseurl https://api.openai.com \
  --apikey sk-xxxx \
  --model gpt-4o-mini
```

## 管道和脚本

回答输出到标准输出，错误输出到标准错误，可以直接交给其他命令：

```bash
otto "总结当前项目的 README" | tee answer.txt
ls -al | otto "这些文件分别是什么"
cat error.log | otto "分析这段日志并给出排查方向"
```

命令参数会作为问题，非终端标准输入会作为附加数据读取，并以明确边界传给模型；标准输入只作为待分析内容，不会被当作额外命令执行。若脚本已经处理过输入，可使用 `--no-stdin` 关闭读取。

工具需要授权时，OTTO 会在标准错误/控制终端显示授权界面。普通工具可以用上下键选择后按 Enter 确认，也可以按 `1`（仅本次）、`2`（本次运行中允许该工具后续执行）或 `3`（拒绝）。Bash 界面同样支持上下键和 Enter，初始选中“拒绝执行”；也可以按 `1` 仅允许屏幕上显示的整段脚本执行一次，或按 `2` 拒绝。授权信息不会混入标准输出。`glob`、`grep` 和 `read` 是默认允许的只读工具，不会显示此界面。找不到可交互终端时，OTTO 会拒绝操作。

## 安装、升级和卸载

从源码更新后，重新执行下面的命令即可升级：

```bash
make install
```

卸载 OTTO：

```bash
make uninstall
```

卸载只清理安装程序创建且未被用户修改的文件；已有的模型服务配置、当前模式和用户改过的提示词会保留。

## 常见问题

### `cargo not found`

Rust 已安装但当前 shell 找不到 Cargo 时，执行：

```bash
. "$HOME/.cargo/env"
```

然后重新运行 `make`。

### `otto: command not found`

确认 `~/.local/bin` 在当前 shell 的 `PATH` 中：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### websearch 无法使用

先确认当前模型和网关确实支持原生联网搜索；如果不支持，确认
`~/.config/otto/search.env` 中设置了一个第三方搜索服务和对应的 API Key 或 URL，
并检查文件权限。也可以检查是否误设置了 `OTTO_SEARCH_MODE=third-party` 或错误的
`OTTO_NATIVE_SEARCH_PROTOCOL`。

### API 请求失败

检查 `otto --config` 中的服务名称、Base URL、模型名称和 API Key 是否属于同一个服务。DeepSeek 会自动使用 Anthropic Messages 接口；其他厂商优先匹配专用适配器，未匹配时使用 OpenAI Chat Completions 兼容兜底。

## 开发者

编译正式版本：

```bash
make
```

运行完整测试：

```bash
make test
```

项目只维护 Rust 构建链；完整测试除 Rust 工具链和 `make` 外，还需要 Python 3
来运行本地 mock 服务和安装测试。
