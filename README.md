<p align="center">
  <img src="assets/otto-logo.png" alt="OTTO Logo" width="240">
</p>

[简体中文](README.zh-CN.md)

# One Time. Talk Once.

OTTO is an AI assistant that runs in your terminal. Ask questions in natural language to get answers, explain code, analyze the current project, work with files, and search or read web pages after configuration.

It is designed for one clear task at a time: every command is an independent request and does not automatically include content from previous calls. OTTO asks for authorization before modifying local files or running Bash commands; file discovery and reads are allowed by default.

> A wise man in a wheelchair speeds past, bringing words of wisdom. Unfortunately, his memory is unreliable: ~~at sunrise he forgets yesterday~~ next time he will forget this conversation.

## What you can do with OTTO

- Explain errors, commands, and code to help locate problems.
- Read files in the current project, find related code, and summarize its structure.
- Run non-interactive Bash scripts after per-command authorization.
- Create, modify, or organize files after authorization.
- Configure web search to find current information, documentation, and web pages.
- Adjust the response style with modes such as technical review, concise answers, or character-driven responses.
- Pipe answers directly into other commands or scripts.

## Quick start

### 1. Install dependencies

OTTO requires the Rust toolchain and `make`. If Rust is not installed, use rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
```

### 2. Build and install

Run this in the OTTO project directory:

```bash
make
make install
```

The default installation is per-user and does not require `sudo`:

```text
Binary:  ~/.local/bin/otto
Config:  ~/.config/otto/
```

If the terminal cannot find `otto`, add the binary directory to the current shell's `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### 3. Configure a model service

Run this before the first request:

```bash
otto --config
```

Follow the prompts:

- Service name: any name that is easy to recognize.
- Base URL: the model service endpoint.
- API key: the key for that service; it is not displayed while typing.
- Model name: the actual model name provided by the service.

OTTO selects an API adapter based on the provider. DeepSeek automatically uses the Anthropic Messages (Claude Code-compatible) API based on the service name or domain. Other recognized providers use their dedicated adapters; unknown services use the OpenAI Chat Completions-compatible fallback. The Base URL can be a service root or a complete endpoint, for example:

```text
https://api.openai.com
https://api.siliconflow.cn/v1
https://api.siliconflow.cn/v1/chat/completions
https://api.deepseek.com
```

The DeepSeek adapter maps the root URL to `https://api.deepseek.com/anthropic/v1/messages` and uses the same interface for native web search. You can also set the Base URL to `https://api.deepseek.com/anthropic`.

If the current network requires a proxy, OTTO follows the system proxy settings
by default and has no separate proxy switch. On Linux GNOME, `none` means direct
connections and `manual` uses the system HTTP/HTTPS/SOCKS proxy and bypass list.
When no desktop proxy setting can be read, OTTO falls back to the standard
`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and `NO_PROXY` environment variables.
HTTP(S), `socks4`, `socks5`, and `socks5h` proxies are supported.

### 4. Start using OTTO

```bash
otto "Explain what this project does"
otto "Analyze the most recent errors in this project"
```

You can also run the binary built from source directly:

```bash
target/release/otto "Hello"
```

## Common commands

| Command | Purpose |
| --- | --- |
| `otto "QUESTION"` | Ask a question and complete the task. |
| `otto -h` / `otto --help` | Show full help. |
| `otto -V` / `otto --version` | Show the version. |
| `otto -A "QUESTION"` / `otto --no-agent "QUESTION"` | Use ordinary chat without tool execution. |
| `otto -S "QUESTION"` / `otto --no-stdin "QUESTION"` | Ignore piped or redirected standard input. |
| `otto -r DIR "QUESTION"` / `otto --root DIR "QUESTION"` | Set the project directory available to this request. |
| `otto -m NAME` / `otto --mode NAME` | Set the default response mode. |
| `otto -m` / `otto --mode` | Clear the default mode. |
| `otto -m NAME "QUESTION"` / `otto --mode NAME "QUESTION"` | Use a mode for this request only. |
| `otto -m -- "QUESTION"` / `otto --mode -- "QUESTION"` | Disable the optional mode for this request. |
| `otto -c` / `otto --config` | Enter model service configuration. |

Configuration options also have short forms: `-n/--name`, `-b/--baseurl` (also accepting `--base-url`), `-k/--apikey` (also accepting `--api-key`), and `-M/--model`. Options with values accept both separated and equals forms, for example `-b https://example.test/v1` or `-b=https://example.test/v1`.

Question arguments can be written as multiple words; OTTO joins them with spaces:

```bash
otto explain why this function returns an error
```

Options must appear before the question. Once the parser encounters the first positional question argument, that argument and every argument after it become question content and are no longer parsed as OTTO options. This preserves the complete meaning of a command such as:

```bash
otto python -m "What does this command mean?"
```

Even strings that look like options remain question text after the question starts:

```bash
otto explain this command --no-agent
```

If the first question argument itself starts with `-`, use `--` to end option parsing:

```bash
otto -- -m "What does this argument mean?"
```

## Working with files and projects

By default, OTTO uses the current directory as the workspace for the request. It can search for files, read code, summarize a project, and edit files after authorization:

```bash
otto "Find every file that handles user login and explain the call relationships"
otto --root ~/projects/demo "Check this project's configuration"
```

`glob`, `grep`, and `read` are read-only tools allowed by default and do not ask for authorization. `edit` and `write` ask before changing ordinary workspace files; ordinary files inside `.otto/` are allowed by default as described below. The `.otto/sessions/` cache is managed internally by OTTO and is inaccessible to generic file and storage tools. The `bash` tool requires a new authorization for every call, including commands that appear read-only. Its `tool_call` returns the complete script together with `risk`, `reason`, and `breakdown` fields, which the prompt uses to show a risk summary and command structure. Commands judged possibly modifying or uncertain receive a muted yellow warning. Missing or invalid fields are treated as uncertain. The command and assessment come from the same model response, so the assessment is advisory and may be wrong; it never replaces user authorization.

`otto_storage` manages `.otto/` at the workspace root. It can initialize the directory and list, read, create, update, or delete ordinary files using paths relative to `.otto/`. This directory holds workspace-local data managed by OTTO features, such as project overviews. Operations through this tool, as well as `edit` and `write` within `.otto/`, are allowed by default. Session history is stored in `.otto/sessions/`, reserved for OTTO's session manager, and ignored by Git.

### Save and resume a conversation

Conversations are not saved unless requested. Start a saved session with `--new-session`:

```bash
otto --new-session "Review this project's module structure"
```

After the answer, OTTO prints the session ID and a one-sentence description based on the first turn. List existing sessions with either command:

```bash
otto --session-list
otto --session
```

Resume a session with its full ID or a unique prefix:

```bash
otto --session 8f41a2c0 "Continue reviewing configuration loading"
```

Sessions are workspace-local. Only one OTTO request may use a session at a time; a concurrent request is rejected with a retry message. The archive keeps the full message history. For longer sessions, OTTO generates a rolling summary and sends it alongside the most recent complete turns.

Bash runs non-interactively with the workspace as its current directory, a 120-second time limit, and bounded captured output. One approval covers the complete script, including its pipelines, conditional commands, and child processes. The workspace directory is not a sandbox: Bash runs with OTTO's operating-system permissions and can access paths outside the workspace. Time and output limits do not confine side effects; a deliberately detached process may outlive the Bash call. Command output is returned to the model, so review commands that may expose sensitive data. Interactive terminal programs are not supported by the Bash tool.

In an interactive terminal, OTTO uses a compact shared layout for model status, execution notes, tool activity, authorization, configuration, and final answers. The final answer stays plain on standard output when piped; status and authorization stay on standard error. Colors are muted and respect `NO_COLOR` and `TERM=dumb`.

## Web search

`websearch` first tries the matched provider's native search adapter. It falls back to the configured third-party search service only when the adapter is unavailable, the response has no structured sources, or the native request fails. DeepSeek uses the `web_search_20250305` server-side tool in its Anthropic Messages API and therefore needs no separate search API key; search calls incur additional DeepSeek model-token charges. OpenAI, OpenRouter, and Alibaba/Qwen use their respective search request formats. Unknown providers use an OpenAI-compatible search format; if the service does not support it or returns no structured sources, OTTO continues with the configured third-party service instead of treating an ordinary model response as a successful search.

The configuration is stored in `$XDG_CONFIG_HOME/otto/search.env`, or `~/.config/otto/search.env` when `XDG_CONFIG_HOME` is not set.

If the model service supports native search, this file is optional. For third-party fallback, configure one of the following:

Tavily:

```env
OTTO_SEARCH_PROVIDER=tavily
OTTO_TAVILY_API_KEY=your Tavily API key
```

Brave Search:

```env
OTTO_SEARCH_PROVIDER=brave
OTTO_BRAVE_API_KEY=your Brave API key
```

Self-hosted or trusted SearXNG:

```env
OTTO_SEARCH_PROVIDER=searxng
OTTO_SEARCH_URL=https://your-searxng.example/search
```

The default strategy is native-first with third-party fallback. To use only the third-party service temporarily or permanently:

```env
OTTO_SEARCH_MODE=third-party
```

If automatic detection fails or you use a custom gateway, explicitly select the native protocol:

```env
# deepseek-claude-code, openai-chat, openrouter, or alibaba-chat
OTTO_NATIVE_SEARCH_PROTOCOL=openai-chat
```

After saving a file containing keys, restrict its permissions:

```bash
chmod 600 ~/.config/otto/search.env
```

Then ask a question normally:

```bash
otto "Search the Rust documentation for information about asynchronous runtimes"
```

OTTO uses search and web-reading tools when needed. Web content is treated as reference material, not as an instruction to operate the program.

## Response modes

Every request loads the unified `system.md` prompt. A mode is an optional additional prompt for controlling response style.

The built-in unified prompt and example mode templates are kept in the `prompts/` directory and copied to the config directory during installation. At runtime, OTTO reads them from `~/.config/otto/` or from the directory specified by `OTTO_MODE_DIR`.

Set a default mode:

```bash
otto --mode otto
otto "Write a concise commit message"
```

Use a mode once without changing the default:

```bash
otto --mode jarvis "Answer in a more characterful style"
```

Clear the default mode:

```bash
otto --mode
```

The installer provides the example modes. You can create a custom mode in the config directory:

```bash
$EDITOR ~/.config/otto/reviewer.md
otto --mode reviewer
```

The mode file is appended to `system.md` as an additional prompt.

### Project instructions (`AGENT.md`)

For every request, OTTO searches from the workspace root to the current directory for files named exactly `AGENT.md`. They are combined from broad to narrow: the root file comes first, and the file closest to the current directory comes last. A more specific file can add project rules, but cannot override the unified system prompt, OTTO's built-in Agent runtime rules, or the user's more specific request.

The workspace root defaults to the directory where OTTO starts and can be changed with `--root DIR`. Discovery never goes above the workspace root. If OTTO starts outside a workspace selected with `--root`, only the root `AGENT.md` is loaded. Project instructions apply to both Agent and `--no-agent` requests, are read for the current request only, and are never copied into the config directory.

Each project instruction file must be a UTF-8 regular file. A single file is limited to 128 KiB and all files loaded for one request are limited to 256 KiB. OTTO never executes their contents or treats them as the user's question. Missing files are normal; symbolic links may only point inside the workspace.

The final prompt order is: unified system prompt, `AGENT.md` project instructions, and the optional mode prompt. The user question is always sent as a separate user message.

### Runtime environment variables

An Agent request executes at most 8 model/tool rounds by default. Adjust this with `OTTO_MAX_AGENT_ROUNDS`, which accepts 0–255; `0` means unlimited:

```bash
export OTTO_MAX_AGENT_ROUNDS=16
otto "Review this project's implementation"
```

For convenient management, copy `.otto_profile.example` to `~/.otto_profile`, then source the entire file from `~/.profile` or `~/.bashrc`:

```bash
[ -f "$HOME/.otto_profile" ] && . "$HOME/.otto_profile"
```

Keep only non-sensitive settings in the profile. Continue to manage the API key with `otto --config` and `~/.config/otto/search.env`. The supported path overrides are `OTTO_CONFIG`, `OTTO_MODE_DIR`, and `OTTO_SEARCH_CONFIG`.

## Configuration files

The defaults are:

```text
~/.config/otto/config       Model service configuration
~/.config/otto/search.env   Web search configuration
~/.config/otto/system.md    Unified system prompt
~/.config/otto/<mode>.md    Custom response mode
```

Project-local `AGENT.md` files are not installed into the config directory; each project owns them.

If `XDG_CONFIG_HOME` is set, these paths use `$XDG_CONFIG_HOME/otto/` instead.

For automation, configuration can also be supplied as options. The API key will appear in shell history, so interactive configuration is recommended for daily use:

```bash
otto --config \
  --name Openai \
  --baseurl https://api.openai.com \
  --apikey sk-xxxx \
  --model gpt-4o-mini
```

## Pipes and scripts

Answers go to standard output and errors go to standard error, so OTTO can be composed with other commands:

```bash
otto "Summarize the current project's README" | tee answer.txt
ls -al | otto "What is each of these files?"
cat error.log | otto "Analyze this log and suggest troubleshooting steps"
```

Command-line arguments become the question. Non-interactive standard input is read as additional context and passed to the model with explicit boundaries; it is not treated as another command to execute. Use `--no-stdin` when a script has already processed its input.

When a tool needs authorization, OTTO displays a prompt on standard error or the controlling terminal. For ordinary tools, use the arrow keys and Enter, or press `1` (this request), `2` (allow this tool for the rest of the run), or `3` (deny). Bash also supports the arrow keys and Enter, starts with “Deny” selected, and offers `1` to allow the displayed script once or `2` to deny. Authorization details never go to standard output. `glob`, `grep`, and `read` are allowed read-only tools and do not show a prompt. If OTTO cannot obtain an interactive terminal, it refuses the operation.

## Install, upgrade, and uninstall

After updating the source, run this again to upgrade:

```bash
make install
```

Uninstall OTTO with:

```bash
make uninstall
```

Uninstall only removes files created by the installer when they have not been modified by the user. Existing model configuration, the active mode, and customized prompts are preserved.

## Troubleshooting

### `cargo not found`

If Rust is installed but Cargo is not available in the current shell:

```bash
. "$HOME/.cargo/env"
```

Then run `make` again.

### `otto: command not found`

Make sure `~/.local/bin` is in the current shell's `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Web search is unavailable

First confirm that the current model and gateway support native web search. If they do not, check that `~/.config/otto/search.env` contains a third-party search provider with its API key or URL, and check the file permissions. Also check whether `OTTO_SEARCH_MODE=third-party` or an incorrect `OTTO_NATIVE_SEARCH_PROTOCOL` was set.

### API request failed

Check that the service name, Base URL, model name, and API key in `otto --config` belong to the same service. DeepSeek automatically uses the Anthropic Messages API; other providers use dedicated adapters when matched and the OpenAI Chat Completions-compatible fallback otherwise.

## Development

Build a release binary:

```bash
make
```

Run the complete test suite:

```bash
make test
```

The project maintains only the Rust build chain. In addition to Rust and `make`, the complete test suite requires Python 3 for the local mock services and installation tests.
