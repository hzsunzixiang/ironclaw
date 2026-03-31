# Session / Thread / Turn — 功能说明与命令手册

## 概念模型

```
Session (per user)
└── Thread (per conversation — 可以有多个)
    └── Turn (per request/response pair)
        ├── user_input: String          — 用户输入
        ├── response: Option<String>    — Agent 回复
        ├── tool_calls: Vec<TurnToolCall> — 本轮工具调用记录
        └── state: TurnState            — Processing / Completed / Failed
```

### Session

- 每个用户对应一个 Session
- 包含一个或多个 Thread
- 记录 session ID、用户 ID、创建时间、最后活跃时间

### Thread

- 一个独立的对话上下文（类似"聊天窗口"）
- 包含有序的 Turn 列表
- **核心能力**：`Thread::messages()` 方法将所有 Turn 历史重建为 LLM 需要的 `ChatMessage` 列表，实现多轮记忆
- 状态：`Idle`（等待输入）/ `Processing`（正在处理）

### Turn

- 一次完整的"用户提问 → Agent 回答"交互
- 记录用户输入、Agent 回复、所有工具调用（含参数和结果）、状态、时间戳
- 状态：`Processing` → `Completed` 或 `Failed`

---

## Session 能做什么

| 能力 | 说明 |
|------|------|
| **多轮对话记忆** | LLM 能看到当前 Thread 的完整对话历史，可以引用之前的结果 |
| **多 Thread 管理** | 在同一个 Session 中创建多个独立对话，互不干扰 |
| **Thread 切换** | 随时在不同 Thread 之间切换，每个 Thread 保留各自的上下文 |
| **Turn 历史回溯** | 查看当前 Thread 的所有 Turn 记录，包括工具调用详情 |
| **工具调用追踪** | 每个 Turn 记录了所有工具调用的名称、参数、结果/错误 |
| **Session 概览** | 查看 Session 级别的统计信息（线程数、总 Turn 数等） |

---

## 命令列表

### Session 管理命令

| 命令 | 操作层级 | 说明 | 示例 |
|------|---------|------|------|
| `/new` | Thread | 创建一个新的 Thread（新对话） | `/new` |
| `/threads` | Session | 列出当前 Session 中所有 Thread | `/threads` |
| `/switch <n>` | Thread | 切换到第 n 个 Thread（编号从 1 开始） | `/switch 2` |
| `/history` | Thread | 显示当前 Thread 的 Turn 历史 | `/history` |
| `/session` | Session | 显示 Session 概览信息 | `/session` |

### 模型管理命令

| 命令 | 操作层级 | 说明 | 示例 |
|------|---------|------|------|
| `/models` | Session | 列出所有可用模型及快捷名 | `/models` |
| `/model <name>` | Session | 切换到指定模型（支持快捷名） | `/model qwen` |

### 其他命令

| 命令 | 操作层级 | 说明 |
|------|---------|------|
| `quit` | Session | 退出程序，显示 Session 统计 |
| `exit` | Session | 同 `quit` |

---

## 命令详细说明

### `/new` — 创建新 Thread

创建一个全新的对话 Thread 并自动切换到该 Thread。新 Thread 没有任何历史，LLM 不会看到之前 Thread 的对话内容。

```
🧑 [T2:a1b2] You: /new
✅ New thread created: c3d4e5f6
   Now in thread c3d4e5f6 (0 turns)
🧑 [T1:c3d4] You: _
```

### `/threads` — 列出所有 Thread

显示当前 Session 中所有 Thread 的摘要，包括编号、ID、Turn 数量、首条消息预览，以及哪个是当前活跃 Thread。

```
🧑 You: /threads

📋 Threads (2):
   #1 [a1b2c3d4] 3 turn(s) — "What is 42 + 58?"
   #2 [e5f6g7h8] 1 turn(s) — "What is 1 + 1?" ← active
```

### `/switch <n>` — 切换 Thread

切换到指定编号的 Thread。编号对应 `/threads` 列出的顺序（从 1 开始）。切换后，后续对话将在目标 Thread 的上下文中进行，LLM 能看到该 Thread 的完整历史。

```
🧑 You: /switch 1
✅ Switched to thread #1 [a1b2c3d4] (3 turns)
```

### `/history` — 查看 Turn 历史

显示当前活跃 Thread 的所有 Turn 记录，每个 Turn 包含：
- Turn 编号和状态图标（✅ 完成 / ⏳ 处理中 / ❌ 失败）
- 用户输入
- 工具调用记录（工具名、参数、结果）
- Agent 回复

```
🧑 You: /history

📜 Turn history for thread a1b2c3d4 (2 turns):

   Turn #1 ✅ [14:30:05]
     🧑 "What is 42 + 58?"
     🔧 calculator({"expression":"42 + 58"}) → 100
     🤖 "The result of 42 + 58 is **100**."

   Turn #2 ✅ [14:30:18]
     🧑 "Now multiply that result by 3"
     🔧 calculator({"expression":"100 * 3"}) → 300
     🤖 "Multiplying the previous result (100) by 3 gives **300**."
```

### `/session` — Session 概览

显示 Session 级别的信息：

```
🧑 You: /session

📊 Session info:
   ID: a1b2c3d4-e5f6-7890-abcd-ef1234567890
   User: cli-user
   Created: 2026-03-31 10:30:00
   Threads: 2
   Total turns: 4
   Active thread: a1b2c3d4 (3 turns, Idle)
   LLM: openai/DeepSeek-V3-0324
```

### `/models` — 列出可用模型

```
🧑 You: /models

📋 Available models:
   (default) → openai/DeepSeek-V3-0324
   ds → openai/DeepSeek-V3-0324 ← current
   ds31 → openai/DeepSeek-V3.1
   qwen → openai/Qwen3-235B-A22B
   qwen32 → openai/Qwen3-32B-FP8
   kimi → openai/Kimi-K2
   Current: openai/DeepSeek-V3-0324
```

### `/model <name>` — 切换模型

支持使用快捷名或完整模型 ID。切换后系统提示词会更新为新模型名称。

```
🧑 You: /model qwen
✅ Switched to model: openai/Qwen3-235B-A22B
```

---

## 测试 Session 效果的操作流程

### 测试 1：多轮对话记忆

```
🧑 You: What is 42 + 58?
🧑 You: Now multiply that result by 3
🧑 You: What was my first question?
```

验证：第 2 轮 LLM 应记住第 1 轮结果（100）并计算 300；第 3 轮应能回忆你问过什么。

### 测试 2：Turn 历史

```
🧑 You: /history
```

验证：应显示上面 3 个 Turn 的完整记录。

### 测试 3：多 Thread 隔离

```
🧑 You: /new
🧑 You: What is 1 + 1?
🧑 You: /threads
🧑 You: /switch 1
🧑 You: What was my last answer?
```

验证：切回 Thread 1 后，LLM 应记得 Thread 1 的对话（300），而不是 Thread 2 的（2）。

### 测试 4：Session 统计

```
🧑 You: /session
```

验证：应显示 2 个 Thread、总共 4+ 个 Turn。

---

## 运行方式

### 交互模式（推荐）

```bash
make run
```

### 带 Trace 日志的交互模式

```bash
make build
RUST_LOG="mini_agent_wasm_session=trace,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn" \
  cargo run --manifest-path host/Cargo.toml -- \
  guest/target/wasm32-wasip2/release/guest_tool.wasm 2>trace-output-raw.log
```

stdout 显示交互界面，trace 日志写入 `trace-output-raw.log`。

### 自动化多轮测试（推荐）

使用专用的多轮测试脚本，自动喂入多行输入并分别收集 stdout 和 trace 日志：

```bash
make run-session-test              # 默认场景 (basic: 3轮对话 + history)
make run-session-test-full         # 完整场景 (多轮 + 多Thread + 所有命令)
```

或直接运行脚本：

```bash
./run-session-test.sh                        # 默认 basic 场景
./run-session-test.sh --scenario threads     # 多Thread隔离测试
./run-session-test.sh --scenario full        # 完整测试
./run-session-test.sh --scenario stress      # 多轮链式计算压力测试
./run-session-test.sh --list                 # 列出所有内置场景
./run-session-test.sh my-inputs.txt          # 自定义输入文件
```

内置场景：

| 场景 | 说明 | 输入行数 |
|------|------|----------|
| `basic` | 多轮记忆测试（3个问题 + /history） | 6 |
| `threads` | 多Thread隔离测试（2个Thread，切换验证） | 10 |
| `full` | 完整测试（记忆 + 多Thread + 所有命令） | 16 |
| `stress` | 链式计算压力测试（6轮连续计算） | 9 |

输出文件：

| 文件 | 内容 |
|------|------|
| `session-test.log` | 合并报告（报告头 + stdout/trace 交织 + 分析摘要） |

自定义输入文件格式（每行一个输入，支持 `/` 命令）：

```text
What is 42 + 58?
Now multiply that result by 3
/history
/new
What is 1 + 1?
/threads
/session
quit
```

### 单轮 Trace（run-trace.sh）

```bash
./run-trace.sh                        # 默认问题
./run-trace.sh "What is 1+1?"         # 自定义问题
./run-trace.sh "question" my-log.log  # 自定义问题 + 自定义日志文件
```

> 注意：`run-trace.sh` 只发送单个问题后自动 quit，不适合测试多轮 Session 效果。
