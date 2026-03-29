
# IronClaw 消息完整流转过程

> 从用户发送一条消息，到 Agent 处理、LLM 推理、工具执行（含 WASM 沙箱）、返回结果的全链路解析。

---

## 全局架构总览

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│                              IronClaw 消息流转全景                                │
│                                                                                 │
│  用户消息                                                                        │
│    │                                                                            │
│    ▼                                                                            │
│  ┌──────────────┐     ┌──────────────┐     ┌──────────────┐                     │
│  │   Channel    │────▶│  Agent Loop  │────▶│  Dispatcher  │                     │
│  │ (消息入口)    │     │  (事件循环)   │     │  (调度器)     │                     │
│  └──────────────┘     └──────────────┘     └──────┬───────┘                     │
│                                                    │                            │
│                                              ┌─────▼──────┐                     │
│                                              │ Agentic    │◀─── 循环 ───┐       │
│                                              │ Loop       │             │       │
│                                              └─────┬──────┘             │       │
│                                                    │                    │       │
│                                         ┌──────────┼──────────┐        │       │
│                                         ▼          ▼          ▼        │       │
│                                    ┌────────┐ ┌────────┐ ┌────────┐    │       │
│                                    │  LLM   │ │ Tool   │ │ WASM   │    │       │
│                                    │  Call   │ │Execute │ │Sandbox │────┘       │
│                                    └────────┘ └────────┘ └────────┘            │
│                                                    │                            │
│                                                    ▼                            │
│                                              ┌──────────┐                       │
│                                              │ Response  │──▶ 用户               │
│                                              └──────────┘                       │
└─────────────────────────────────────────────────────────────────────────────────┘
```

---

## 第 1 步：消息入口 — Channel 层

### 涉及文件
- `src/channels/channel.rs` — `Channel` trait、`IncomingMessage` 定义
- `src/channels/mod.rs` — `ChannelManager`
- `src/channels/repl.rs` — CLI 交互通道
- `src/channels/signal.rs` — Signal 通道
- `src/channels/web/` — Web Gateway 通道

### 发生了什么

用户通过某个 **Channel**（通道）发送消息。IronClaw 支持多种通道：

| 通道 | 说明 |
|------|------|
| `ReplChannel` | 命令行交互（本地开发用） |
| `HttpChannel` | HTTP API |
| `SignalChannel` | Signal 即时通讯 |
| `GatewayChannel` | Web UI（SSE + WebSocket） |
| `WasmChannel` | 动态加载的 WASM 通道 |

每个通道实现 `Channel` trait：

```rust
#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self) -> Result<MessageStream, ChannelError>;
    async fn respond(&self, msg: &IncomingMessage, response: OutgoingResponse) -> Result<(), ChannelError>;
    async fn send_status(&self, status: StatusUpdate, metadata: &serde_json::Value) -> Result<(), ChannelError>;
}
```

通道将外部消息统一转换为 `IncomingMessage`：

```rust
pub struct IncomingMessage {
    pub id: Uuid,              // 唯一消息 ID
    pub channel: String,       // 来源通道名
    pub user_id: String,       // 用户标识
    pub content: String,       // 消息内容
    pub thread_id: Option<String>,  // 会话线程 ID
    pub attachments: Vec<IncomingAttachment>,  // 附件（图片、音频、文档）
    pub metadata: serde_json::Value,  // 通道特定的元数据
    // ...
}
```

### 关键点
- **所有通道产生统一的 `IncomingMessage`**，后续流程完全一致
- `ChannelManager` 使用 `select_all` 将所有通道的消息流合并为一个 `MessageStream`
- 消息流是异步的，使用 `futures::Stream`

---

## 第 2 步：Agent 事件循环 — 接收与预处理

### 涉及文件
- `src/agent/agent_loop.rs` — `Agent::run()` 主事件循环

### 发生了什么

`Agent::run()` 是整个系统的心脏，它在一个无限循环中等待消息：

```rust
loop {
    let message = tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => { break; }  // Ctrl+C 退出
        msg = message_stream.next() => {             // 等待下一条消息
            match msg {
                Some(m) => m,
                None => { break; }
            }
        }
    };

    // 1. 语音转文字（如果有音频附件）
    if let Some(ref transcription) = self.deps.transcription {
        transcription.process(&mut message).await;
    }

    // 2. 文档提取（如果有文档附件）
    if let Some(ref doc_extraction) = self.deps.document_extraction {
        doc_extraction.process(&mut message).await;
    }

    // 3. 交给 handle_message 处理
    let response = self.handle_message(&message).await;
}
```

### `handle_message` 内部流程

```
handle_message(&message)
  │
  ├── 1. 内部消息？ → 直接转发给用户（跳过 LLM）
  │
  ├── 2. 设置消息上下文（channel、target）
  │
  ├── 3. SubmissionParser::parse() 解析消息类型
  │     ├── /undo, /redo, /compact → 控制命令
  │     ├── /help, /tools, /model → 系统命令
  │     ├── yes/no/approve → 审批响应
  │     └── 其他 → UserInput（进入 agentic loop）
  │
  ├── 4. 系统命令 → 直接处理，不进入 LLM
  │
  └── 5. UserInput → process_user_input()
```

---

## 第 3 步：用户输入处理 — Session/Thread/Turn

### 涉及文件
- `src/agent/thread_ops.rs` — `process_user_input()`
- `src/agent/session.rs` — `Session`, `Thread`, `Turn`
- `src/agent/session_manager.rs` — `SessionManager`

### 数据模型

```
Session (每个用户一个)
└── Thread (每个对话一个，可以有多个)
    └── Turn (每次请求/响应一对)
        ├── user_input: String        // 用户说了什么
        ├── response: Option<String>  // Agent 回复了什么
        ├── tool_calls: Vec<ToolCall> // 调用了哪些工具
        └── state: TurnState          // Pending | Running | Complete | Failed
```

### 发生了什么

```rust
async fn process_user_input(&self, message: &IncomingMessage) -> Result<SubmissionResult, Error> {
    // 1. 查找或创建 Session
    let (session, thread_id) = self.get_or_create_session(message).await?;

    // 2. 增强内容（附件转录、元数据）
    let augmented = augment_with_attachments(content, &message.attachments);

    // 3. 在 Thread 中开始新的 Turn
    let turn_messages = {
        let mut sess = session.lock().await;
        let thread = sess.threads.get_mut(&thread_id)?;
        let turn = thread.start_turn(effective_content);
        thread.messages()  // 收集历史消息作为 LLM 上下文
    };

    // 4. 持久化用户消息到数据库（防止崩溃丢失）
    self.persist_user_message(thread_id, &message.channel, &message.user_id, content).await;

    // 5. 发送 "Thinking..." 状态
    self.channels.send_status(&message.channel, StatusUpdate::Thinking("Processing...".into()), &message.metadata).await;

    // 6. 进入 Agentic Loop！
    let result = self.run_agentic_loop(message, tenant, session.clone(), thread_id, turn_messages).await;

    // 7. 处理结果
    match result {
        AgenticLoopResult::Response(response) => {
            thread.complete_turn(&response);
            // 持久化、发送给用户...
        }
        AgenticLoopResult::NeedApproval { pending } => {
            // 暂停，等待用户审批
        }
    }
}
```

### 关键点
- **Session 是有锁的** (`Arc<Mutex<Session>>`)，保证并发安全
- **用户消息立即持久化到 DB**，即使后续崩溃也不会丢失
- **历史消息被收集为 `Vec<ChatMessage>`**，作为 LLM 的上下文

---

## 第 4 步：Agentic Loop — 核心推理循环

### 涉及文件
- `src/agent/agentic_loop.rs` — `run_agentic_loop()` 引擎
- `src/agent/dispatcher.rs` — `ChatDelegate` 实现

### 这是整个系统最核心的部分

Agentic Loop 是一个 **循环**：调用 LLM → 如果 LLM 要求调用工具 → 执行工具 → 把结果喂回 LLM → 再次调用 LLM → ... 直到 LLM 给出文本回复。

```
┌─────────────────────────────────────────────────────────┐
│                    Agentic Loop                          │
│                                                          │
│  ┌──────────────┐                                        │
│  │ check_signals│ ← 检查是否被取消/中断                    │
│  └──────┬───────┘                                        │
│         ▼                                                │
│  ┌──────────────┐                                        │
│  │before_llm_call│ ← 刷新工具定义、注入 Skill 上下文       │
│  └──────┬───────┘                                        │
│         ▼                                                │
│  ┌──────────────┐                                        │
│  │  call_llm()  │ ← 调用 LLM（GPT-4/Claude/...）         │
│  └──────┬───────┘                                        │
│         │                                                │
│    ┌────┴────┐                                           │
│    ▼         ▼                                           │
│  文本回复   工具调用                                       │
│    │         │                                           │
│    ▼         ▼                                           │
│  返回给    execute_tool_calls()                           │
│  用户       │                                            │
│            ▼                                             │
│         工具结果 → 加入上下文 → 回到 check_signals ↑       │
│                                                          │
│  最多循环 max_iterations 次（默认 50）                     │
└─────────────────────────────────────────────────────────┘
```

### 代码结构

```rust
pub async fn run_agentic_loop(
    delegate: &dyn LoopDelegate,
    reasoning: &Reasoning,
    reason_ctx: &mut ReasoningContext,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, Error> {
    for iteration in 1..=config.max_iterations {
        // 1. 检查信号
        match delegate.check_signals().await {
            LoopSignal::Stop => return Ok(LoopOutcome::Stopped),
            LoopSignal::InjectMessage(msg) => { /* 注入消息 */ }
            LoopSignal::Continue => {}
        }

        // 2. LLM 调用前准备
        if let Some(outcome) = delegate.before_llm_call(reason_ctx, iteration).await {
            return Ok(outcome);
        }

        // 3. 调用 LLM
        let output = delegate.call_llm(reasoning, reason_ctx, iteration).await?;

        // 4. 处理 LLM 输出
        match output {
            // LLM 返回文本 → 可能结束循环
            TextResponse(text) => {
                match delegate.handle_text_response(&text, reason_ctx).await {
                    TextAction::Return(outcome) => return Ok(outcome),
                    TextAction::Continue => { /* 继续循环 */ }
                }
            }
            // LLM 要求调用工具 → 执行工具
            ToolCalls(calls) => {
                if let Some(outcome) = delegate.execute_tool_calls(calls, content, reason_ctx).await? {
                    return Ok(outcome);  // 可能需要审批
                }
            }
        }

        // 5. 迭代后处理
        delegate.after_iteration(iteration).await;
    }

    Ok(LoopOutcome::MaxIterations)
}
```

### LoopDelegate trait

三种消费者通过实现 `LoopDelegate` 来定制行为：

| Delegate | 场景 | 文件 |
|----------|------|------|
| `ChatDelegate` | 用户对话 | `dispatcher.rs` |
| `JobDelegate` | 后台任务 | `src/worker/job.rs` |
| `ContainerDelegate` | Docker 容器 | `src/worker/container.rs` |

---

## 第 5 步：LLM 调用 — 让 AI 思考

### 发生了什么

`ChatDelegate::call_llm()` 将收集到的上下文发送给 LLM：

```
发送给 LLM 的内容：
┌─────────────────────────────────────────┐
│ System Prompt                            │
│ ├── Agent 人设                           │
│ ├── 可用工具列表（含 schema）             │
│ ├── Skill 上下文（如果激活了 Skill）      │
│ └── 对话上下文（时区、通道信息等）         │
│                                          │
│ Messages (历史对话)                       │
│ ├── [user] 你好，帮我查一下天气            │
│ ├── [assistant] 好的，让我查一下           │
│ ├── [assistant] → tool_call: weather()    │
│ ├── [tool] {"temp": 25, "city": "北京"}   │
│ ├── [assistant] 北京今天 25°C             │
│ ├── [user] 帮我算一下 42 + 58 ← 当前消息  │
│                                          │
│ Available Tools                          │
│ ├── calculator: {schema...}              │
│ ├── weather: {schema...}                 │
│ ├── http_request: {schema...}            │
│ └── ...                                  │
└─────────────────────────────────────────┘
```

LLM 返回两种可能的结果：

**情况 A：文本回复**
```json
{
  "content": "42 + 58 = 100",
  "finish_reason": "stop"
}
```
→ 直接返回给用户，循环结束。

**情况 B：工具调用**
```json
{
  "content": null,
  "tool_calls": [
    {
      "id": "call_abc123",
      "name": "calculator",
      "arguments": {"operation": "add", "a": 42, "b": 58}
    }
  ]
}
```
→ 进入工具执行阶段。

---

## 第 6 步：工具执行 — execute_tool_with_safety()

### 涉及文件
- `src/tools/execute.rs` — `execute_tool_with_safety()`
- `src/tools/registry.rs` — `ToolRegistry`
- `src/tools/tool.rs` — `Tool` trait

### 执行管道

```
execute_tool_with_safety(tools, safety, "calculator", params, job_ctx)
  │
  ├── 1. 查找工具：tools.get("calculator")
  │     └── 从 ToolRegistry 中查找（可能是 Builtin、WASM、MCP）
  │
  ├── 2. 参数规范化：prepare_tool_params(tool, &params)
  │     └── 处理类型转换（字符串数组 → 真数组等）
  │
  ├── 3. 参数校验：safety.validator().validate_tool_params(&params)
  │     └── 检查注入攻击、参数合法性
  │
  ├── 4. 日志记录（敏感参数已脱敏）
  │
  ├── 5. 超时执行：tokio::time::timeout(timeout, tool.execute(params, ctx))
  │     │
  │     ├── Builtin Tool → 直接执行 Rust 代码
  │     ├── MCP Tool → 通过 MCP 协议调用外部服务
  │     └── WASM Tool → 进入 WASM 沙箱执行 ← 重点！
  │
  ├── 6. 结果序列化：serde_json::to_string_pretty(&result)
  │
  └── 7. 返回结果字符串
```

### 三种工具类型

| 类型 | 说明 | 安全级别 |
|------|------|---------|
| **Builtin** | Rust 原生实现（shell、http、file 等） | 受信任 |
| **MCP** | 通过 MCP 协议调用的外部服务 | 半受信任 |
| **WASM** | 在 WASM 沙箱中运行的第三方工具 | 不受信任 |

---

## 第 7 步：WASM 沙箱执行 — 核心安全机制

### 涉及文件
- `src/tools/wasm/wrapper.rs` — `WasmToolWrapper`（核心）
- `src/tools/wasm/runtime.rs` — `WasmToolRuntime`
- `src/tools/wasm/host.rs` — `HostState`
- `src/tools/wasm/capabilities.rs` — `Capabilities`
- `src/tools/wasm/allowlist.rs` — HTTP 白名单
- `src/tools/wasm/credential_injector.rs` — 凭证注入
- `wit/tool.wit` — WIT 接口定义

### WIT 接口（Host ↔ Guest 的契约）

```wit
package near:agent@0.3.0;

// Host 提供给 Guest 的能力（Guest 可以调用的函数）
interface host {
    log: func(level: log-level, message: string);
    now-millis: func() -> u64;
    workspace-read: func(path: string) -> option<string>;
    http-request: func(method: string, url: string, ...) -> result<http-response, string>;
    tool-invoke: func(alias: string, params-json: string) -> result<string, string>;
    secret-exists: func(name: string) -> bool;
}

// Guest 必须实现的接口（Host 调用的函数）
interface tool {
    execute: func(req: request) -> response;
    schema: func() -> string;
    description: func() -> string;
}

world sandboxed-tool {
    import host;    // Guest 可以调用 Host 的函数
    export tool;    // Guest 必须导出这些函数给 Host 调用
}
```

### WASM 执行的完整流程

```
WasmToolWrapper::execute(params, ctx)
  │
  ├── 1. 获取预编译的 WASM 模块
  │     └── PreparedModule { component, limits, capabilities }
  │
  ├── 2. 创建全新的 Store（每次执行都是全新的！）
  │     ├── WasiCtx（最小化 WASI 上下文）
  │     ├── HostState（日志收集、HTTP 代理、凭证注入）
  │     ├── Capabilities（该工具被授权的能力）
  │     └── ResourceLimiter（内存限制，默认 10MB）
  │
  ├── 3. 设置资源限制
  │     ├── Fuel: 100,000,000（CPU 指令限制）
  │     ├── Epoch Deadline（超时中断）
  │     └── Memory Limiter（内存上限）
  │
  ├── 4. 创建 Linker 并注册 Host 函数
  │     ├── wasmtime_wasi::add_to_linker_sync()  // WASI 基础
  │     └── near::agent::host::add_to_linker()   // 我们的 Host 函数
  │
  ├── 5. 实例化 Guest
  │     └── SandboxedTool::instantiate(&mut store, &component, &linker)
  │
  ├── 6. 调用 Guest 的 execute()
  │     │
  │     │  ┌─────────── WASM 沙箱边界 ───────────┐
  │     │  │                                       │
  │     │  │  Guest 代码运行在这里                   │
  │     │  │  ├── 解析 JSON 参数                    │
  │     │  │  ├── 调用 host::log() ──────────────────┼──▶ Host 收集日志
  │     │  │  ├── 调用 host::http_request() ─────────┼──▶ Host 检查白名单 → 注入凭证 → 发请求
  │     │  │  ├── 调用 host::now_millis() ───────────┼──▶ Host 返回时间戳
  │     │  │  ├── 纯计算（数学、字符串处理）          │
  │     │  │  └── 返回 Response { output, error }    │
  │     │  │                                       │
  │     │  └───────────────────────────────────────┘
  │     │
  │     └── 获取 Response
  │
  ├── 7. 收集 Guest 日志
  │     └── store.data_mut().host_state.take_logs()
  │
  ├── 8. 检查错误
  │     ├── Fuel 耗尽 → WasmError::FuelExhausted
  │     ├── Trap（unreachable）→ WasmError::Trapped
  │     └── 工具返回错误 → WasmError::ToolReturnedError
  │
  └── 9. 返回结果
        └── ToolOutput::success(result, duration)
```

### 安全机制详解

```
┌─────────────────────────────────────────────────────────────────┐
│                    WASM 安全防护层                                │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐           │
│  │  Fuel 计量   │  │  内存限制     │  │  超时中断     │           │
│  │  (CPU 限制)  │  │  (10MB 默认) │  │  (Epoch)     │           │
│  └──────────────┘  └──────────────┘  └──────────────┘           │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐           │
│  │  HTTP 白名单  │  │  凭证注入     │  │  泄露检测     │           │
│  │  (Allowlist) │  │  (Host 边界)  │  │  (LeakDetect)│           │
│  └──────────────┘  └──────────────┘  └──────────────┘           │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐           │
│  │  能力授权     │  │  无文件系统   │  │  无网络直连   │           │
│  │ (Capability) │  │  (No FS)     │  │  (No Socket) │           │
│  └──────────────┘  └──────────────┘  └──────────────┘           │
└─────────────────────────────────────────────────────────────────┘
```

| 威胁 | 防护措施 |
|------|---------|
| CPU 耗尽（死循环） | Fuel 计量 + Epoch 中断 |
| 内存耗尽 | ResourceLimiter，默认 10MB |
| 无限执行 | tokio::time::timeout |
| 文件系统访问 | 无 WASI FS，只有 host::workspace_read |
| 网络访问 | 仅白名单端点，通过 Host 代理 |
| 凭证泄露 | 凭证在 Host 边界注入，Guest 永远看不到 |
| 响应泄露 | LeakDetector 扫描所有输出 |

---

## 第 8 步：工具结果处理 — 回到 Agentic Loop

### 涉及文件
- `src/tools/execute.rs` — `process_tool_result()`
- `src/safety/` — `SafetyLayer`

### 发生了什么

工具执行完成后，结果需要经过安全处理才能喂回 LLM：

```
工具原始输出
  │
  ├── 1. 安全消毒：safety.sanitize_tool_output(tool_name, &output)
  │     └── 检查并移除敏感信息、注入攻击内容
  │
  ├── 2. XML 包装：safety.wrap_for_llm(tool_name, &sanitized.content)
  │     └── 包装为 <tool_output>...</tool_output> 格式
  │
  ├── 3. 构建 ChatMessage：ChatMessage::tool_result(call_id, tool_name, content)
  │     └── role: Tool, name: "calculator", content: "<tool_output>..."
  │
  └── 4. 加入上下文：reason_ctx.messages.push(tool_result_message)
        └── 这条消息会在下一次 LLM 调用时被包含在上下文中
```

然后 **Agentic Loop 继续下一轮迭代**：
- 再次调用 LLM，这次上下文中包含了工具的执行结果
- LLM 看到结果后，可能：
  - 给出文本回复（循环结束）
  - 继续调用其他工具（循环继续）

---

## 第 9 步：响应返回 — 从 Agent 到用户

### 涉及文件
- `src/agent/thread_ops.rs` — 结果处理
- `src/agent/dispatcher.rs` — `extract_suggestions()`
- `src/hooks/` — `HookRegistry`

### 发生了什么

当 Agentic Loop 返回文本响应后：

```
AgenticLoopResult::Response(response)
  │
  ├── 1. 提取建议：extract_suggestions(&response)
  │     └── 从 <suggestions> 标签中提取后续建议
  │
  ├── 2. Hook 处理：hooks.run(HookEvent::ResponseTransform { ... })
  │     └── 允许 Hook 修改或拒绝响应
  │
  ├── 3. 完成 Turn：thread.complete_turn(&response)
  │     └── 更新 Turn 状态为 Complete
  │
  ├── 4. 发送状态：StatusUpdate::Status("Done")
  │
  ├── 5. 持久化工具调用记录到 DB
  │
  ├── 6. 持久化 Assistant 响应到 DB
  │
  ├── 7. 发送建议（如果有）：StatusUpdate::Suggestions { ... }
  │
  └── 8. 通过 Channel 发送响应给用户
        └── channel.respond(msg, OutgoingResponse::text(response))
```

---

## 完整时序图

```
用户          Channel       Agent Loop    Dispatcher    Agentic Loop    LLM         Tool/WASM
 │               │              │              │              │           │              │
 │──"算42+58"──▶│              │              │              │           │              │
 │               │──Message───▶│              │              │           │              │
 │               │              │──parse()───▶│              │           │              │
 │               │              │              │              │           │              │
 │               │              │  process_user_input()       │           │              │
 │               │              │──────────────┼──────────────│           │              │
 │               │              │  create session/thread/turn │           │              │
 │               │              │  persist user message to DB │           │              │
 │               │              │              │              │           │              │
 │  ◀──"Thinking..."──────────│              │              │           │              │
 │               │              │              │              │           │              │
 │               │              │──run_agentic_loop()────────▶│           │              │
 │               │              │              │              │           │              │
 │               │              │              │    ┌─── Iteration 1 ───┐│              │
 │               │              │              │    │ check_signals()   ││              │
 │               │              │              │    │ before_llm_call() ││              │
 │               │              │              │    │                   ││              │
 │               │              │              │    │──call_llm()──────▶││              │
 │               │              │              │    │                   ││              │
 │               │              │              │    │◀─tool_calls───────││              │
 │               │              │              │    │  [{calculator,    ││              │
 │               │              │              │    │    {add,42,58}}]  ││              │
 │               │              │              │    │                   ││              │
 │  ◀──"ToolStarted: calculator"│              │    │                   ││              │
 │               │              │              │    │                   ││              │
 │               │              │              │    │──execute_tool()───┼┼────────────▶│
 │               │              │              │    │                   ││  ┌──WASM──┐ │
 │               │              │              │    │                   ││  │parse   │ │
 │               │              │              │    │                   ││  │compute │ │
 │               │              │              │    │                   ││  │return  │ │
 │               │              │              │    │                   ││  └────────┘ │
 │               │              │              │    │◀─result───────────┼┼─────────────│
 │               │              │              │    │  {"result":100}   ││              │
 │               │              │              │    │                   ││              │
 │  ◀──"ToolCompleted: calculator"             │    │                   ││              │
 │               │              │              │    │ sanitize + wrap   ││              │
 │               │              │              │    │ add to context    ││              │
 │               │              │              │    └───────────────────┘│              │
 │               │              │              │                        │              │
 │               │              │              │    ┌─── Iteration 2 ───┐│              │
 │               │              │              │    │──call_llm()──────▶││              │
 │               │              │              │    │  (含工具结果)      ││              │
 │               │              │              │    │                   ││              │
 │               │              │              │    │◀─text_response────││              │
 │               │              │              │    │  "42+58=100"      ││              │
 │               │              │              │    └───────────────────┘│              │
 │               │              │              │                        │              │
 │               │              │◀─Response("42+58=100")────────────────│              │
 │               │              │              │              │           │              │
 │               │              │  complete_turn()            │           │              │
 │               │              │  persist to DB              │           │              │
 │               │              │              │              │           │              │
 │  ◀──"42+58=100"────────────│              │              │           │              │
 │               │              │              │              │           │              │
```

---

## 关键设计决策总结

### 1. 统一消息格式
所有通道（CLI、HTTP、Signal、Web）产生相同的 `IncomingMessage`，后续处理完全一致。

### 2. Agentic Loop 是可插拔的
通过 `LoopDelegate` trait，同一个循环引擎服务于对话、后台任务、容器三种场景。

### 3. 工具执行有统一管道
`execute_tool_with_safety()` 是唯一的工具执行入口，所有消费者共用。

### 4. WASM 沙箱是"每次执行全新实例"
借鉴 NEAR 区块链的模式，每次执行创建全新的 Store，确保完全隔离。

### 5. 安全是多层的
```
参数校验 → 工具执行 → 输出消毒 → 泄露检测 → XML 包装 → 喂回 LLM
```

### 6. 审批机制
需要审批的工具会暂停 Agentic Loop，等待用户确认后继续。

---

## 文件索引

| 阶段 | 关键文件 | 作用 |
|------|---------|------|
| 消息入口 | `src/channels/channel.rs` | Channel trait、IncomingMessage |
| 消息入口 | `src/channels/mod.rs` | ChannelManager |
| 事件循环 | `src/agent/agent_loop.rs` | Agent::run() 主循环 |
| 消息解析 | `src/agent/submission.rs` | SubmissionParser |
| 会话管理 | `src/agent/session.rs` | Session/Thread/Turn |
| 用户输入 | `src/agent/thread_ops.rs` | process_user_input() |
| 调度器 | `src/agent/dispatcher.rs` | ChatDelegate |
| 核心循环 | `src/agent/agentic_loop.rs` | run_agentic_loop() |
| 工具执行 | `src/tools/execute.rs` | execute_tool_with_safety() |
| 工具注册 | `src/tools/registry.rs` | ToolRegistry |
| WASM 运行时 | `src/tools/wasm/runtime.rs` | WasmToolRuntime |
| WASM 包装器 | `src/tools/wasm/wrapper.rs` | WasmToolWrapper |
| WASM Host | `src/tools/wasm/host.rs` | HostState |
| WIT 接口 | `wit/tool.wit` | Host ↔ Guest 契约 |
| 安全层 | `src/safety/` | SafetyLayer |
