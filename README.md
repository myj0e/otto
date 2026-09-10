# OTTO

OTTO（**One-time.Talk once**）是一个面向 CLI 的单轮大模型问答工具。

它适合解决一次对话即可完成的简单问题：

```bash
otto 你好
otto 请用一句话解释什么是 TCP
```

问题参数会在程序内部自动用空格拼接，因此通常不需要加引号。

请求默认启用 OpenAI Chat Completions 的 SSE 流式输出，模型生成一段就会立即显示一段，不需要额外配置。

## TODO

- [ ] 添加 Agent 工具调用能力：在回答前根据问题调用合适的工具，逐步扩展为支持生成文件等任务的单轮 Agent 会话。
- [ ] 添加音频模式：支持语音输出，输出 OTTO 的“活字印刷”语音。

## 系统提示词与语气模式

`system.md` 是统一系统提示词，每次请求都会加载，文件缺失时请求会报错。模式提示词是可选的附加内容，一次最多选择一个；没有选择模式时仍然会使用 `system.md`。

项目根目录的 `system.md` 和 `otto.md` 可以直接用于源码目录中的程序。如果配置目录没有同名文件，开发环境会回退读取项目根目录的文件；安装后可以复制到配置目录：

```bash
mkdir -p ~/.config/otto
cp system.md ~/.config/otto/system.md
cp otto.md ~/.config/otto/otto.md
cp jarvis.md ~/.config/otto/jarvis.md
```

模式文件与配置文件放在同一个目录中：

```text
~/.config/otto/system.md
~/.config/otto/otto.md
~/.config/otto/test.md
~/.config/otto/active_mode
```

其中 `system.md` 是必需的统一提示词，`otto.md`、`test.md` 等是可选的模式提示词。

先选择模式，选择结果会保存下来，之后普通的 `otto <问题>` 会自动使用它：

```bash
otto --mode otto
otto 你好
```

`otto --mode otto` 会保存 `otto` 模式；之后每次请求都会把 `system.md` 与 `otto.md` 合并后作为 system prompt。其他模式同理：

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

也可以只对当前请求临时指定模式或不附加模式，不改变已保存的模式：

```bash
otto --mode test 你好
otto --mode -- 你好
```

上面的 `otto --mode -- 你好` 仍然会加载 `system.md`，只是本次不附加任何模式提示词。

也可以通过 `OTTO_MODE_DIR` 指定模式文件目录。

## 流式输出

程序默认发送 `"stream": true`，并解析 `data:` SSE 事件。兼容 OpenAI Chat Completions 流式接口的服务可以直接使用，例如 SiliconFlow，不需要增加命令行参数。

## 依赖

- C11 编译器
- libcurl 开发库
- pkg-config

Ubuntu/Debian 示例：

```bash
sudo apt install build-essential pkg-config libcurl4-openssl-dev
```

## 编译与测试

```bash
make
make test
```

安装到 `/usr/local/bin`：

```bash
sudo make install
```

也可以指定安装目录：

```bash
make install PREFIX="$HOME/.local"
```

## 配置

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

## 以管道方式使用

回答输出到 stdout，错误输出到 stderr：

```bash
otto 总结这段文字 | tee answer.txt
```

如果问题以短横线开头，可以使用 `--`：

```bash
otto -- --help 是什么意思
```
