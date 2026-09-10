# C 版本基线：otto v0.1

本文档记录 Rust 迁移前的 C 版本行为。当前基线提交为
`4c0b18f docs: formalize README`，Git tag 为 `c-v0.1.0`，回溯分支为
`legacy/c-v0.1`。

## 定位

这一版是一个使用 C11 和 libcurl 实现的单轮问答 CLI：一次进程只发送一次
Chat Completions 请求，不保存跨进程的对话历史，也不执行工具调用。

## 已实现功能

### 命令行

- `otto <问题内容...>`：把多个参数用单个空格拼接后作为完整问题。
- `otto --help`：显示帮助。
- `otto --version`：显示版本。
- `otto --config`：在交互式终端中配置服务。
- `otto --config --name ... --baseurl ... --apikey ... [--model ...]`：参数式配置。
- `otto --mode <模式>`：保存默认模式。
- `otto --mode`：清除默认附加模式。
- `otto --mode <模式> <问题...>`：只对本次请求附加模式。
- `otto --mode -- <问题...>`：只对本次请求禁用附加模式。

### 模型请求

- 使用 OpenAI Chat Completions 兼容接口。
- 通过 `Authorization: Bearer ...` 发送 API Key。
- 请求默认使用 `stream: true`。
- 解析 Chat Completions SSE 的 `data:` 和 `[DONE]`。
- 支持 SiliconFlow 等 OpenAI 兼容服务，前提是服务接受当前请求格式。

### 提示词模式

- 每次请求必须加载 `system.md`。
- 可选地附加一个 `<mode>.md`。
- `active_mode` 保存默认模式名称。
- `OTTO_MODE_DIR` 可覆盖提示词读取目录。
- 配置目录找不到提示词时，会按现有规则回退到项目目录。
- 已提供 `otto.md` 和 `jarvis.md` 示例模式。

### 配置和安装

- 配置默认位于 `$XDG_CONFIG_HOME/otto/config` 或 `~/.config/otto/config`。
- `OTTO_CONFIG` 可以直接覆盖配置文件路径。
- 配置保存采用临时文件、`fsync` 和原子替换。
- 配置文件权限为 `0600`，配置目录权限为 `0700`。
- 当前用户安装脚本使用安装清单和校验和保护已有文件。
- 卸载脚本只删除清单记录且内容未被用户修改的文件。
- API 配置、`active_mode` 和用户修改过的提示词不会被卸载脚本删除。

### 测试

- CLI 帮助、版本和错误参数。
- 交互式配置和 API Key 隐藏输入。
- 参数拼接、普通 SSE 响应和提示词组合。
- 模式切换、清除模式和单次模式。
- 安装碰撞保护、文件校验和安全卸载。

## 尚未实现功能

以下内容不属于 C v0.1 的实现范围：

- Agent Loop 和多轮工具调用。
- OpenAI `tools`、`tool_calls` 和 `role=tool` 消息。
- `Glob`、`Grep`、`Read`。
- `edit`、`write`。
- 本地文件读取和写入授权。
- `websearch`、Brave provider、SearXNG provider。
- `webfetch`、HTML 正文提取和来源追踪。
- SSRF、重定向和网页提示词注入防护。
- Agent 工具调用 SSE 分片解析。
- 音频模式。

## 已知限制

- JSON 解析器是项目内的定制实现，主要覆盖当前 Chat 响应格式，尚未覆盖工具调用和搜索 provider 的完整 JSON 结构。
- HTTP 层主要面向 Chat Completions POST，请求抽象还没有独立成通用 GET/POST 客户端。
- 当前普通请求没有 Agent 的轮数、工具次数和工具结果限制。
- C 代码依赖 POSIX 文件、终端和配置接口，跨平台支持有限。
- 在受限沙箱中运行测试时，Python mock server 可能因为禁止监听本地端口而无法启动；这属于测试环境限制。

## 兼容性契约

Rust 版本完成替换时必须保持以下行为：

```text
otto <问题内容...>
otto --help
otto --version
otto --config
otto --mode otto
otto --mode
OTTO_CONFIG
OTTO_MODE_DIR
system.md
otto.md
jarvis.md
active_mode
config 文件格式
SSE 默认输出
安装和卸载的用户文件保护规则
```

Rust 版本可以增加 Agent 和工具能力，但不能要求已有用户重新创建配置或修改
已有提示词文件。
