# Mini Agent Loop — 详细日志版运行指南

> **读者画像**：你是一个新手，想通过日志输出来理解 AI Agent 的完整运行过程。
> 本文档会告诉你如何运行、如何看懂每一条日志、以及每条日志背后发生了什么。

---

## 目录

1. [这个版本和原版有什么区别？](#1-这个版本和原版有什么区别)
2. [快速开始](#2-快速开始)
3. [日志级别详解](#3-日志级别详解)
4. [完整运行示例：一次数学问题的全过程](#4-完整运行示例)
5. [逐条日志解读](#5-逐条日志解读)
6. [日志埋点地图：每个文件加了什么日志](#6-日志埋点地图)
7. [高级用法：模块级过滤](#7-高级用法)
8. [日志保存与分析](#8-日志保存与分析)
9. [常见问题](#9-常见问题)

---

## 1. 这个版本和原版有什么区别？

`mini-agent-loop-log` 是 `mini-agent-loop` 的**日志增强版**。功能完全相同，但在每个关键环节都添加了 `tracing` 结构化日志，让你能像"X 光"一样看穿整个 Agent 的运行过程。

### 新增的依赖

```toml
# Cargo.toml 中新增：
tracing = "0.1"                                              # 日志宏（info!, debug!, trace! 等）
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }  # 日志输出 + 环境变量过滤
```

### 改动的文件

| 文件 | 新增日志数量 | 覆盖的关键环节 |
|------|-------------|---------------|
| `src/main.rs` | ~25 条 | 启动、配置加载、用户输入、模型切换、消息构建、循环结果 |
| `src/agent.rs` | ~20 条 | 迭代计数、LLM 调用耗时、响应类型判断、工具执行、上下文变化 |
| `src/llm/openai.rs` | ~20 条 | HTTP 请求体、响应体、状态码、JSON 解析、tool call 解析 |
| `src/tools/mod.rs` | ~15 条 | 工具注册、查找、执行超时、结果序列化、结果包装 |
| `src/tools/calculator.rs` | ~8 条 | 参数解析、运算过程、计算结果 |

### 核心原理：tracing 是什么？

`tracing` 是 Rust 生态中最流行的结构化日志库。它的核心思想是：

```rust
// 传统 println! — 只有文字，无法过滤
println!("Calling LLM with {} messages", messages.len());

// tracing — 结构化字段 + 级别控制 + 模块过滤
info!(
    message_count = messages.len(),   // 结构化字段，可被日志系统索引
    model = %self.model,              // % 表示用 Display trait 格式化
    "📤 Calling LLM"                  // 人类可读的消息
);
```

好处：
- **可以通过环境变量动态控制**：不改代码就能开关日志
- **有级别**：`trace` < `debug` < `info` < `warn` < `error`
- **有模块路径**：可以只看某个模块的日志
- **有结构化字段**：不只是文字，还有可查询的键值对

---

## 2. 快速开始

### 前置条件

1. 已安装 Rust（`rustup` + `cargo`）
2. 已配置 `HAI_WOA.json`（从 `HAI_WOA.template.json` 复制并填入 API Key）

### 三种运行方式

```bash
cd /Users/ericksun/workspace/ironclaw/mydocs/mini-agent-loop-log

# ① 默认运行（info 级别）— 只看关键事件
cargo run

# ② 详细模式（debug 级别）— 推荐新手使用 ⭐
RUST_LOG=debug cargo run

# ③ 最详细模式（trace 级别）— 看一切，包括完整 JSON
RUST_LOG=trace cargo run
```

### 推荐新手的第一次运行

```bash
# 用 debug 级别运行，同时保存到文件方便回看
RUST_LOG=debug cargo run 2>&1 | tee debug_run.log
```

然后输入 `What is 42 + 58?`，你会看到完整的 Agent 处理过程。

---

## 3. 日志级别详解

### 级别对照表

| 级别 | 环境变量 | 适合谁 | 看到什么 |
|------|---------|--------|---------|
| `error` | `RUST_LOG=error` | 只看错误 | API 失败、工具执行错误、解析失败 |
| `warn` | `RUST_LOG=warn` | 看异常 | 截断响应、未知 finish_reason、超时 |
| `info` | `RUST_LOG=info` | **默认** | 🚀启动、📝用户输入、📤LLM调用、🔧工具执行、✅结果 |
| `debug` | `RUST_LOG=debug` | **新手推荐** | 完整请求摘要、响应内容、工具参数和结果、耗时统计 |
| `trace` | `RUST_LOG=trace` | 深度调试 | 完整 JSON 请求体/响应体、每个变量值、消息上下文全貌 |

### 日志输出格式说明

每条日志的格式如下：

```
2024-01-15T10:30:45.123456Z  INFO main ThreadId(01) mini_agent_loop::agent: 📤 Step 1: Calling LLM... message_count=2 tool_count=1
│                             │    │                  │                       │                        └── 结构化字段
│                             │    │                  │                       └── 人类可读消息
│                             │    │                  └── 模块路径（哪个文件产生的）
│                             │    └── 线程名
│                             └── 日志级别
└── 时间戳（精确到微秒）
```

---

## 4. 完整运行示例

以下是用 `RUST_LOG=debug cargo run` 运行，输入 `What is 42 + 58?` 的完整日志流程。

### 阶段 1：启动与初始化

```
 INFO mini_agent_loop: 🚀 Starting Mini Agent Loop — tracing initialized
╔══════════════════════════════════════════════════════════════╗
║           Mini Agent Loop — IronClaw Core Distilled         ║
╚══════════════════════════════════════════════════════════════╝

 INFO mini_agent_loop: 📂 Loading LLM configuration from ./HAI_WOA.json ...
DEBUG mini_agent_loop::llm::openai: Loading config file path="./HAI_WOA.json"
DEBUG mini_agent_loop::llm::openai: Config parsed: default model='DeepSeek-V3', 5 model shortcuts
 INFO mini_agent_loop: ✅ Config loaded successfully model=DeepSeek-V3 base_url=https://api.example.com/v1
DEBUG mini_agent_loop: Available model shortcuts available_models=[("ds", "DeepSeek-V3"), ...]
 INFO mini_agent_loop: 🤖 Initial model selected current_model=DeepSeek-V3
DEBUG mini_agent_loop: Creating new LLM provider instance model=DeepSeek-V3
 INFO mini_agent_loop::llm::openai: Creating OpenAI-compatible provider (timeout=120s) model=DeepSeek-V3

 INFO mini_agent_loop::tools: Registering tool: 'calculator' tool_name=calculator
DEBUG mini_agent_loop::tools: Tool details tool_name=calculator description="Perform basic arithmetic..."
 INFO mini_agent_loop: 🔧 Tool registry initialized tools=["calculator"]
 INFO mini_agent_loop: ⚙️  Agentic loop config max_iterations=10
 INFO mini_agent_loop: 🔄 Entering main input loop — waiting for user input...
```

**解读**：
- 程序启动，初始化 tracing 日志系统
- 从 `HAI_WOA.json` 加载配置（API Key、Base URL、模型列表）
- 创建 LLM Provider（HTTP 客户端，120 秒超时）
- 注册 calculator 工具到 ToolRegistry
- 设置 agentic loop 最大迭代次数为 10

### 阶段 2：用户输入

```
🧑 You: What is 42 + 58?

 INFO mini_agent_loop: 📝 User input received input="What is 42 + 58?"
 INFO mini_agent_loop: 📋 Building conversation context (system prompt + user message)
DEBUG mini_agent_loop: Initial messages for agentic loop
    system_prompt="You are a helpful assistant. You have access to a calculator tool..."
    user_message="What is 42 + 58?"
    message_count=2
```

**解读**：
- 用户输入被捕获
- 构建消息上下文：1 条 system prompt（不含模型名，模型自己知道自己的身份）+ 1 条 user message = 2 条消息

### 阶段 3：进入 Agentic Loop

```
 INFO mini_agent_loop: 🧠 Starting agentic loop (model=DeepSeek-V3, max_iterations=10)
 INFO mini_agent_loop::agent: 🔄 Entering agentic loop tool_count=1 tools=["calculator"] initial_message_count=2

🤖 Agent thinking...
```

**解读**：
- Agentic Loop 启动，携带 1 个工具定义和 2 条初始消息

### 阶段 4：第一次 LLM 调用（LLM 决定调用工具）

```
 INFO mini_agent_loop::agent: ━━━ Iteration 1/10 ━━━ iteration=1 max=10 message_count=2
 INFO mini_agent_loop::agent: 📤 Step 1: Calling LLM...
DEBUG mini_agent_loop::agent: Sending 2 messages and 1 tool definitions to LLM

 INFO mini_agent_loop::llm::openai: 📤 Preparing LLM API request url=".../chat/completions" model=DeepSeek-V3 message_count=2 tool_count=1
DEBUG mini_agent_loop::llm::openai: Request summary: 2 messages [system → user], 1 tools
 INFO mini_agent_loop::llm::openai: 🌐 Sending HTTP POST to .../chat/completions...
 INFO mini_agent_loop::llm::openai: 📥 HTTP response received: 200 OK in 1234ms status=200 elapsed_ms=1234

DEBUG mini_agent_loop::llm::openai: Parsed response: 1 choice(s)
DEBUG mini_agent_loop::llm::openai: Response choice: finish_reason=tool_calls, has_content=false, has_tool_calls=true
 INFO mini_agent_loop::llm::openai: 🔧 LLM requested 1 tool call(s)
DEBUG mini_agent_loop::llm::openai: Parsing tool call [0]: calculator (id=call_abc123)
    raw_arguments="{\"operation\":\"add\",\"a\":42,\"b\":58}"

 INFO mini_agent_loop::agent: 📥 LLM responded in 1234ms (finish_reason=ToolUse) elapsed_ms=1234
 INFO mini_agent_loop::agent: 🔧 LLM returned TOOL CALLS (1 tool(s)) tool_call_count=1
DEBUG mini_agent_loop::agent: Tool call [0]: calculator (id=call_abc123)
    arguments={"operation": "add", "a": 42, "b": 58}
```

**解读**：
- **Iteration 1** 开始
- 向 LLM API 发送 HTTP POST 请求（2 条消息 + 1 个工具定义）
- LLM 返回 200 OK，耗时 1234ms
- LLM 的 `finish_reason` 是 `tool_calls` — 它决定调用 calculator 工具
- 工具参数：`operation=add, a=42, b=58`

### 阶段 5：执行工具

```
 INFO mini_agent_loop::agent: 📝 Adding assistant message with tool calls to conversation context
DEBUG mini_agent_loop::agent: Context now has 3 messages (after adding assistant tool call message)

 INFO mini_agent_loop::agent: ⚙️  Executing tool [1/1]: calculator
 INFO mini_agent_loop::tools: 🔍 Looking up tool in registry tool=calculator
DEBUG mini_agent_loop::tools: Tool found, preparing execution tool=calculator params={"operation":"add","a":42,"b":58}
 INFO mini_agent_loop::tools: ⏱️  Executing tool with 30s timeout tool=calculator

 INFO mini_agent_loop::tools::calculator: 🧮 Calculator tool invoked
DEBUG mini_agent_loop::tools::calculator: Calculator input params params={"operation":"add","a":42,"b":58}
DEBUG mini_agent_loop::tools::calculator: Parsed: 42 add 58 operation=add a=42 b=58
 INFO mini_agent_loop::tools::calculator: ✅ Calculator result: 42 + 58 = 100 expression="42 + 58 = 100" result=100

 INFO mini_agent_loop::tools: ✅ Tool execution succeeded in 0ms tool=calculator elapsed_ms=0
DEBUG mini_agent_loop::tools: Serialized tool output (52 chars) tool=calculator
DEBUG mini_agent_loop::tools: Wrapping tool output in <tool_output> tags (75 chars)
 INFO mini_agent_loop::tools: 📝 Creating tool result ChatMessage (role=Tool)

 INFO mini_agent_loop::agent: ✅ Tool 'calculator' succeeded in 0ms (52 chars output)
 INFO mini_agent_loop::agent: 📝 Adding tool result for 'calculator' (call_id=call_abc123) to conversation context
DEBUG mini_agent_loop::agent: Context now has 4 messages (after adding tool result)
 INFO mini_agent_loop::agent: 🔄 All tools executed. Looping back to LLM with updated context (4 messages)
```

**解读**：
- 先把 LLM 的 "我要调用工具" 这条消息加入上下文（OpenAI 协议要求）
- 在 ToolRegistry 中查找 `calculator` 工具
- 执行计算：`42 + 58 = 100`，耗时 0ms
- 把结果包装成 `<tool_output>...</tool_output>` 格式
- 创建 Tool 角色的 ChatMessage，加入上下文
- 此时上下文有 4 条消息：`[SYSTEM, USER, ASSISTANT(tool_calls), TOOL(result)]`

### 阶段 6：第二次 LLM 调用（LLM 给出最终回答）

```
 INFO mini_agent_loop::agent: ━━━ Iteration 2/10 ━━━ iteration=2 max=10 message_count=4
 INFO mini_agent_loop::agent: 📤 Step 1: Calling LLM...
DEBUG mini_agent_loop::agent: Sending 4 messages and 1 tool definitions to LLM

 INFO mini_agent_loop::llm::openai: 📤 Preparing LLM API request message_count=4 tool_count=1
DEBUG mini_agent_loop::llm::openai: Request summary: 4 messages [system → user → assistant → tool], 1 tools
 INFO mini_agent_loop::llm::openai: 🌐 Sending HTTP POST to .../chat/completions...
 INFO mini_agent_loop::llm::openai: 📥 HTTP response received: 200 OK in 890ms

DEBUG mini_agent_loop::llm::openai: Response choice: finish_reason=stop, has_content=true, has_tool_calls=false
 INFO mini_agent_loop::llm::openai: 💬 LLM returned text response (25 chars)

 INFO mini_agent_loop::agent: 📥 LLM responded in 890ms (finish_reason=Stop)
 INFO mini_agent_loop::agent: 💬 LLM returned TEXT response (25 chars)
DEBUG mini_agent_loop::agent: Full text response from LLM text="42 + 58 = 100"

 INFO mini_agent_loop: ✅ Agentic loop completed with text response elapsed_ms=2124 response_len=25
DEBUG mini_agent_loop: Full agent response response="42 + 58 = 100"

🤖 Agent: 42 + 58 = 100
```

**解读**：
- **Iteration 2** 开始，这次带着 4 条消息（包含工具结果）
- LLM 看到工具结果后，`finish_reason=stop` — 它决定直接回答
- 返回文本 "42 + 58 = 100"
- Agentic Loop 结束，总耗时 2124ms（两次 LLM 调用 + 一次工具执行）

---

## 5. 逐条日志解读

### 日志中的 emoji 含义

| Emoji | 含义 | 出现在哪里 |
|-------|------|-----------|
| 🚀 | 程序启动 | main.rs |
| 📂 | 加载文件/配置 | main.rs |
| ✅ | 操作成功 | 全局 |
| ❌ | 操作失败 | 全局 |
| ⚠️ | 警告（截断、超时等） | agent.rs |
| 📝 | 用户输入 / 添加消息到上下文 | main.rs, agent.rs |
| 📋 | 构建上下文 | main.rs |
| 🧠 | 开始 agentic loop | main.rs |
| 🔄 | 循环/切换 | main.rs, agent.rs |
| 📤 | 发送请求 | agent.rs, openai.rs |
| 📥 | 收到响应 | openai.rs |
| 🌐 | HTTP 网络请求 | openai.rs |
| 🔧 | 工具调用 | agent.rs, openai.rs |
| ⚙️ | 执行工具 | agent.rs, tools/mod.rs |
| 🔍 | 查找工具 | tools/mod.rs |
| ⏱️ | 超时控制 | tools/mod.rs |
| 🧮 | 计算器工具 | calculator.rs |
| 💬 | 文本响应 | agent.rs, openai.rs |
| 🤖 | 模型相关 | main.rs |
| 👋 | 退出 | main.rs |

### 日志中的结构化字段含义

| 字段名 | 含义 | 示例 |
|--------|------|------|
| `message_count` | 当前上下文中的消息数量 | `message_count=4` |
| `tool_count` | 可用工具数量 | `tool_count=1` |
| `elapsed_ms` | 操作耗时（毫秒） | `elapsed_ms=1234` |
| `tool_call_count` | LLM 请求的工具调用数量 | `tool_call_count=1` |
| `finish_reason` | LLM 停止原因 | `Stop`, `ToolUse`, `Length` |
| `iteration` | 当前迭代轮次 | `iteration=1` |
| `max` | 最大迭代次数 | `max=10` |
| `status` | HTTP 状态码 | `status=200` |
| `text_len` | 文本长度 | `text_len=25` |
| `response_len` | 响应长度 | `response_len=25` |
| `output_len` | 工具输出长度 | `output_len=52` |

---

## 6. 日志埋点地图

### main.rs — 程序入口与主循环

```
启动
  │
  ├─ INFO  🚀 Starting Mini Agent Loop — tracing initialized
  │
  ├─ INFO  📂 Loading LLM configuration from ./HAI_WOA.json ...
  ├─ INFO  ✅ Config loaded successfully (model, base_url)
  ├─ DEBUG Available model shortcuts
  ├─ INFO  🤖 Initial model selected
  ├─ DEBUG Creating new LLM provider instance
  │
  ├─ INFO  🔧 Tool registry initialized (tools 列表)
  ├─ DEBUG Full tool definitions (完整 JSON schema)
  ├─ INFO  ⚙️  Agentic loop config (max_iterations)
  ├─ INFO  🔄 Entering main input loop
  │
  └─ 循环 ─┐
           ├─ TRACE Empty input, skipping
           ├─ INFO  👋 User requested exit
           ├─ INFO  📝 User input received (input 内容)
           │
           ├─ DEBUG Listing available models (/models 命令)
           ├─ INFO  🔄 Switching model (/model 命令)
           │
           ├─ INFO  📋 Building conversation context
           ├─ DEBUG Initial messages (system_prompt, user_message)
           ├─ TRACE Full message context before agentic loop
           │
           ├─ INFO  🧠 Starting agentic loop (model, max_iterations)
           │
           ├─ 结果处理:
           │   ├─ INFO  ✅ Agentic loop completed (elapsed_ms, response_len)
           │   ├─ DEBUG Full agent response
           │   ├─ TRACE Final message context
           │   ├─ WARN  ⚠️  Reached max iterations
           │   └─ ERROR ❌ Agentic loop failed
           │
           └─ 继续循环 ──┘
```

### agent.rs — Agentic Loop 核心引擎

```
run_agentic_loop()
  │
  ├─ INFO  🔄 Entering agentic loop (tool_count, initial_message_count)
  │
  └─ 迭代 ─┐
           ├─ INFO  ━━━ Iteration N/M ━━━ (iteration, max, message_count)
           │
           ├─ INFO  📤 Step 1: Calling LLM...
           ├─ DEBUG Sending N messages and M tool definitions
           ├─ TRACE Message flow: [0]SYS → [1]USR → [2]AST → [3]TOL
           │
           ├─ [调用 openai.rs 的 chat() 方法]
           │
           ├─ INFO  📥 LLM responded in Xms (finish_reason)
           │
           ├─ 如果是文本响应:
           │   ├─ INFO  💬 LLM returned TEXT response (text_len)
           │   ├─ DEBUG Full text response
           │   └─ return Ok(Response)
           │
           ├─ 如果是工具调用:
           │   ├─ INFO  🔧 LLM returned TOOL CALLS (tool_call_count)
           │   ├─ DEBUG Tool call [i]: name (id, arguments)
           │   ├─ DEBUG Assistant content alongside tool calls
           │   │
           │   ├─ 如果 finish_reason == Length:
           │   │   ├─ WARN  ⚠️  Response was truncated
           │   │   ├─ INFO  Injected truncation recovery message
           │   │   └─ continue (下一轮迭代)
           │   │
           │   ├─ INFO  📝 Adding assistant message with tool calls
           │   ├─ DEBUG Context now has N messages
           │   │
           │   └─ 对每个工具调用:
           │       ├─ INFO  ⚙️  Executing tool [i/N]: name
           │       ├─ DEBUG Tool execution input (name, id, arguments)
           │       │
           │       ├─ [调用 tools/mod.rs 的 execute_tool_with_safety()]
           │       │
           │       ├─ INFO  ✅ Tool succeeded in Xms (output_len)
           │       ├─ DEBUG Full tool output
           │       ├─ ERROR ❌ Tool failed
           │       │
           │       ├─ INFO  📝 Adding tool result to conversation context
           │       └─ DEBUG Context now has N messages
           │
           ├─ INFO  🔄 All tools executed. Looping back to LLM
           └─ 继续迭代 ──┘
  │
  └─ WARN  ⚠️  Reached max iterations
```

### openai.rs — HTTP 请求与响应

```
chat()
  │
  ├─ INFO  📤 Preparing LLM API request (url, model, message_count, tool_count)
  ├─ TRACE Full OpenAI API request body (完整 JSON)
  ├─ DEBUG Request summary: N messages [system → user → ...], M tools
  │
  ├─ INFO  🌐 Sending HTTP POST to URL...
  ├─ INFO  📥 HTTP response received: STATUS in Xms
  │
  ├─ 如果 HTTP 失败:
  │   └─ ERROR API error response (status, error_body)
  │
  ├─ TRACE Raw API response body (完整 JSON, N bytes)
  ├─ DEBUG Parsed response: N choice(s)
  │
  ├─ DEBUG Response choice: finish_reason, has_content, has_tool_calls
  ├─ WARN  Unknown finish_reason (如果遇到未知值)
  │
  ├─ 如果有 tool_calls:
  │   ├─ INFO  🔧 LLM requested N tool call(s)
  │   ├─ DEBUG Parsing tool call [i]: name (id, raw_arguments)
  │   ├─ WARN  Failed to parse tool arguments (如果 JSON 解析失败)
  │   ├─ TRACE Parsed tool arguments (格式化后的 JSON)
  │   └─ DEBUG Assistant also returned text content
  │
  └─ 如果是文本:
      ├─ INFO  💬 LLM returned text response (text_len)
      └─ DEBUG Full LLM text response
```

### tools/mod.rs — 工具执行管线

```
execute_tool_with_safety()
  │
  ├─ INFO  🔍 Looking up tool in registry
  ├─ ERROR Tool not found (如果工具不存在)
  ├─ DEBUG Tool found, preparing execution (params)
  │
  ├─ INFO  ⏱️  Executing tool with 30s timeout
  │
  ├─ [调用具体工具的 execute() 方法]
  │
  ├─ 成功:
  │   ├─ INFO  ✅ Tool execution succeeded in Xms
  │   ├─ TRACE Raw tool output (完整 JSON)
  │   ├─ DEBUG Serialized tool output (N chars)
  │   └─ ERROR Failed to serialize tool result (如果序列化失败)
  │
  ├─ 失败:
  │   └─ ERROR ❌ Tool execution failed
  │
  └─ 超时:
      └─ ERROR ⏰ Tool timed out after 30s

process_tool_result()
  │
  ├─ DEBUG Wrapping tool output in <tool_output> tags
  ├─ TRACE Full wrapped tool result
  ├─ WARN  Creating error tool result message (如果工具失败)
  └─ INFO  📝 Creating tool result ChatMessage (role=Tool)
```

### calculator.rs — 计算器工具

```
execute()
  │
  ├─ INFO  🧮 Calculator tool invoked
  ├─ DEBUG Calculator input params (完整 JSON)
  ├─ DEBUG Parsed: A op B (operation, a, b)
  │
  ├─ ERROR Division by zero attempted (如果除以零)
  ├─ ERROR Unknown operation (如果操作不支持)
  │
  ├─ INFO  ✅ Calculator result: expression (expression, result)
  └─ TRACE Full calculator output (完整 JSON)
```

---

## 7. 高级用法

### 模块级过滤

`RUST_LOG` 支持精确到模块级别的过滤：

```bash
# 只看 LLM 模块的详细日志（HTTP 请求/响应）
RUST_LOG=mini_agent_loop::llm=debug cargo run

# 只看工具执行的详细日志
RUST_LOG=mini_agent_loop::tools=debug cargo run

# 只看 agentic loop 的详细日志
RUST_LOG=mini_agent_loop::agent=debug cargo run

# 组合：全局 info + LLM 模块 trace（看完整请求/响应 JSON）
RUST_LOG=info,mini_agent_loop::llm::openai=trace cargo run

# 组合：全局 info + agent 和 tools 模块 debug
RUST_LOG=info,mini_agent_loop::agent=debug,mini_agent_loop::tools=debug cargo run

# 只看 calculator 工具的日志
RUST_LOG=mini_agent_loop::tools::calculator=trace cargo run
```

### 模块路径对照表

| 模块路径 | 对应文件 | 内容 |
|---------|---------|------|
| `mini_agent_loop` | `src/main.rs` | 主循环、用户交互 |
| `mini_agent_loop::agent` | `src/agent.rs` | Agentic Loop 引擎 |
| `mini_agent_loop::llm::openai` | `src/llm/openai.rs` | HTTP 请求/响应 |
| `mini_agent_loop::tools` | `src/tools/mod.rs` | 工具注册与执行 |
| `mini_agent_loop::tools::calculator` | `src/tools/calculator.rs` | 计算器工具 |

---

## 8. 日志保存与分析

### 保存到文件

```bash
# 方法 1：tee 命令（同时显示在终端和保存到文件）
RUST_LOG=debug cargo run 2>&1 | tee debug_run.log

# 方法 2：只保存到文件（终端不显示日志，只显示 println! 输出）
RUST_LOG=debug cargo run 2>debug_run.log

# 方法 3：trace 级别保存到文件，终端只看 info
RUST_LOG=trace cargo run 2>trace_run.log
```

### 分析日志

```bash
# 搜索所有 LLM 调用耗时
grep "elapsed_ms" debug_run.log

# 搜索所有工具执行
grep "Executing tool" debug_run.log

# 搜索所有错误
grep "ERROR" debug_run.log

# 搜索所有警告
grep "WARN" debug_run.log

# 统计每次迭代
grep "Iteration" debug_run.log

# 查看消息数量变化
grep "message_count" debug_run.log
```

---

## 9. 常见问题

### Q: 日志太多了，怎么只看关键信息？

用 `RUST_LOG=info`（默认），只会看到带 emoji 的关键事件。

### Q: 我想看完整的 HTTP 请求和响应 JSON？

```bash
RUST_LOG=mini_agent_loop::llm::openai=trace cargo run
```

### Q: 日志中的 `%` 和 `?` 是什么意思？

在 tracing 的代码中：
- `%variable` — 用 `Display` trait 格式化（人类可读）
- `?variable` — 用 `Debug` trait 格式化（程序员可读，带类型信息）

### Q: 为什么有些日志在 stderr 而不是 stdout？

`tracing` 默认输出到 **stderr**。这是有意为之的设计：
- `println!` → stdout（程序的正常输出）
- `tracing` → stderr（诊断信息）

这样你可以用 `2>/dev/null` 隐藏日志，或用 `2>log.txt` 单独保存日志。

### Q: 生产环境怎么用？

```bash
# 生产环境：只记录 warn 和 error
RUST_LOG=warn cargo run

# 或者完全关闭日志
RUST_LOG=off cargo run
```

### Q: 消息上下文中的 4 条消息分别是什么？

当用户问数学问题时，一次完整的工具调用会产生 4 条消息：

```
[0] SYSTEM  — "You are a helpful assistant..."        ← 系统提示词
[1] USER    — "What is 42 + 58?"                      ← 用户输入
[2] ASSISTANT (tool_calls) — 调用 calculator(add,42,58) ← LLM 决定调用工具
[3] TOOL    — "<tool_output>100</tool_output>"         ← 工具执行结果
```

然后这 4 条消息一起发给 LLM，LLM 看到工具结果后给出最终文本回答。

### Q: 什么是 finish_reason？

LLM 返回的 `finish_reason` 告诉我们它为什么停止生成：

| 值 | 含义 | 后续动作 |
|----|------|---------|
| `stop` | 正常完成，给出了最终回答 | 返回文本给用户 |
| `tool_calls` | 想调用工具 | 执行工具，把结果喂回 LLM |
| `length` | 达到 token 上限，回答被截断 | 丢弃工具调用，要求 LLM 简化 |

---

## 数据流全景图

```
┌─────────────────────────────────────────────────────────────────────┐
│                        main.rs — 主循环                             │
│                                                                     │
│  ┌──────────┐    ┌──────────────────────────────────────────────┐   │
│  │ 用户输入  │───▶│  构建消息上下文 [SYSTEM, USER]                │   │
│  └──────────┘    └──────────────┬───────────────────────────────┘   │
│                                 │                                   │
│                                 ▼                                   │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │              agent.rs — Agentic Loop                         │   │
│  │                                                              │   │
│  │  ┌─────────────────────────────────────────────────────┐     │   │
│  │  │  Iteration 1:                                       │     │   │
│  │  │                                                     │     │   │
│  │  │  📤 调用 LLM ──────────────────────────────────┐    │     │   │
│  │  │                                                │    │     │   │
│  │  │  ┌─────────────────────────────────────────┐   │    │     │   │
│  │  │  │  openai.rs — HTTP 请求                  │   │    │     │   │
│  │  │  │                                         │   │    │     │   │
│  │  │  │  构建 OpenAI 格式请求体                  │   │    │     │   │
│  │  │  │  POST /chat/completions                 │   │    │     │   │
│  │  │  │  解析响应 JSON                          │   │    │     │   │
│  │  │  │  返回 LlmResponse                       │   │    │     │   │
│  │  │  └─────────────────────────────────────────┘   │    │     │   │
│  │  │                                                │    │     │   │
│  │  │  📥 LLM 返回: ToolCalls [calculator(add,42,58)]│    │     │   │
│  │  │                                                │    │     │   │
│  │  │  ⚙️  执行工具 ────────────────────────────┐    │    │     │   │
│  │  │                                           │    │    │     │   │
│  │  │  ┌────────────────────────────────────┐   │    │    │     │   │
│  │  │  │  tools/mod.rs — 执行管线           │   │    │    │     │   │
│  │  │  │                                    │   │    │    │     │   │
│  │  │  │  🔍 查找工具                       │   │    │    │     │   │
│  │  │  │  ⏱️  设置超时 (30s)                │   │    │    │     │   │
│  │  │  │  ┌──────────────────────────┐      │   │    │    │     │   │
│  │  │  │  │ calculator.rs            │      │   │    │    │     │   │
│  │  │  │  │ 🧮 42 + 58 = 100        │      │   │    │    │     │   │
│  │  │  │  └──────────────────────────┘      │   │    │    │     │   │
│  │  │  │  序列化结果                        │   │    │    │     │   │
│  │  │  │  包装为 <tool_output>              │   │    │    │     │   │
│  │  │  └────────────────────────────────────┘   │    │    │     │   │
│  │  │                                           │    │    │     │   │
│  │  │  📝 添加工具结果到上下文                    │    │    │     │   │
│  │  │  上下文: [SYSTEM, USER, ASSISTANT, TOOL]   │    │    │     │   │
│  │  └─────────────────────────────────────────────┘    │   │     │   │
│  │                                                     │   │     │   │
│  │  ┌─────────────────────────────────────────────────────┐│     │   │
│  │  │  Iteration 2:                                       ││     │   │
│  │  │                                                     ││     │   │
│  │  │  📤 调用 LLM（带 4 条消息）                          ││     │   │
│  │  │  📥 LLM 返回: Text "42 + 58 = 100"                 ││     │   │
│  │  │  💬 返回最终文本                                     ││     │   │
│  │  └─────────────────────────────────────────────────────┘│     │   │
│  └──────────────────────────────────────────────────────────┘     │   │
│                                 │                                   │
│                                 ▼                                   │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  ✅ 输出: "🤖 Agent: 42 + 58 = 100"                         │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

---

> **提示**：建议第一次运行时用 `RUST_LOG=debug cargo run 2>&1 | tee first_run.log`，然后对照本文档逐条阅读日志，你会对整个 Agent 的运行过程有非常清晰的理解。
