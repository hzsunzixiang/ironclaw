
# Multi-Agent 实现方案

> 基于 IronClaw 核心架构，为 `mini-agent-multiagent-log` 添加多 Agent 支持的完整演进路线。

## 目录

- [背景与目标](#背景与目标)
- [IronClaw 多 Agent 架构概览](#ironclaw-多-agent-架构概览)
- [演进路线总览](#演进路线总览)
- [Phase 1: LoopDelegate + ChatDelegate ✅](#phase-1-loopdelegate--chatdelegate-)
- [Phase 2: Scheduler + JobDelegate ✅](#phase-2-scheduler--jobdelegate-)
- [Phase 3: Router（统一命令路由）✅](#phase-3-router统一命令路由)
- [Phase 4: Agent Profile（不同 Agent 使用不同 LLM/工具集）](#phase-4-agent-profile不同-agent-使用不同-llm工具集)
- [模块映射表](#模块映射表)
- [附录：IronClaw 完整 Agent 模块参考](#附录ironclaw-完整-agent-模块参考)

---

## 背景与目标

### 当前状态（mini-agent-compress）

```
用户输入 → 单一 Agentic Loop → 单一 LLM + 工具集 → 响应
```

只有一个 `process_user_input()` 函数，使用固定的 LLM provider 和 ToolRegistry。所有操作都在前台同步执行，无法并行处理多个任务。

### 目标状态（mini-agent-multiagent）

```
用户输入 → Router → ChatDelegate (前台交互)
                   → Scheduler → JobDelegate (后台并行)
                   → (未来) ContainerDelegate (沙箱执行)
```

支持多种类型的 Agent 并行运行，每种 Agent 可以有不同的 LLM、工具集和行为策略。

---

## IronClaw 多 Agent 架构概览

IronClaw 通过 **LoopDelegate trait** 实现了统一的 agentic loop 引擎，三种不同类型的 Agent 共享同一个循环逻辑：

```
┌─────────────────────────────────────────────────────────────────────┐
│                     IronClaw Agent Architecture                      │
│                                                                      │
│  ┌──────────┐    ┌──────────────────────────────────────────────┐   │
│  │  Input    │───▶│  Router / SubmissionParser                   │   │
│  │  (Web/CLI)│◀──│    ├── UserInput → ChatDelegate (foreground)  │   │
│  └──────────┘    │    ├── /job      → Scheduler → JobDelegate   │   │
│                  │    └── /container → ContainerDelegate          │   │
│                  └──────────────────────────────────────────────┘   │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Shared Agentic Loop Engine (agentic_loop.rs)                │   │
│  │                                                              │   │
│  │    run_agentic_loop(delegate, reasoning, reason_ctx, config) │   │
│  │      1. Check signals (stop/cancel) via delegate             │   │
│  │      2. Pre-LLM hook via delegate                            │   │
│  │      3. LLM call via delegate                                │   │
│  │      4. Text response → delegate.handle_text_response()      │   │
│  │      5. Tool calls → delegate.execute_tool_calls()           │   │
│  │      6. Post-iteration hook via delegate                     │   │
│  │      7. Repeat until LoopOutcome or max_iterations           │   │
│  │                                                              │   │
│  │  Three LoopDelegate implementations:                         │   │
│  │    ├── ChatDelegate   (dispatcher.rs)  — 交互式对话          │   │
│  │    ├── JobDelegate    (worker/job.rs)  — 后台任务            │   │
│  │    └── ContainerDelegate (worker/container.rs) — Docker 沙箱 │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │  Scheduler (scheduler.rs)                                    │   │
│  │    ├── jobs: HashMap<Uuid, ScheduledJob>      (LLM-driven)   │   │
│  │    ├── subtasks: HashMap<Uuid, Subtask>       (lightweight)  │   │
│  │    ├── dispatch_job() → Worker → run_agentic_loop()          │   │
│  │    └── spawn_subtask() / spawn_batch()                       │   │
│  └──────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### 核心设计模式

| 模式 | IronClaw 实现 | 说明 |
|------|-------------|------|
| **LoopDelegate trait** | `agentic_loop.rs` | 统一的 agentic loop 引擎，通过 trait 多态支持不同类型的 agent |
| **ChatDelegate** | `dispatcher.rs` | 交互式对话 agent（前台），支持 tool approval、skill context injection |
| **JobDelegate** | `worker/job.rs` | 后台任务 agent，支持 planning、completion detection |
| **ContainerDelegate** | `worker/container.rs` | Docker 沙箱 agent，sequential tool exec、HTTP event streaming |
| **Scheduler** | `scheduler.rs` | 管理并行 job + subtask，mpsc channel 通信 |
| **Router** | `router.rs` | 将 `/commands` 路由到 `MessageIntent` |
| **CostGuard** | `cost_guard.rs` | LLM 费用和调用频率控制 |
| **SelfRepair** | `self_repair.rs` | 检测 stuck jobs 和 broken tools，自动恢复 |

---

## 演进路线总览

```
mini-agent-loop           → 最小 agent（无沙箱、无状态）
    ↓
mini-agent-wasm           → + WASM 沙箱（安全工具执行）
    ↓
mini-agent-wasm-session   → + Session/Thread/Turn（短期记忆）
    ↓
mini-agent-memory         → + Workspace memory（长期记忆）
    ↓
mini-agent-compress       → + Memory dedup + quality + compaction
    ↓
mini-agent-multiagent     → + Multi-agent（本项目）
    ├── Phase 1: LoopDelegate + ChatDelegate     ✅ 已完成
    ├── Phase 2: Scheduler + JobDelegate          ✅ 已完成
    ├── Phase 3: Router（统一命令路由）            ✅ 已完成
    └── Phase 4: Agent Profile（不同 LLM/工具集）  📋 待实现
```

> **注意**：Phase 1–3 在本次实现中一起完成，因为它们紧密耦合。

---

## Phase 1: LoopDelegate + ChatDelegate ✅

### 目标

将硬编码的 agentic loop 重构为基于 trait 的设计，不改变外部行为。

### 设计原理

#### 问题：为什么需要重构？

在 `mini-agent-compress` 中，agentic loop 是一个**硬编码的函数**：

```rust
// mini-agent-compress/host/src/agent.rs — 重构前
async fn run_agentic_loop(
    llm: &dyn LlmProvider,          // ← 直接接收具体依赖
    registry: &ToolRegistry,         // ← 直接接收具体依赖
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    for iteration in 1..=config.max_iterations {
        let response = llm.chat(messages, &tool_defs).await?;  // ← 硬编码 LLM 调用
        match response.result {
            LlmOutput::Text(text) => {
                return Ok(LoopOutcome::Response { text, tool_calls });  // ← 硬编码：text = 结束
            }
            LlmOutput::ToolCalls { tool_calls, .. } => {
                // ← 硬编码工具执行
                let result = execute_tool_with_safety(registry, &tc.name, tc.arguments.clone()).await;
                // ...
            }
        }
    }
}
```

这个设计有三个根本性限制：

| 限制 | 说明 | 影响 |
|------|------|------|
| **行为不可变** | text 响应永远意味着"结束"，无法实现"text 但继续"的语义 | 无法支持 JobDelegate 的 planning 模式 |
| **信号不可检** | 没有 check_signals 机制，循环一旦开始就无法从外部中断 | 无法支持后台任务取消 |
| **依赖硬绑定** | `llm` 和 `registry` 作为参数直接传入，所有调用者共享同一套 | 无法让不同 Agent 使用不同的 LLM 或工具集 |

这三个限制使得**同一个 loop 引擎无法服务于不同类型的 Agent**。

#### 解法：Strategy Pattern（策略模式）

核心思想是 **GoF Strategy Pattern** 的 Rust 实现：将 agentic loop 中**变化的部分**抽取为 trait 方法，**不变的部分**保留在共享引擎中。

```
┌─────────────────────────────────────────────────────────────────┐
│                    变化的部分 (LoopDelegate)                      │
│                                                                  │
│  ┌─────────────────┐  ┌─────────────────┐  ┌────────────────┐  │
│  │  check_signals   │  │ handle_text_resp │  │  execute_tool  │  │
│  │  before_llm_call │  │ call_llm         │  │  after_iter    │  │
│  └─────────────────┘  └─────────────────┘  └────────────────┘  │
│         ▲                     ▲                     ▲           │
│         │                     │                     │           │
│  ┌──────┴──────┐       ┌─────┴──────┐       ┌──────┴──────┐   │
│  │ ChatDelegate │       │ JobDelegate │       │ (Future...)  │   │
│  └─────────────┘       └────────────┘       └─────────────┘   │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│                  不变的部分 (run_agentic_loop)                    │
│                                                                  │
│  for iteration in 1..=max_iterations {                           │
│      check_signals()          // ← 委托给 delegate              │
│      before_llm_call()        // ← 委托给 delegate              │
│      call_llm()               // ← 委托给 delegate              │
│      match response {                                            │
│          Text → handle_text_response()   // ← 委托给 delegate   │
│          ToolCalls → execute_tool()       // ← 委托给 delegate   │
│      }                                                           │
│      after_iteration()        // ← 委托给 delegate              │
│  }                                                               │
│                                                                  │
│  // 不变的逻辑：迭代计数、消息管理、tool result 拼装、            │
│  // truncation 恢复、recorded_tool_calls 收集                    │
└─────────────────────────────────────────────────────────────────┘
```

#### 变化点分析：哪些行为需要多态？

通过对比 ChatDelegate（前台对话）和 JobDelegate（后台任务）的需求差异，识别出 7 个变化点：

| # | 变化点 | ChatDelegate 行为 | JobDelegate 行为 | trait 方法 |
|---|--------|-------------------|------------------|-----------|
| 1 | **外部信号** | 无（CLI 模式无中断） | mpsc channel 检查 Stop 消息 | `check_signals()` |
| 2 | **LLM 调用前** | 无特殊处理 | 可注入 planning prompt | `before_llm_call()` |
| 3 | **LLM 调用** | 直接调用 `llm.chat()` | 通过 `Arc<dyn LlmProvider>` 调用 | `call_llm()` |
| 4 | **text 响应处理** | text = 结束，返回 Response | text = 结束，返回 Response（但可扩展为 Continue） | `handle_text_response()` |
| 5 | **工具执行** | 通过 `&ToolRegistry` 执行 | 通过 `Arc<ToolRegistry>` 执行 | `execute_tool()` |
| 6 | **工具定义** | `registry.definitions()` | `registry.definitions()`（未来可过滤） | `tool_definitions()` |
| 7 | **迭代后处理** | 无 | 可更新 job 进度 | `after_iteration()` |

#### 关键设计决策

**决策 1：`&dyn LoopDelegate` vs 泛型 `<D: LoopDelegate>`**

选择 **trait object**（`&dyn LoopDelegate`）而非泛型，原因：
- 避免为每种 delegate 生成独立的 `run_agentic_loop` 单态化副本（减少二进制体积）
- loop 引擎内部没有热路径需要内联优化（LLM 调用和工具执行本身就是 I/O bound）
- 与 IronClaw 保持一致

```rust
// 选择 trait object
pub async fn run_agentic_loop(
    delegate: &dyn LoopDelegate,  // ← dynamic dispatch
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String>
```

**决策 2：ChatDelegate 用借用，JobDelegate 用 Arc**

两种 delegate 的**生命周期需求**不同：

```rust
// ChatDelegate — 借用引用，生命周期绑定到 process_user_input() 调用栈
struct ChatDelegate<'a> {
    llm: &'a dyn LlmProvider,      // ← 借用，零成本
    registry: &'a ToolRegistry,     // ← 借用，零成本
}

// JobDelegate — Arc 所有权，因为要 tokio::spawn 到独立任务
struct JobDelegate {
    llm: Arc<dyn LlmProvider>,      // ← 共享所有权
    registry: Arc<ToolRegistry>,    // ← 共享所有权
    stop_rx: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,  // ← 跨任务通信
}
```

这是因为 `tokio::spawn` 要求 `'static` 生命周期，借用引用无法满足。ChatDelegate 不需要 spawn，所以用借用更高效。

**决策 3：`Send + Sync` bound**

`LoopDelegate` 要求 `Send + Sync`：

```rust
#[async_trait]
pub trait LoopDelegate: Send + Sync { ... }
```

- `Send`：delegate 可能跨线程传递（`tokio::spawn`）
- `Sync`：`&dyn LoopDelegate` 需要 `Sync`（因为 `run_agentic_loop` 接收 `&dyn`）
- ChatDelegate 的 `&'a dyn LlmProvider` 和 `&'a ToolRegistry` 天然满足（因为 `LlmProvider: Send + Sync`）

**决策 4：`LoopOutcome` 中携带 `tool_calls`**

```rust
pub enum LoopOutcome {
    Response { text: String, tool_calls: Vec<RecordedToolCall> },
    Stopped  { tool_calls: Vec<RecordedToolCall> },
    MaxIterations { tool_calls: Vec<RecordedToolCall> },
}
```

每个变体都携带 `tool_calls`，这样调用者（`process_user_input`）可以将工具调用记录到 Turn 中，无论循环以何种方式结束。这比让 delegate 自己记录更简洁，因为记录逻辑对所有 delegate 都相同。

#### 重构手法：Extract → Implement → Replace

整个重构分三步，每步都保持编译通过和行为不变：

```
Step 1: Extract（抽取）
  agent.rs 中的 run_agentic_loop() 函数体
    → 移动到 agentic_loop.rs
    → 将硬编码的 llm.chat() / execute_tool_with_safety() 替换为 delegate 方法调用
    → 新增 LoopDelegate trait + LoopSignal/TextAction/LoopOutcome 类型

Step 2: Implement（实现）
  在 agent.rs 中创建 ChatDelegate struct
    → 实现 LoopDelegate trait
    → 每个方法的实现 = 原来硬编码的行为（1:1 对应）

Step 3: Replace（替换）
  agent.rs 中的 process_user_input()
    → 原来: run_agentic_loop(llm, registry, &mut messages, config)
    → 现在: let delegate = ChatDelegate { llm, registry };
            run_agentic_loop(&delegate, &mut messages, config)
```

**行为等价性证明**：

| 原始代码 | ChatDelegate 实现 | 等价？ |
|---------|-------------------|--------|
| `llm.chat(messages, &tool_defs).await` | `call_llm()` → `self.llm.chat(messages, tool_defs).await` | ✅ |
| `return Ok(LoopOutcome::Response { text, .. })` | `handle_text_response()` → `TextAction::Return(LoopOutcome::Response { .. })` | ✅ |
| `execute_tool_with_safety(registry, ..)` | `execute_tool()` → `execute_tool_with_safety(self.registry, ..)` | ✅ |
| `registry.definitions()` | `tool_definitions()` → `self.registry.definitions()` | ✅ |
| （无信号检查） | `check_signals()` → `LoopSignal::Continue` | ✅ (no-op) |
| （无 pre-LLM hook） | `before_llm_call()` → `None` | ✅ (no-op) |
| （无 post-iteration hook） | `after_iteration()` → `{}` | ✅ (no-op) |

所有 ChatDelegate 方法都是原始硬编码行为的 1:1 映射，新增的 hook 点（check_signals、before_llm_call、after_iteration）在 ChatDelegate 中都是 no-op，因此**外部行为完全不变**。

#### 与 IronClaw 的对应关系

```
IronClaw                              mini-agent-multiagent
────────                              ─────────────────────
src/agent/agentic_loop.rs             host/src/agentic_loop.rs
  ├── trait LoopDelegate              ├── trait LoopDelegate
  │     ├── check_signals()           │     ├── check_signals()
  │     ├── before_llm_call()         │     ├── before_llm_call()
  │     ├── call_llm()               │     ├── call_llm()
  │     ├── handle_text_response()    │     ├── handle_text_response()
  │     ├── execute_tool_calls()      │     ├── execute_tool()        ← 简化：单个工具
  │     └── after_iteration()         │     └── after_iteration()
  ├── run_agentic_loop()              ├── run_agentic_loop()
  ├── LoopSignal                      ├── LoopSignal
  ├── TextAction                      ├── TextAction
  ├── LoopOutcome                     ├── LoopOutcome
  └── AgenticLoopConfig               └── AgenticLoopConfig

src/agent/dispatcher.rs               host/src/agent.rs
  └── ChatDelegate                    └── ChatDelegate
        ├── impl LoopDelegate               ├── impl LoopDelegate
        └── (skill injection, approval)     └── (simplified: no skills/approval)

src/agent/thread_ops.rs               host/src/agent.rs
  └── process_user_input()            └── process_user_input()
```

**简化点**：
- IronClaw 的 `execute_tool_calls()` 接收整个 `Vec<ToolCall>`，mini-agent 简化为 `execute_tool()` 接收单个工具调用（循环在引擎中完成）
- IronClaw 的 ChatDelegate 包含 skill injection 和 tool approval，mini-agent 省略
- IronClaw 的 `run_agentic_loop` 还接收 `reasoning` 和 `reason_ctx` 参数（用于 extended thinking），mini-agent 省略

### 核心变更

**新增文件：`agentic_loop.rs`**

抽取 `LoopDelegate` trait 和共享的 `run_agentic_loop()` 引擎：

```rust
/// Strategy trait — each consumer implements this to customize I/O and lifecycle.
#[async_trait]
pub trait LoopDelegate: Send + Sync {
    /// Check for external signals (cancellation, user messages, stop requests).
    async fn check_signals(&self) -> LoopSignal;

    /// Called before the LLM call. Allows refreshing tool definitions, cost guards, etc.
    async fn before_llm_call(
        &self,
        messages: &mut Vec<ChatMessage>,
        iteration: usize,
    ) -> Option<LoopOutcome>;

    /// Call the LLM with current messages and tool definitions.
    async fn call_llm(
        &self,
        messages: &mut Vec<ChatMessage>,
        tool_defs: &[ToolDefinition],
    ) -> Result<LlmResponse, String>;

    /// Handle a text-only response from the LLM.
    async fn handle_text_response(
        &self,
        text: &str,
        recorded_tool_calls: &[RecordedToolCall],
    ) -> TextAction;

    /// Execute a single tool call.
    async fn execute_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, String>;

    /// Get the current tool definitions.
    fn tool_definitions(&self) -> Vec<ToolDefinition>;

    /// Called after each successful iteration.
    async fn after_iteration(&self, _iteration: usize) {}
}
```

**关键类型：**

| 类型 | 说明 |
|------|------|
| `LoopSignal` | `Continue` / `Stop` / `InjectMessage(String)` |
| `TextAction` | `Return(LoopOutcome)` / `Continue` |
| `LoopOutcome` | `Response { text, tool_calls }` / `Stopped` / `MaxIterations` |
| `AgenticLoopConfig` | `max_iterations`, `context_token_limit`, `compaction_threshold` 等 |

**重构文件：`agent.rs`**

原来的 `process_user_input()` 中硬编码的 agentic loop 被替换为 `ChatDelegate`：

```rust
/// ChatDelegate — LoopDelegate for interactive chat sessions.
struct ChatDelegate<'a> {
    llm: &'a dyn LlmProvider,
    registry: &'a ToolRegistry,
}

#[async_trait]
impl<'a> LoopDelegate for ChatDelegate<'a> {
    async fn check_signals(&self) -> LoopSignal {
        LoopSignal::Continue  // CLI mode: no external signals
    }

    async fn handle_text_response(&self, text: &str, _: &[RecordedToolCall]) -> TextAction {
        TextAction::Return(LoopOutcome::Response { ... })  // Text = done
    }

    async fn execute_tool(&self, name: &str, args: Value) -> Result<String, String> {
        execute_tool_with_safety(self.registry, name, args).await
    }
    // ...
}
```

### 架构变化

```
Before:                              After:
┌──────────────────┐                 ┌──────────────────┐
│ process_user_input│                 │ process_user_input│
│   ├── build msgs  │                 │   ├── build msgs  │
│   ├── for loop {  │                 │   ├── ChatDelegate│
│   │   call LLM    │    ──────▶     │   └── run_agentic_loop(delegate, ...)
│   │   exec tools  │                 │                    │
│   │   process     │                 │ run_agentic_loop() │ ← shared engine
│   │ }             │                 │   ├── check_signals│
│   └── record turn │                 │   ├── call_llm     │
└──────────────────┘                 │   ├── exec tools   │
                                     │   └── record turn  │
                                     └──────────────────┘
```

### IronClaw 映射

| mini-agent | IronClaw |
|------------|----------|
| `agentic_loop.rs` → `LoopDelegate` | `src/agent/agentic_loop.rs` → `trait LoopDelegate` |
| `agentic_loop.rs` → `run_agentic_loop()` | `src/agent/agentic_loop.rs` → `run_agentic_loop()` |
| `agent.rs` → `ChatDelegate` | `src/agent/dispatcher.rs` → `ChatDelegate` |
| `agent.rs` → `process_user_input()` | `src/agent/thread_ops.rs` → `process_user_input()` |

---

## Phase 2: Scheduler + JobDelegate ✅

### 目标

添加后台任务执行能力，支持 `/job` 命令创建独立的 agentic loop。

### 核心变更

**新增文件：`scheduler.rs`**

包含 `Scheduler`、`JobDelegate`、`Worker` 和 `JobInfo`：

```rust
/// Job scheduler for parallel execution.
pub struct Scheduler {
    llm: Arc<dyn LlmProvider>,
    registry: Arc<ToolRegistry>,
    memory_store: Arc<RwLock<MemoryStore>>,
    jobs: Arc<RwLock<HashMap<Uuid, ScheduledJob>>>,
    job_info: Arc<RwLock<HashMap<Uuid, JobInfo>>>,
    max_parallel_jobs: usize,  // default: 3
}

impl Scheduler {
    pub async fn dispatch_job(&self, description: &str) -> Result<Uuid, String>;
    pub async fn cancel_job(&self, job_id: Uuid) -> Result<(), String>;
    pub async fn list_jobs(&self) -> Vec<JobInfo>;
    pub async fn find_job_by_short_id(&self, short_id: &str) -> Option<JobInfo>;
}
```

**JobDelegate — 后台任务的 LoopDelegate 实现：**

```rust
/// Delegate for background job execution.
struct JobDelegate {
    job_id: Uuid,
    llm: Arc<dyn LlmProvider>,
    registry: Arc<ToolRegistry>,
    stop_rx: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
    stopped: Arc<Mutex<bool>>,
}

#[async_trait]
impl LoopDelegate for JobDelegate {
    async fn check_signals(&self) -> LoopSignal {
        // Non-blocking check for stop messages via mpsc channel
        match rx.try_recv() {
            Ok(WorkerMessage::Stop) => LoopSignal::Stop,
            _ => LoopSignal::Continue,
        }
    }
    // ...
}
```

### ChatDelegate vs JobDelegate 对比

| 特性 | ChatDelegate | JobDelegate |
|------|-------------|-------------|
| **生命周期** | 借用引用 (`&'a`) | `Arc`-based 所有权 |
| **运行方式** | 同步，在用户输入处理中 | `tokio::spawn` 异步任务 |
| **Session 感知** | 是（记录 Turn） | 否（独立运行） |
| **信号检查** | 无（CLI 模式） | mpsc channel（Stop/Start） |
| **text 响应** | 立即返回给用户 | 记录到 JobInfo |
| **最大迭代** | 10（默认） | 15（更多空间完成任务） |
| **系统提示** | 通用对话 + memory | 任务导向 + memory |

### Worker 通信模型

```
Scheduler                          Worker (tokio::spawn)
    │                                  │
    ├── mpsc::channel(16) ────────────▶│
    │                                  │
    ├── tx.send(Start) ──────────────▶ │ ← 开始执行
    │                                  │
    │   (job running...)               │ ← run_agentic_loop(JobDelegate)
    │                                  │
    ├── tx.send(Stop) ───────────────▶ │ ← 取消信号
    │                                  │
    │                                  │ ← check_signals() 检测到 Stop
    │                                  │
    │   cleanup task polls             │
    │   handle.is_finished() ─────────▶│ ← 清理完成的 job
    │                                  │
```

### IronClaw 映射

| mini-agent | IronClaw |
|------------|----------|
| `scheduler.rs` → `Scheduler` | `src/agent/scheduler.rs` → `Scheduler` |
| `scheduler.rs` → `JobDelegate` | `src/worker/job.rs` → `JobDelegate` |
| `scheduler.rs` → `WorkerMessage` | `src/agent/scheduler.rs` → `WorkerMessage` |
| `scheduler.rs` → `ScheduledJob` | `src/agent/scheduler.rs` → `ScheduledJob` |
| `scheduler.rs` → `JobState` | `src/context/mod.rs` → `JobState` |
| `scheduler.rs` → `JobInfo` (in-memory) | IronClaw: Database-backed via `ContextManager` |

---

## Phase 3: Router（统一命令路由）✅

### 目标

将用户输入路由到不同的处理路径，而不是在 main loop 中硬编码 if-else。

### 核心变更

**新增文件：`router.rs`**

```rust
/// Intent extracted from a message.
#[derive(Debug, Clone)]
pub enum MessageIntent {
    CreateJob { description: String },
    CheckJobStatus { job_id: String },
    CancelJob { job_id: String },
    ListJobs,
    UserInput { content: String },
}

/// Message router — parses `/commands` into `MessageIntent`.
pub struct Router;

impl Router {
    pub fn route(input: &str) -> MessageIntent {
        // /job <desc>   → CreateJob
        // /jobs          → ListJobs
        // /status <id>   → CheckJobStatus
        // /cancel <id>   → CancelJob
        // everything else → UserInput
    }
}
```

### 路由流程

```
用户输入
    │
    ▼
┌─────────────────────────────┐
│ handle_session_command()     │ ← /new, /threads, /switch, /history, /session
│ (session + memory commands)  │   /memory, /memory-search, /memory-tree
│                             │   quit, exit
└──────────┬──────────────────┘
           │ NotACommand
           ▼
┌─────────────────────────────┐
│ handle_model_command()       │ ← /models, /model <name>
└──────────┬──────────────────┘
           │ None
           ▼
┌─────────────────────────────┐
│ Router::route(input)         │ ← 解析为 MessageIntent
└──────────┬──────────────────┘
           │
     ┌─────┴─────────────────┐
     │                       │
     ▼                       ▼
┌──────────┐          ┌──────────────┐
│ Job cmds │          │ UserInput    │
│ /job     │          │ → ChatDelegate│
│ /jobs    │          │ → agentic loop│
│ /status  │          └──────────────┘
│ /cancel  │
└──────────┘
```

### 命令总览

| 命令 | 类别 | 处理函数 |
|------|------|---------|
| `/new` | Session | `handle_session_command()` |
| `/threads` | Session | `handle_session_command()` |
| `/switch <n>` | Session | `handle_session_command()` |
| `/history` | Session | `handle_session_command()` |
| `/session` | Session | `handle_session_command()` |
| `/memory` | Memory | `handle_session_command()` |
| `/memory-search <q>` | Memory | `handle_session_command()` |
| `/memory-tree` | Memory | `handle_session_command()` |
| `/models` | Model | `handle_model_command()` |
| `/model <name>` | Model | `handle_model_command()` |
| `/job <desc>` | **Job (NEW)** | `handle_job_command()` via Router |
| `/jobs` | **Job (NEW)** | `handle_job_command()` via Router |
| `/status <id>` | **Job (NEW)** | `handle_job_command()` via Router |
| `/cancel <id>` | **Job (NEW)** | `handle_job_command()` via Router |
| `quit` / `exit` | Control | `handle_session_command()` |
| 其他文本 | Chat | `handle_user_input()` via Router |

### IronClaw 映射

| mini-agent | IronClaw |
|------------|----------|
| `router.rs` → `Router` | `src/agent/router.rs` → `Router` |
| `router.rs` → `MessageIntent` | `src/agent/router.rs` → `MessageIntent` |
| `commands.rs` → `handle_job_command()` | `src/agent/commands.rs` → job intent handlers |

---

## Phase 4: Agent Profile（不同 Agent 使用不同 LLM/工具集）

### 目标

支持不同的 Agent 使用不同的 LLM 模型和工具集，实现真正的异构多 Agent 系统。

### 状态：📋 待实现

### 设计方案

#### 4.1 AgentProfile 配置

```rust
/// Profile defining an agent's capabilities and configuration.
///
/// Maps to: IronClaw's per-job metadata + tool filtering
pub struct AgentProfile {
    /// Profile name (e.g., "researcher", "coder", "analyst").
    pub name: String,

    /// LLM model to use (overrides default).
    /// None = use the session's current model.
    pub model: Option<String>,

    /// System prompt template.
    /// Supports placeholders: {task}, {memory}, {model}.
    pub system_prompt_template: String,

    /// Tool filter: which tools this agent can use.
    /// None = all tools available.
    pub allowed_tools: Option<Vec<String>>,

    /// Max iterations for the agentic loop.
    pub max_iterations: usize,

    /// Whether to inject long-term memory into system prompt.
    pub use_memory: bool,
}
```

#### 4.2 预定义 Profile

```rust
impl AgentProfile {
    /// General-purpose chat agent (default).
    pub fn chat() -> Self {
        Self {
            name: "chat".to_string(),
            model: None,
            system_prompt_template: DEFAULT_CHAT_PROMPT.to_string(),
            allowed_tools: None,
            max_iterations: 10,
            use_memory: true,
        }
    }

    /// Math/calculation specialist.
    pub fn calculator() -> Self {
        Self {
            name: "calculator".to_string(),
            model: None,
            system_prompt_template: CALCULATOR_PROMPT.to_string(),
            allowed_tools: Some(vec!["calculator".to_string()]),
            max_iterations: 5,
            use_memory: false,
        }
    }

    /// Memory management specialist.
    pub fn memory_manager() -> Self {
        Self {
            name: "memory_manager".to_string(),
            model: None,
            system_prompt_template: MEMORY_MANAGER_PROMPT.to_string(),
            allowed_tools: Some(vec![
                "memory_search".to_string(),
                "memory_write".to_string(),
                "memory_read".to_string(),
            ]),
            max_iterations: 8,
            use_memory: true,
        }
    }

    /// Research agent using a cheaper/faster model.
    pub fn researcher() -> Self {
        Self {
            name: "researcher".to_string(),
            model: Some("gpt-4o-mini".to_string()),  // cheaper model
            system_prompt_template: RESEARCHER_PROMPT.to_string(),
            allowed_tools: None,
            max_iterations: 15,
            use_memory: true,
        }
    }
}
```

#### 4.3 ToolRegistry 过滤

```rust
impl ToolRegistry {
    /// Create a filtered view of the registry for a specific profile.
    pub fn filtered(&self, allowed_tools: &[String]) -> FilteredToolRegistry {
        FilteredToolRegistry {
            inner: self,
            allowed: allowed_tools.to_vec(),
        }
    }
}

pub struct FilteredToolRegistry<'a> {
    inner: &'a ToolRegistry,
    allowed: Vec<String>,
}

impl<'a> FilteredToolRegistry<'a> {
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.inner.definitions()
            .into_iter()
            .filter(|d| self.allowed.contains(&d.name))
            .collect()
    }
}
```

#### 4.4 命令扩展

```
/job <desc>                    → 使用默认 profile 创建 job
/job --profile calculator <desc> → 使用 calculator profile
/job --model gpt-4o-mini <desc>  → 指定 LLM 模型
/profiles                       → 列出可用 profiles
```

#### 4.5 Scheduler 集成

```rust
impl Scheduler {
    pub async fn dispatch_job_with_profile(
        &self,
        description: &str,
        profile: &AgentProfile,
    ) -> Result<Uuid, String> {
        // 1. Resolve LLM provider (profile.model or default)
        let llm = match &profile.model {
            Some(model) => self.make_provider(model)?,
            None => Arc::clone(&self.llm),
        };

        // 2. Filter tools based on profile
        let tool_defs = match &profile.allowed_tools {
            Some(allowed) => self.registry.filtered(allowed).definitions(),
            None => self.registry.definitions(),
        };

        // 3. Build system prompt from template
        let system_prompt = profile.build_system_prompt(&self.memory_store, description);

        // 4. Create JobDelegate with profile-specific config
        let config = AgenticLoopConfig {
            max_iterations: profile.max_iterations,
            ..Default::default()
        };

        // 5. Spawn worker
        // ...
    }
}
```

#### 4.6 IronClaw 映射

| mini-agent (Phase 4) | IronClaw |
|----------------------|----------|
| `AgentProfile` | Per-job metadata + `AgentConfig` |
| `FilteredToolRegistry` | Tool filtering in `ToolStore` |
| `profile.model` | `AgentDeps.cheap_llm` / per-job LLM override |
| `--profile` flag | Job metadata in `ContextManager` |

#### 4.7 实现步骤

1. **新增 `profile.rs`** — `AgentProfile` 结构体 + 预定义 profiles
2. **修改 `tools/mod.rs`** — 添加 `FilteredToolRegistry`
3. **修改 `scheduler.rs`** — `dispatch_job_with_profile()` 方法
4. **修改 `router.rs`** — 解析 `--profile` 和 `--model` 参数
5. **修改 `commands.rs`** — 添加 `/profiles` 命令
6. **修改 `main.rs`** — 注册预定义 profiles

---

## 模块映射表

### 完整模块结构

```
host/src/
├── main.rs              ← 入口 + 初始化 + 主循环
├── agentic_loop.rs      ← LoopDelegate trait + run_agentic_loop() 引擎
├── agent.rs             ← ChatDelegate + process_user_input()
├── router.rs            ← Router + MessageIntent
├── scheduler.rs         ← Scheduler + JobDelegate + Worker
├── commands.rs          ← CLI 命令处理
├── session.rs           ← Session/Thread/Turn 模型
├── memory.rs            ← 持久化记忆存储
├── utils.rs             ← 工具函数
├── llm/
│   ├── mod.rs           ← LLM 类型定义
│   ├── provider.rs      ← LlmProvider trait
│   └── openai.rs        ← OpenAI 兼容实现
└── tools/
    ├── mod.rs           ← ToolRegistry + Tool trait
    ├── wasm.rs          ← WASM 沙箱工具
    └── memory_tools.rs  ← 原生记忆工具
```

### IronClaw 映射

| mini-agent 模块 | IronClaw 源文件 | 职责 |
|-----------------|----------------|------|
| `agentic_loop.rs` | `src/agent/agentic_loop.rs` | 共享 loop 引擎 + LoopDelegate trait |
| `agent.rs` | `src/agent/dispatcher.rs` + `thread_ops.rs` | ChatDelegate + 用户输入处理 |
| `scheduler.rs` | `src/agent/scheduler.rs` + `src/worker/job.rs` | Scheduler + JobDelegate |
| `router.rs` | `src/agent/router.rs` | 命令路由 |
| `commands.rs` | `src/agent/commands.rs` | 命令处理 |
| `session.rs` | `src/agent/session.rs` | Session/Thread/Turn |
| `memory.rs` | `src/workspace/` | 持久化记忆 |
| `llm/` | `src/llm/provider.rs` | LLM 抽象 |
| `tools/` | `src/tools/` + `src/wasm/` | 工具注册 + WASM 沙箱 |

### IronClaw 中有但 mini-agent 中简化/省略的模块

| IronClaw 模块 | 说明 | mini-agent 中的处理 |
|--------------|------|-------------------|
| `compaction.rs` | 三种压缩策略 (MoveToWorkspace/Summarize/Truncate) | 简化为 `truncate_turns()` |
| `context_monitor.rs` | Token 使用监控 | 简化为 `estimate_tokens()` + `needs_compaction()` |
| `self_repair.rs` | Stuck job 检测和恢复 | 未实现 |
| `heartbeat.rs` | 定期主动执行 | 未实现 |
| `cost_guard.rs` | LLM 费用和频率控制 | 未实现 |
| `undo.rs` | Turn-based undo/redo | 未实现 |
| `routine.rs` / `routine_engine.rs` | Cron/event 触发的例行任务 | 未实现 |
| `submission.rs` | 统一的用户输入解析 | 简化为 `Router::route()` |
| `session_manager.rs` | 多用户 session 管理 | 简化为单用户 `Session` |
| `worker/container.rs` | Docker 容器 agent | 未实现 |

---

## 附录：IronClaw 完整 Agent 模块参考

> 摘自 `src/agent/CLAUDE.md`

### Session / Thread / Turn 模型

```
Session (per user)
└── Thread (per conversation — can have many)
    └── Turn (per request/response pair)
        ├── user_input: String
        ├── response: Option<String>
        ├── tool_calls: Vec<ToolCall>
        └── state: TurnState (Pending | Running | Complete | Failed)
```

### Agentic Loop 流程

```
run_agentic_loop(delegate, reasoning, reason_ctx, config)
  1. Check signals (stop/cancel) via delegate.check_signals()
  2. Pre-LLM hook via delegate.before_llm_call()
  3. LLM call via delegate.call_llm()
  4. If text response → delegate.handle_text_response() → Continue or Return
  5. If tool calls → delegate.execute_tool_calls() → Continue or Return
  6. Post-iteration hook via delegate.after_iteration()
  7. Repeat until LoopOutcome returned or max_iterations reached
```

### Scheduler 设计

- `jobs` — full LLM-driven jobs, each with a `Worker` and `mpsc` channel for `WorkerMessage`
- `subtasks` — lightweight `ToolExec` or `Background` tasks via `spawn_subtask()` / `spawn_batch()`
- Check-insert under single write lock to prevent TOCTOU races
- Cleanup task polls every second for job completion

### Key Invariants

- Never call `.unwrap()` or `.expect()` — use `?` with proper error mapping
- All state mutations on `Session`/`Thread` happen under `Arc<Mutex<Session>>` lock
- Agent loop is single-threaded per thread; parallel execution at job/scheduler level
- Tool results pass through `SafetyLayer` before returning to LLM
- `CostGuard.check_allowed()` before LLM calls; `record_llm_call()` after
