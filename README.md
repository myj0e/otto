<p align="center">
  <img src="assets/otto-logo.png" alt="OTTO Logo" width="240">
</p>


# One Time. Talk Once.

OTTO 是一个运行在终端里的 AI 助手。你可以直接用自然语言提问，让它回答问题、解释代码、分析当前项目、处理文件，并在配置后搜索和阅读网页。

它适合快速完成一个明确的任务：每次命令都是独立请求，不会自动混入上一次调用的内容。修改本地文件时，OTTO 会在操作前请求授权；查找和读取文件默认允许执行。

> 一个睿智的轮椅人，从眼前冲刺而过，带来智慧的哲言。但是记性不好的他，~~太阳升起时就把昨天忘掉~~下次会忘记上次的对话。

## 你可以用 OTTO 做什么

- 解释报错、命令和代码，帮助定位问题。
- 阅读当前项目中的文件，查找相关代码并总结结构。
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
| `otto --help` | 查看完整帮助。 |
| `otto --version` | 查看版本。 |
| `otto --no-agent "问题"` | 只进行普通问答，不让程序操作工具。 |
| `otto --no-stdin "问题"` | 忽略管道或重定向传入的标准输入。 |
| `otto --root DIR "问题"` | 指定本次任务可以访问的项目目录。 |
| `otto --mode NAME` | 设置默认回答模式。 |
| `otto --mode` | 清除默认模式。 |
| `otto --mode NAME "问题"` | 只为本次请求使用指定模式。 |
| `otto --mode -- "问题"` | 本次请求不使用可选模式。 |

问题参数可以直接写成多个单词，OTTO 会自动用空格拼接：

```bash
otto 请解释这个函数为什么返回错误
```

## 让 OTTO 使用文件和项目

默认情况下，OTTO 将当前目录作为本次任务的工作区。你可以让它查找文件、阅读代码、总结项目，或在授权后编辑文件：

```bash
otto "找出所有处理用户登录的文件，并说明调用关系"
otto --root ~/projects/demo "检查这个项目的配置问题"
```

`glob`、`grep` 和 `read` 属于默认允许的只读工具，不会在执行前询问；`edit` 和 `write` 修改文件时仍会请求授权。OTTO 只会在当前工作区范围内处理相对路径。

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

需要修改本地文件时，OTTO 会在标准错误/控制终端显示授权界面。可以用上下键选择后按 Enter 确认，也可以直接按 `1`（仅本次）、`2`（本次运行中允许该工具后续执行）或 `3`（拒绝）；授权信息不会混入标准输出。`glob`、`grep` 和 `read` 是默认允许的只读工具，不会显示此界面。

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

完整测试还需要 C 编译器、`pkg-config` 和 libcurl 开发库。Ubuntu/Debian 可以执行：

```bash
sudo apt install build-essential pkg-config libcurl4-openssl-dev
```
