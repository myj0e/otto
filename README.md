<p align="center">
  <img src="assets/otto-logo.png" alt="OTTO Logo" width="240">
  <div style="text-align:center; font-weight:bold; font-size:2.5rem;">
    One Time. Talk Once.
  </div>

</p>

OTTO 是一个运行在终端里的 AI 助手。你可以直接用自然语言提问，让它回答问题、解释代码、分析当前项目、处理文件，并在配置后搜索和阅读网页。

它适合快速完成一个明确的任务：每次命令都是独立请求，不会自动混入上一次调用的内容。需要读取或修改本地文件时，OTTO 会在相应操作前请求授权。

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

OTTO 使用 OpenAI Chat Completions 兼容接口。Base URL 可以填写服务根地址、`/v1` 地址，或完整的 `/chat/completions` 地址。例如：

```text
https://api.openai.com
https://api.siliconflow.cn/v1
https://api.siliconflow.cn/v1/chat/completions
```

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

读取和写入操作会分别请求授权；OTTO 只会在当前工作区范围内处理相对路径。

## 联网搜索

联网搜索需要单独配置搜索服务。OTTO 不会自动使用 Google；当前支持 Tavily、Brave Search 和 SearXNG。

配置保存在 `$XDG_CONFIG_HOME/otto/search.env`，未设置 `XDG_CONFIG_HOME` 时默认是 `~/.config/otto/search.env`。

以下配置任选一种，不要同时设置多个搜索服务。

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
```

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

确认 `~/.config/otto/search.env` 中设置了一个搜索服务和对应的 API Key 或 URL，并检查文件权限。

### API 请求失败

检查 `otto --config` 中的 Base URL、模型名称和 API Key 是否属于同一个服务，并确认该服务支持 OpenAI Chat Completions 兼容接口。

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
