# Mini Agent Multi-Agent — Trace 日志完整分析

> 源文件：`trace-output-raw.log`（6236 行，488 KB）
> 运行时间：2026-04-01 12:00:45 — 12:01:00（约 15 秒）
> 模型：DeepSeek-V3-0324 via http://api.haihub.cn/v1

---

## 目录

1. [全局概览](#1-全局概览)
2. [系统架构图](#2-系统架构图)
3. [Session 1 — ChatDelegate: Store Memories](#3-session-1--chatdelegate-store-memories)
4. [Session 2 — Multi-Agent: Jobs + Chat Interleave](#4-session-2--multi-agent-jobs--chat-interleave)
5. [Session 3 — Parallel Jobs + Memory Recall](#5-session-3--parallel-jobs--memory-recall)
6. [关键数据统计](#6-关键数据统计)
7. [多 Agent 架构验证](#7-多-agent-架构验证)
8. [日志级别说明](#8-日志级别说明)

---

## 1. 全局概览

### 三个 Session 的定位

| Session | 时间范围 | 核心测试目标 | 用户输入数 | LLM 调用数 |
|---------|---------|-------------|-----------|-----------|
| **Session 1** | 12:00:45 — 12:00:51 | ChatDelegate 前台对话 + 记忆存储 | 6（含 quit） | 8 |
| **Session 2** | 12:00:54 — 12:00:56 | 多 Agent 并行：Job + Chat 交替 | 7（含 quit） | 6+ |
| **Session 3** | 12:00:59 — 12:01:00 | 3 个并行 Job + 跨 Session 记忆召回 | 8（含 quit） | 8+ |

### 每个 Session 的启动流程（完全相同）

```
1. 🚀 tracing 初始化
2. 🔧 加载 WASM 工具（guest_tool.wasm, 141583 bytes）
   ├── 编译 WASM component（~25ms）
   └── 读取元数据：name=calculator, 4 个参数
3. 📂 加载 LLM 配置（HAI_WOA.json）
   ├── 模型：DeepSeek-V3-0324
   ├── Base URL：http://api.haihub.cn/v1
   └── 5 个快捷方式：ds, ds31, kimi, qwen, qwen32
4. 📂 初始化 Memory Store（workspace/）
   └── 读取 MEMORY.md（Session 1: 38 words → Session 2/3: 91 words）
5. 🔧 注册 4 个工具
   ├── calculator [WASM] — 数学计算
   ├── memory_search [native] — 记忆搜索
   ├── memory_write [native] — 记忆写入
   └── memory_read [native] — 记忆读取
6. 🆕 创建 Session + Thread
7. 📋 初始化 Scheduler（max 3 parallel jobs）  ← Session 2/3 才有
8. 🔄 进入主输入循环
```

---

## 2. 系统架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                        Main Input Loop                          │
│                    (stdin → Router → dispatch)                   │
└──────────────────────────┬──────────────────────────────────────┘
                           │
                    ┌──────▼──────┐
                    │   Router    │  解析用户输入，分发到不同处理器
                    └──────┬──────┘
                           │
              ┌────────────┼────────────────┐
              │            │                │
     ┌────────▼───┐  ┌────▼─────┐  ┌───────▼──────┐
     │  UserInput  │  │ CreateJob │  │ ListJobs /   │
     │ (自然语言)  │  │ (/job)   │  │ Status /     │
     │             │  │          │  │ Cancel /     │
     │             │  │          │  │ Memory       │
     └──────┬──────┘  └────┬─────┘  └──────────────┘
            │              │
   ┌────────▼────────┐  ┌──▼──────────────┐
   │  ChatDelegate   │  │   Scheduler     │
   │  (前台, main)   │  │  (max 3 jobs)   │
   │                 │  │                 │
   │  ┌───────────┐  │  │  ┌───────────┐  │
   │  │ Agentic   │  │  │  │ JobDelegate│  │
   │  │ Loop      │  │  │  │ (后台,     │  │
   │  │ (shared)  │  │  │  │ tokio::    │  │
   │  │           │  │  │  │ spawn)     │  │
   │  └─────┬─────┘  │  │  └─────┬─────┘  │
   └────────┼────────┘  └────────┼────────┘
            │                    │
            └────────┬───────────┘
                     │
              ┌──────▼──────┐
              │ LoopDelegate │  ← 共享 trait：LLM 调用 + 工具执行
              │  (trait)     │
              └──────┬──────┘
                     │
         ┌───────────┼───────────┐
         │           │           │
    ┌────▼────┐ ┌────▼────┐ ┌───▼────────┐
    │  LLM    │ │  Tool   │ │  Memory    │
    │ OpenAI  │ │Registry │ │  Store     │
    │ API     │ │         │ │            │
    └─────────┘ └────┬────┘ └────────────┘
                     │
              ┌──────┼──────┐
              │      │      │
         ┌────▼┐ ┌──▼───┐ ┌▼────────┐
         │WASM │ │memory│ │memory   │
         │calc │ │search│ │write/   │
         │     │ │      │ │read     │
         └─────┘ └──────┘ └─────────┘
```

---

## 3. Session 1 — ChatDelegate: Store Memories

> **目标**：验证 ChatDelegate 前台对话、记忆存储、WASM 工具调用
> **时间**：12:00:45 — 12:00:51（约 6 秒）
> **Session ID**：`b0608e71`（Thread）

### 3.1 Turn 逐轮分析

#### Turn #0 — 自我介绍（记忆自动提取）

```
用户: "My name is Erick and I work at IronClaw Labs. I live in Shanghai."
```

**处理流程**：
```
1. Router → UserInput（无命令前缀，路由到自然语言处理）
2. 读取 MEMORY.md（38 words，空白初始状态）
3. ChatDelegate → 创建 Turn #0
4. 构建上下文：2 messages（System + User），~301 tokens
5. Compaction check：301 tokens vs 6400 threshold → OK（不需要压缩）
6. 🔄 Agentic Loop 启动（max 10 iterations）
   └── Iteration 1: 调用 LLM
       ├── 发送 2 messages + 4 tools
       ├── HTTP POST → 200 OK（922ms）
       └── LLM 返回 memory_write tool call
   └── Iteration 2: 执行 memory_write
       ├── 写入 MEMORY.md："User's name is Erick..."
       └── 再次调用 LLM → 返回文本回复
7. ✅ Turn 完成（2018ms）
8. 🧠 自动记忆提取：检测到个人信息，追加到 MEMORY.md
9. 📝 写入 daily log
```

**LLM 回复**：
> "Got it, Erick! I've noted that you work at IronClaw Labs and live in Shanghai."

**记忆变化**：MEMORY.md 从 38 words → 新增 "User's name is Erick. He works at IronClaw Labs and lives in Shanghai."

---

#### Turn #1 — 项目信息存储

```
用户: "Remember that our project deadline is April 15th 2026 and the project name is Phoenix."
```

**处理流程**：
```
1. Router → UserInput
2. ChatDelegate → Turn #1
3. 上下文：4 messages（含 Turn #0 历史），~186 tokens
4. Agentic Loop:
   └── Iteration 1: LLM 返回 memory_write（target=memory）
       ├── 写入："Project deadline for Phoenix is April 15, 2026."
       └── 工具执行 0ms
   └── Iteration 2: LLM 返回文本回复
5. 🧠 自动记忆提取：检测到 "remember" 关键词
   ├── 相似度检查：与已有条目对比（0.167 匹配度 → 不重复）
   └── 追加到 MEMORY.md
```

**LLM 回复**：
> "I've noted that the project deadline for Phoenix is April 15, 2026."

---

#### Turn #2 — 技术偏好

```
用户: "I prefer using Rust for backend development and TypeScript for frontend."
```

**处理流程**：
```
1. Router → UserInput
2. ChatDelegate → Turn #2
3. 上下文：6 messages，~222 tokens
4. Agentic Loop:
   └── Iteration 1: LLM 返回 memory_write
       ├── 写入："Erick prefers using Rust for backend..."
       └── 工具执行 0ms
   └── Iteration 2: LLM 返回文本回复
5. 🧠 自动记忆提取：检测到偏好信息
```

**LLM 回复**：
> "I've updated your preferences: Backend: Rust, Frontend: TypeScript"

---

#### Turn #3 — 复杂数学计算（WASM 工具链）⭐

```
用户: "What is 42 + 58 * 99 + (88 * 12 / 3)?"
```

这是最能展示 **Agentic Loop 多轮工具调用** 的 Turn。LLM 需要 3 次迭代才能完成：

**处理流程**：
```
Iteration 1: LLM 分解问题，并行调用 3 个 calculator
  ├── call_1: 58 × 99 = 5742     ← WASM sandbox, fuel=16217/1000000
  ├── call_2: 88 × 12 = 1056     ← WASM sandbox
  └── call_3: 1056 ÷ 3 = 352     ← WASM sandbox
  （3 个工具结果返回给 LLM）

Iteration 2: LLM 继续组合结果，调用 2 个 calculator
  ├── call_4: 42 + 5742 = 5784   ← WASM sandbox
  └── call_5: 5784 + 352 = 6136  ← WASM sandbox, fuel=16594/1000000
  （2 个工具结果返回给 LLM）

Iteration 3: LLM 输出最终答案
  └── "The result is **6136**."
```

**关键观察**：
- LLM 正确分解了运算优先级（先乘除后加减）
- 每次 WASM 执行消耗约 16000 fuel（上限 1000000），非常轻量
- 每次 WASM 执行耗时 0-1ms（亚毫秒级）
- 总耗时 2018ms（主要是 3 次 LLM API 调用：922ms + 639ms + 455ms）
- 记忆提取跳过：检测到是数学问题，不提取记忆

**消息流演变**：
```
Iter 1: [SYS → USR → AST → USR → AST → USR → AST → USR]  (8 messages)
Iter 2: [... → AST(tool_calls) → TOL → TOL → TOL]          (12 messages)
Iter 3: [... → AST(tool_calls) → TOL → TOL]                 (15 messages)
```

---

#### Turn #4 — 基础设施信息 + 自动记忆

```
用户: "Please remember that the database we use is PostgreSQL and our deployment target is CentOS."
```

**处理流程**：
```
1. 上下文：10 messages，~522 tokens（仍远低于 6400 阈值）
2. Agentic Loop:
   └── Iteration 1: LLM 直接返回文本（未调用 memory_write 工具！）
       └── 因为 LLM 判断信息不在 Long-Term Memory 中，但选择了直接回复
3. 🧠 自动记忆提取（关键！）：
   ├── Heuristic 1 命中：检测到 "remember" 关键词
   ├── 提取事实："the database we use is PostgreSQL and our deployment target is CentOS"
   ├── 相似度检查：
   │   ├── vs "User's name is Erick..." → 0.167（不重复）
   │   ├── vs "Project deadline..." → 0.000（不重复）
   │   └── vs "Erick prefers Rust..." → 0.000（不重复）
   └── ✅ 追加到 MEMORY.md
4. 📝 写入 daily log
```

**LLM 回复**：
> "Got it! Database: PostgreSQL, Deployment target: CentOS"

**关键观察**：即使 LLM 没有主动调用 `memory_write`，系统的**自动记忆提取**机制仍然捕获了信息并持久化。这是双保险设计。

---

#### Turn #5 — 退出

```
用户: "quit"
```

```
Session 统计：1 thread, 5 total turns
📝 写入 daily log：session end
```

### 3.2 Session 1 最终 MEMORY.md

```markdown
# Memory
- User's name is Erick. He works at IronClaw Labs and lives in Shanghai.
- Project deadline for Phoenix is April 15, 2026.
- Erick prefers using Rust for backend development and TypeScript for frontend.
- User asked to remember: the database we use is PostgreSQL and our deployment target is CentOS.
```

**从 38 words → 91 words**，4 条记忆条目。

---

## 4. Session 2 — Multi-Agent: Jobs + Chat Interleave

> **目标**：验证 JobDelegate 后台任务 + ChatDelegate 前台对话交替执行
> **时间**：12:00:54 — 12:00:56（约 2 秒）
> **Session ID**：`57684a6b`，Thread：`d5e233e8`

### 4.1 关键特性：记忆持久化验证

Session 2 启动时，MEMORY.md 已包含 Session 1 存储的 91 words。这证明了**跨 Session 记忆持久化**。

### 4.2 命令序列与路由

```
输入 1: /job Calculate the result of 999 * 888 + 777...
         → Router: CreateJob → Scheduler.dispatch_job(b0f74200)

输入 2: /jobs
         → Router: ListJobs → [b0f74200] ⏳ Pending

输入 3: What is my name and where do I work?
         → Router: UserInput → ChatDelegate

输入 4: /job Search memory for information about our project deadline...
         → Router: CreateJob → Scheduler.dispatch_job(56b4b0ee)

输入 5: /jobs
         → Router: ListJobs → [56b4b0ee] 🔄 In Progress, [b0f74200] 🔄 In Progress

输入 6: What programming languages do I prefer?
         → Router: UserInput → ChatDelegate

输入 7: /jobs
         → Router: ListJobs → [56b4b0ee] 🔄 In Progress, [b0f74200] 🔄 In Progress

输入 8: quit
```

### 4.3 并行执行时间线 ⭐

这是最能体现多 Agent 架构的部分。以下是精确到毫秒的时间线：

```
时间轴 (12:00:54.xxx — 12:00:56.xxx)
│
├─ 54.501 ─ /job 999*888+777 → Scheduler dispatch job b0f74200
│            ├── Job 状态：Pending
│
├─ 54.501 ─ /jobs → 显示 [b0f74200] ⏳ Pending
│
├─ 54.501 ─ "What is my name?" → ChatDelegate 开始处理
│            │
│            ├─ 54.502 ─ [后台] Job b0f74200 worker 启动 (tokio-rt-worker)
│            │            └── JobDelegate Agentic Loop 开始
│            │            └── 发送 LLM 请求（后台线程）
│            │
│            ├─ 54.502 ─ [前台] ChatDelegate Agentic Loop 开始
│            │            └── 发送 LLM 请求（main 线程）
│            │
│            │  ┌──────────────────────────────────────────┐
│            │  │  两个 LLM 请求同时在飞！                    │
│            │  │  • main 线程：ChatDelegate (name query)    │
│            │  │  • tokio-rt-worker：JobDelegate (999*888)  │
│            │  └──────────────────────────────────────────┘
│            │
│            ├─ 55.751 ─ [前台] ChatDelegate 收到 LLM 回复（1249ms）
│            │            └── "Your name is Erick, you work at IronClaw Labs"
│            │
│            ├─ 55.752 ─ [后台] Job b0f74200 收到 LLM 回复（1249ms）
│            │            └── LLM 请求 calculator: 999 × 888
│            │            └── WASM 执行：999 × 888 = 887112（1ms）
│            │
├─ 55.753 ─ /job Search memory... → Scheduler dispatch job 56b4b0ee
│            ├── Job 56b4b0ee worker 启动
│            └── JobDelegate Agentic Loop 开始
│
├─ 55.753 ─ /jobs → [56b4b0ee] 🔄 In Progress, [b0f74200] 🔄 In Progress
│
├─ 55.753 ─ "What programming languages?" → ChatDelegate 开始
│            │
│            │  ┌──────────────────────────────────────────┐
│            │  │  三个 Agent 同时运行！                      │
│            │  │  • main：ChatDelegate (lang query)        │
│            │  │  • worker-1：Job b0f74200 (887112+777)    │
│            │  │  • worker-2：Job 56b4b0ee (memory_search) │
│            │  └──────────────────────────────────────────┘
│            │
│            ├─ 56.331 ─ [后台] Job 56b4b0ee: memory_search 返回
│            │            └── 找到 daily log 中的 project deadline 记录
│            │
│            ├─ 56.455 ─ [前台] ChatDelegate 回复（698ms）
│            │            └── "You prefer Rust for backend, TypeScript for frontend"
│
├─ 56.455 ─ /jobs → 两个 Job 仍在运行
│
├─ 56.455 ─ quit → 退出（后台 Job 仍在运行中被终止）
```

### 4.4 ChatDelegate vs JobDelegate 系统提示词对比

| 特性 | ChatDelegate | JobDelegate |
|------|-------------|-------------|
| **角色** | "helpful assistant" | "background worker agent" |
| **上下文** | 包含完整对话历史 | 仅包含任务描述 |
| **max_iterations** | 10 | 15（更多迭代空间） |
| **线程** | main 线程（阻塞式） | tokio-rt-worker（异步） |
| **记忆** | 加载 Long-Term Memory | 加载 Long-Term Memory |
| **工具** | 共享 4 个工具 | 共享 4 个工具 |
| **指令** | 通用助手 | "Focus on completing the task efficiently. Do NOT ask for clarification." |

### 4.5 关键观察

1. **真正的并行**：日志中可以看到 `main` 和 `tokio-rt-worker` 线程的日志交错出现，证明 ChatDelegate 和 JobDelegate 确实在并行执行。

2. **LLM 请求并行**：两个 Agent 的 HTTP POST 请求几乎同时发出（54.502），几乎同时收到响应（55.751 和 55.752），说明 LLM API 调用是真正并行的。

3. **跨 Session 记忆**：ChatDelegate 在 Session 2 中成功回答了 Session 1 存储的信息（姓名、工作地点、编程偏好），证明 MEMORY.md 持久化有效。

---

## 5. Session 3 — Parallel Jobs + Memory Recall

> **目标**：验证 3 个并行 Job（达到 Scheduler 上限）+ 跨 Session 记忆召回
> **时间**：12:00:59 — 12:01:00（约 1 秒）
> **Session ID**：`fd7c2ef0`，Thread：`8492da18`

### 5.1 命令序列

```
输入 1: /job Calculate 123 * 456 + 789...
         → Scheduler dispatch job c0b49d91

输入 2: /job Calculate 100 + 200 + 300 + 400 + 500...
         → Scheduler dispatch job c5d038c7

输入 3: /job Search memory for all stored information...
         → Scheduler dispatch job 509fb54b

输入 4: /jobs
         → 3 total, 3 running（达到 max_parallel_jobs=3 上限）

输入 5: What project am I working on and when is the deadline?
         → ChatDelegate（前台）

输入 6: /jobs → 3 个 Job 仍在运行

输入 7: /memory → 显示完整 MEMORY.md

输入 8: quit
```

### 5.2 三个并行 Job 的执行详情

```
┌─────────────────────────────────────────────────────────────────┐
│                    Session 3 并行执行时间线                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  59.396 ─ dispatch job c0b49d91 (123*456+789)                  │
│  59.396 ─ dispatch job c5d038c7 (100+200+300+400+500)          │
│  59.397 ─ dispatch job 509fb54b (memory search all)            │
│                                                                 │
│  59.396 ─ Job c0b49d91 worker started ──┐                      │
│  59.396 ─ Job c5d038c7 worker started ──┤ 三个 worker 几乎      │
│  59.397 ─ Job 509fb54b worker started ──┘ 同时启动               │
│                                                                 │
│  59.397 ─ /jobs → 3 total, 3 running ← 全部 In Progress        │
│                                                                 │
│  59.397 ─ ChatDelegate 开始处理 "What project..."               │
│           │                                                     │
│           │  ┌────────────────────────────────────────────┐     │
│           │  │  4 个 Agent 同时运行！                       │     │
│           │  │  • main：ChatDelegate                       │     │
│           │  │  • worker-1：Job c0b49d91 (calculator)      │     │
│           │  │  • worker-2：Job c5d038c7 (calculator)      │     │
│           │  │  • worker-3：Job 509fb54b (memory_search)   │     │
│           │  └────────────────────────────────────────────┘     │
│                                                                 │
│  00.194 ─ Job 509fb54b: memory_search 返回 2 results           │
│  00.195 ─ ChatDelegate: memory_search 返回 1 result            │
│  00.245 ─ Job c5d038c7: calculator 100+200=300                 │
│  00.651 ─ Job c0b49d91: calculator 123×456=56088, 789+56088    │
│                                                                 │
│  00.752 ─ ChatDelegate 回复：                                   │
│           "Phoenix project, deadline April 15, 2026"            │
│                                                                 │
│  00.753 ─ /jobs → 3 个 Job 仍在运行                             │
│  00.753 ─ /memory → 显示完整 MEMORY.md（4 条记忆）               │
│  00.753 ─ quit                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 5.3 各 Job 的 Agentic Loop 详情

#### Job c0b49d91 — Calculate 123 * 456 + 789

```
Iteration 1 (1255ms):
  LLM → 2 个并行 tool_calls:
    ├── calculator: 123 × 456 = 56088  (fuel: 16634)
    └── calculator: 789 + 56088 = 56877 (fuel: 16868)

Iteration 2:
  LLM → 最终回复（任务完成）
```

#### Job c5d038c7 — Calculate 100 + 200 + 300 + 400 + 500

```
Iteration 1 (849ms):
  LLM → calculator: 100 + 200 = 300

Iteration 2:
  LLM → calculator: 300 + 300 = 600
  （逐步累加...）
```

#### Job 509fb54b — Search memory for all stored information

```
Iteration 1 (798ms):
  LLM → memory_search: query="all stored information", limit=20
  结果：2 条匹配
    ├── daily/2026-04-01.md (score: 1.0)
    └── MEMORY.md (score: 1.0)

Iteration 2:
  LLM → memory_read: 读取 MEMORY.md 完整内容
  → 汇总所有存储信息
```

### 5.4 /memory 命令输出

Session 3 中 `/memory` 命令展示了跨 3 个 Session 持久化的完整记忆：

```
🧠 Long-Term Memory (MEMORY.md):
─────────────────────────────────
# Memory
- User's name is Erick. He works at IronClaw Labs and lives in Shanghai.
- Project deadline for Phoenix is April 15, 2026.
- Erick prefers using Rust for backend development and TypeScript for frontend.
- User asked to remember: the database we use is PostgreSQL and our deployment target is CentOS.
─────────────────────────────────
```

---

## 6. 关键数据统计

### 6.1 LLM API 调用统计

| 指标 | Session 1 | Session 2 | Session 3 | 总计 |
|------|-----------|-----------|-----------|------|
| HTTP POST 请求数 | 8 | 6+ | 8+ | ~22 |
| 平均响应时间 | ~700ms | ~900ms | ~800ms | ~800ms |
| 最快响应 | 455ms | 577ms | 551ms | 455ms |
| 最慢响应 | 922ms | 1249ms | 1255ms | 1255ms |
| 总 Token 消耗 | ~12000 | ~8000 | ~10000 | ~30000 |

### 6.2 工具调用统计

| 工具 | Session 1 | Session 2 | Session 3 | 总计 |
|------|-----------|-----------|-----------|------|
| calculator (WASM) | 5 | 1+ | 6+ | ~12 |
| memory_write | 3 | 0 | 0 | 3 |
| memory_search | 0 | 1+ | 3+ | ~4 |
| memory_read | 0 | 0 | 1+ | ~1 |
| **总计** | **8** | **2+** | **10+** | **~20** |

### 6.3 WASM 沙箱性能

| 指标 | 值 |
|------|-----|
| WASM 文件大小 | 141,583 bytes |
| 编译时间 | ~25ms |
| 单次执行时间 | 0-1ms |
| Fuel 消耗/次 | 15,770 — 16,868 |
| Fuel 上限 | 1,000,000 |
| 利用率 | ~1.6%（极其轻量） |

### 6.4 记忆系统统计

| 指标 | 值 |
|------|-----|
| MEMORY.md 初始大小 | 38 words |
| MEMORY.md 最终大小 | 91 words |
| 记忆条目数 | 4 |
| 自动提取次数 | 4（Session 1） |
| 相似度检查次数 | 3（防重复） |
| Daily log 写入次数 | ~10 |

### 6.5 多 Agent 统计

| 指标 | Session 2 | Session 3 |
|------|-----------|-----------|
| Job 创建数 | 2 | 3 |
| 最大并行 Agent 数 | 3（1 Chat + 2 Job） | 4（1 Chat + 3 Job） |
| Scheduler 容量利用 | 2/3 (67%) | 3/3 (100%) |
| Router 路由决策 | 7 | 8 |

---

## 7. 多 Agent 架构验证

### 7.1 LoopDelegate trait — 共享引擎 ✅

**证据**：ChatDelegate 和 JobDelegate 都通过相同的 `agentic_loop` 模块执行，日志中可以看到：
- ChatDelegate: `Launching agentic loop via ChatDelegate...`
- JobDelegate: `Entering agentic loop` (在 `tokio-rt-worker` 线程)

两者共享完全相同的：
- LLM 调用逻辑（`llm::openai`）
- 工具执行逻辑（`tools::*`）
- 迭代控制（iteration 1/10 或 1/15）
- 消息流管理（`messages_summary`）

### 7.2 ChatDelegate — 前台交互 ✅

**证据**：
- 所有自然语言输入都通过 `Router → UserInput → ChatDelegate` 路径
- 在 `main` 线程执行（阻塞式，等待 LLM 回复后才处理下一个输入）
- 维护完整的对话历史（Turn #0 → #1 → #2 → ...）
- Session 2 中成功回忆 Session 1 的信息

### 7.3 JobDelegate — 后台执行 ✅

**证据**：
- `/job` 命令通过 `Router → CreateJob → Scheduler.dispatch_job()` 路径
- 在 `tokio-rt-worker` 线程执行（非阻塞，不影响前台交互）
- 使用独立的系统提示词（"background worker agent"）
- max_iterations=15（比 ChatDelegate 的 10 更多）
- 不维护对话历史（每个 Job 独立上下文）

### 7.4 Router — 命令路由 ✅

**证据**：日志中清晰可见路由决策：
```
/job ...    → "Routing to CreateJob"
/jobs       → "Routing to ListJobs"
/status ... → "Routing to CheckJobStatus"
/cancel ... → "Routing to CancelJob"
/memory     → "User requested memory content"
自然语言     → "Routing to UserInput (no command match)"
```

### 7.5 Scheduler — 并行管理 ✅

**证据**：
- `Scheduler initialized (max_parallel_jobs=3)`
- Session 3 中成功创建 3 个并行 Job（达到上限）
- 每个 Job 有独立的 `job_id`（8 位 hex）
- Job 状态追踪：Pending → In Progress → Completed
- `/jobs` 命令正确显示所有 Job 的实时状态

### 7.6 Memory 持久化 — 跨 Session ✅

**证据**：
- Session 1：MEMORY.md 从 38 words 增长到 91 words
- Session 2：启动时读取到 91 words（Session 1 的数据完整保留）
- Session 3：启动时读取到 91 words（Session 1+2 的数据完整保留）
- Session 2/3 中 ChatDelegate 成功回答基于 Session 1 记忆的问题

---

## 8. 日志级别说明

日志使用 Rust `tracing` 框架，4 个级别：

| 级别 | 用途 | 示例 |
|------|------|------|
| **TRACE** | 最详细，包含完整数据 | Token 估算、消息流、WASM guest 日志、相似度分数 |
| **DEBUG** | 内部状态和中间结果 | 完整 API request/response body、工具参数、路由决策 |
| **INFO** | 关键事件和里程碑 | LLM 调用开始/结束、工具执行结果、Turn 完成、Job 状态变化 |
| **WARN** | 警告（本次运行无） | — |

### 日志过滤器配置

```
RUST_LOG="mini_agent_memory=trace,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn"
```

只对 `mini_agent_memory` crate 开启 trace 级别，第三方 crate 设为 warn，避免噪音。

### 线程标识

| 线程名 | 含义 |
|--------|------|
| `main` | 主线程，运行 ChatDelegate 和主输入循环 |
| `tokio-rt-worker` | Tokio 运行时工作线程，运行 JobDelegate |

日志中通过线程名可以区分前台和后台 Agent 的执行：
```
2026-04-01T12:00:54.502028Z  INFO tokio-rt-worker ...  ← 后台 Job
2026-04-01T12:00:54.502045Z  INFO            main ...  ← 前台 Chat
```

---

## 附录：快速 grep 指南

```bash
# 查看所有 Router 路由决策
grep 'Router intent\|Routing to' trace-output-raw.log

# 查看所有 LLM API 调用和响应时间
grep 'Sending HTTP POST\|HTTP response received' trace-output-raw.log

# 查看所有工具执行结果
grep 'Tool.*succeeded' trace-output-raw.log

# 查看所有 Job 生命周期事件
grep 'Dispatching new job\|Job worker started\|Job completed\|Job iteration' trace-output-raw.log

# 查看所有记忆操作
grep 'Auto-saved memory\|Appending to document\|memory_write\|memory_search' trace-output-raw.log

# 查看 WASM 沙箱执行
grep 'WASM.*sandbox\|Fuel consumed\|WASM LOG' trace-output-raw.log

# 查看 Agentic Loop 迭代
grep 'Iteration.*/' trace-output-raw.log

# 查看 Token 估算和 Compaction
grep 'Token estimate\|Compaction check\|est_tokens' trace-output-raw.log

# 查看前台 vs 后台线程交错
grep -E '(main|tokio-rt-worker).*INFO' trace-output-raw.log | head -50
```
