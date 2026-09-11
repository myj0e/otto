# Rust 迁移方案

otto 从 C v0.1 迁移到 Rust，目标是保留现有 CLI 和配置兼容性，同时为 Agent、
本地文件工具以及 websearch/webfetch 建立可扩展的核心架构。

## 当前状态

M0–M7 已完成，M8 正式切换也已落地。Rust `1.0.0` 现在是默认构建和安装版本，
普通请求默认进入 Agent loop；C11/libcurl 版本作为 v0.1 基线保留。

构建入口：

```text
make                         # 构建正式 Rust release binary
make rust-build              # 同上
make c-build                 # 显式构建 C 基线 build/c/otto-c
cargo test --manifest-path rust/otto/Cargo.toml
```

`make install` 和 `make uninstall` 是正式生命周期入口；旧的安装脚本仍作为兼容入口保留。

## 迁移原则

1. 先保留 C 版本，再逐模块迁移。
2. 先迁移已实现行为，再增加 Agent 能力。
3. 旧配置、旧提示词和旧命令保持兼容。
4. 每个阶段都要有独立测试和可回退提交。
5. Rust 版本完成兼容性验证后，正式入口、安装入口和版本号一起切换。
6. C 版本的历史实现通过 `c-v0.1.0` 和 `legacy/c-v0.1` 保留。

## 目标架构

```text
otto-cli
  ├── cli
  ├── config
  ├── prompt
  ├── http
  │   ├── chat
  │   └── sse
  ├── agent
  │   ├── loop
  │   ├── message
  │   ├── registry
  │   └── limits
  ├── permission
  ├── workspace
  ├── tools
  │   ├── glob
  │   ├── grep
  │   ├── read
  │   ├── edit
  │   ├── write
  │   ├── websearch
  │   └── webfetch
  └── providers
      ├── brave
      ├── searxng
      └── tavily
```

工具通过统一接口注册：

```text
工具定义：名称、描述、JSON 参数 schema、能力类别
工具执行：参数校验、权限检查、实际执行、结构化结果
工具结果：成功、拒绝、参数错误、运行错误
```

Agent 主循环不直接依赖具体工具。新增工具只需要新增模块、实现接口并注册。

## Rust 技术选择

第一阶段推荐依赖：

当前代码实际使用 tokio、reqwest、serde、serde_json、futures-util、libc、thiserror、
tempfile、rpassword、async-trait、walkdir、globset、regex、similar、sha2、url 和
scraper。CLI 先保留手写解析器，以便精确兼容 C 版本的 mode 和问题参数拼接行为；
后续如果参数继续扩展，再评估引入 clap。

```text
clap          CLI
tokio         异步运行时
reqwest       HTTP、TLS、SSE 数据流
serde         类型序列化
serde_json    JSON
futures-util  流式处理
url            URL 解析
thiserror     错误类型
sha2          文件哈希
tempfile      临时文件和原子写入
walkdir       文件遍历
globset       Glob 匹配
regex         Grep
similar       diff
dialoguer     交互式配置和授权
rpassword     隐藏输入
scraper       HTML DOM 解析
```

网络层不再强制绑定 libcurl。C 版本继续使用 libcurl，Rust 版本使用 reqwest，
以获得更直接的流式、TLS、重定向和异步接口。

## 分阶段路线

### M0：基线留档

- 创建 `c-v0.1.0`。
- 创建 `legacy/c-v0.1`。
- 保存 `docs/legacy/c-v0.1.md`。
- 创建 `rust-migration` 分支。

### M1：Rust 双构建骨架

- 创建 Cargo workspace。
- 创建 `rust/otto` package。
- 加入 Rust CLI 入口、版本和帮助信息。
- 不改变 C 的默认 `make` 构建。

### M2：迁移基础行为

- CLI 参数解析。
- 配置读写和 `0600` 权限。
- `OTTO_CONFIG`、XDG 路径规则。
- system prompt 和 mode 加载。
- `active_mode`。
- 保持问题参数拼接行为。

### M3：迁移 Chat 和 SSE

- OpenAI Chat Completions 请求。
- 普通流式文本输出。
- HTTP 错误和 API 错误解析。
- 超时和响应大小限制。
- 与 C 版本进行请求和输出回归比较。

### M4：Agent 核心

- Agent 消息模型（当前用 serde_json 保持 OpenAI 兼容字段可扩展）。
- `Tool` trait 和 `ToolRegistry`。
- `tools` 请求字段。
- `tool_calls` 响应解析。
- `role=tool` 结果消息。
- 最大 Agent 轮数和工具调用次数。
- 默认 `tool_choice=auto`。

### M5：本地工具和权限

按顺序实现：

```text
Glob → Grep → Read → 权限 → edit → write → hash/diff/atomic write
```

默认 workspace 为当前目录，支持 `--root`。读取权限和写入权限分开，权限只在
当前进程和当前会话中有效。

### M6：websearch

- 定义 `SearchProvider` trait。
- 实现 Brave provider。
- 实现 SearXNG provider。
- 实现 Tavily provider，并默认使用 `basic` 搜索深度控制额度消耗。
- 由原生 `otto` 读取 `search.env`，不依赖 shell 启动器传递配置。
- 统一搜索结果结构。
- 保存当前会话的 `search_result_id` 到 URL 映射。

### M7：webfetch

- HTTP/HTTPS URL 校验。
- SSRF 和内网地址防护。
- 手动重定向限制。
- 响应大小和超时限制。
- HTML 正文提取。
- 来源 URL、标题和截断标记。
- 将网页内容作为不可信工具数据交给模型。

### M8：正式替换（已完成）

- Rust 普通问答和 Agent 测试全部通过。
- `make` 默认构建 Rust。
- `make install` / `make uninstall` 成为正式安装和卸载入口。
- 安装脚本切换到 Rust release binary。
- 安装清单升级为 v2，同时兼容 v1。
- 保留 `config`、`active_mode` 和用户修改过的 mode。
- C 源码保留在现有位置，通过 `make c-build` / `make c-test` 显式维护；历史 tag 不变。

## 兼容性验收

Rust 版本必须通过：

当前 Rust 单元测试覆盖 CLI 参数、SSE 分片、tool call 拼接、workspace 路径穿越、
符号链接逃逸、工具输出截断和网页正文清洗；功能测试覆盖普通 Chat、提示词模式、
安装、v1 清单升级和安全卸载。真实 provider 仍需要用户在自己的 API 环境中验证。

- 帮助、版本和错误参数测试。
- 配置文件读写和 API Key 隐藏输入测试。
- 模式切换、清除、单次 mode 测试。
- 多参数问题拼接测试。
- SSE 分片测试。
- 普通 Chat 请求回归测试。
- Agent 无工具、搜索后抓取、连续多轮工具调用测试。
- 读取拒绝、读取本轮授权、写入拒绝和写入授权测试。
- 路径穿越、符号链接逃逸、文件 hash 冲突测试。
- URL 重定向到本地地址的 SSRF 测试。
- HTML 脚本删除、正文截断和网页注入内容隔离测试。
- 安装、升级和卸载保护测试。

## 版本计划

```text
0.1.x   C 单轮问答基线
0.2.0   Rust 普通问答兼容版
0.3.0   Rust Agent Loop
0.4.0   本地工具和授权
0.5.0   websearch
0.6.0   webfetch
1.0.0   Rust Agent 正式版（当前）
```
