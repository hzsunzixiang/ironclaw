
# Mini Agent + WASM Sandbox + Long-Term Memory 全过程追踪分析

> **源文件**：`trace-output-raw.log`（4168 行，377KB）
> **测试目标**：验证跨会话的长期记忆持久化功能
> **模型**：DeepSeek-V3-0324（via http://api.haihub.cn/v1）
> **WASM 沙箱**：guest_tool.wasm（141,583 bytes），Fuel 限制 1,000,000 units/次
> **记忆存储**：workspace/MEMORY.md（Session 1 启动时 457 词 → Session 2 启动时 541 词）

---

## 总览

本次测试包含 **两个完整会话**，通过进程重启验证长期记忆的持久化能力：

| 维度 | Session 1（存储记忆） | Session 2（验证记忆） |
|------|----------------------|----------------------|
| Session ID | `7c0d478c` | `9417d422` |
| Thread ID | `cb2c3c3a` | `d75d4b88` |
| 用户交互轮次 | 7 轮（含 quit） | 6 轮（含 /memory + quit） |
| LLM 调用次数 | 9 次 | 5 次 |
| WASM 工具调用 | 9 次（calculator） | 2 次（calculator） |
| Memory 工具调用 | 1 次（memory_write） | 0 次 |
| 自动记忆提取 | 6 条 | 3 条 |
| 总耗时 | ~10.6s | ~2.9s |

### 工具注册表

系统注册了 4 个工具（1 个 WASM + 3 个 Native）：

| 工具名 | 类型 | 描述 |
|--------|------|------|
| `calculator` | WASM | 沙箱化计算器，支持 add/sub/mul/div |
| `memory_search` | Native | 搜索过往记忆和上下文 |
| `memory_write` | Native | 写入持久化记忆（MEMORY.md / daily_log） |
| `memory_read` | Native | 读取 workspace 中的文件 |

---

## Session 1：存储记忆（7 轮交互）

### 全过程时间线

```
时间轴 (UTC 2026-04-01)
│
│  04:21:10.170  🚀 程序启动，初始化 tracing
│  04:21:10.171  🔧 加载 WASM 工具 (guest_tool.wasm)
│  04:21:10.173  📦 WASM 文件加载 (141,583 bytes)
│  04:21:10.205  ✅ WASM 组件编译完成 (~32ms)
│  04:21:10.206  ✅ 元数据加载: name='calculator'
│  04:21:10.207  📂 加载 HAI_WOA.json 配置
│  04:21:10.207  ✅ 配置加载: DeepSeek-V3-0324
│  04:21:10.211  📂 初始化 Memory Store (457 words)
│  04:21:10.211  🔧 注册 4 个工具到 ToolRegistry
│  04:21:10.211  🆕 Session 7c0d478c 创建
│  04:21:10.212  🧵 Thread cb2c3c3a 创建
│  04:21:10.212  🔄 进入主输入循环
│
│  ══════ Turn 1: 自我介绍 ══════
│  04:21:10.212  📝 输入: "My name is Erick and I work at IronClaw Labs..."
│  04:21:10.212  📨 上下文: 2 条消息 (SYS → USR)
│  04:21:10.212  📤 LLM 请求发送
│  04:21:14.325  📥 LLM 响应 (4113ms) — 直接文本回复
│  04:21:14.326  🧠 自动提取记忆: "User info: My name is Erick..."
│  04:21:14.331  ✅ Turn #0 完成 (349 chars, 0 tool calls)
│
│  ══════ Turn 2: 项目信息 ══════
│  04:21:14.331  📝 输入: "Remember that our project deadline is April 15th..."
│  04:21:14.333  📨 上下文: 4 条消息 (SYS → USR → AST → USR)
│  04:21:14.333  📤 LLM 请求发送
│  04:21:14.962  📥 LLM 响应 (628ms) — 直接文本回复
│  04:21:14.962  🧠 自动提取记忆: "User asked to remember: ...Phoenix..."
│  04:21:14.963  ✅ Turn #1 完成 (234 chars, 0 tool calls)
│
│  ══════ Turn 3: 技术偏好 ══════
│  04:21:14.963  📝 输入: "I prefer using Rust for backend..."
│  04:21:14.964  📨 上下文: 6 条消息
│  04:21:14.964  📤 LLM 请求发送
│  04:21:15.846  📥 LLM 响应 (882ms) — 直接文本回复
│  04:21:15.846  🧠 自动提取记忆: "User preference: I prefer using Rust..."
│  04:21:15.849  ✅ Turn #2 完成 (222 chars, 0 tool calls)
│
│  ══════ Turn 4: 颜色偏好 ══════
│  04:21:15.850  📝 输入: "My favorite color is blue."
│  04:21:15.851  📨 上下文: 8 条消息
│  04:21:15.851  📤 LLM 请求发送
│  04:21:16.510  📥 LLM 响应 (658ms) — 直接文本回复
│  04:21:16.510  🧠 自动提取记忆: "User preference: My favorite color is blue."
│  04:21:16.513  ✅ Turn #3 完成 (172 chars, 0 tool calls)
│
│  ══════ Turn 5: 复杂数学计算 ══════
│  04:21:16.513  📝 输入: "What is 42 + 58 * 99 + (88 * 12 / 3)?"
│  04:21:16.514  📨 上下文: 10 条消息
│  04:21:16.514  📤 LLM 请求发送 (Iteration 1)
│  04:21:17.429  📥 LLM 响应 (915ms) — 请求 3 个工具调用
│  04:21:17.430  ⚙️  WASM #1: calculator(mul, 58, 99) → 5742 (0ms)
│  04:21:17.430  ⚙️  WASM #2: calculator(mul, 88, 12) → 1056 (0ms)
│  04:21:17.431  ⚙️  WASM #3: calculator(div, 1056, 3) → 352 (0ms)
│               📤 LLM 请求发送 (Iteration 2) — 带工具结果
│               📥 LLM 响应 — 请求 6 个工具调用（含重复）
│               ⚙️  WASM #4-#9: add 操作（42+5742=5784, 5784+352=6136）
│               📤 LLM 请求发送 (Iteration 3)
│               📥 LLM 响应 — 最终文本回答
│  04:21:19.457  ✅ Turn #4 完成 (答案: 6136, 9 tool calls)
│
│  ══════ Turn 6: 数据库信息 ══════
│  04:21:19.457  📝 输入: "Please remember that the database we use is PostgreSQL..."
│  04:21:19.458  📨 上下文: 12 条消息
│  04:21:19.458  📤 LLM 请求发送 (Iteration 1)
│  04:21:20.214  📥 LLM 响应 (755ms) — 请求 1 个工具调用
│               ⚙️  memory_write: 写入 MEMORY.md
│               📤 LLM 请求发送 (Iteration 2)
│               📥 LLM 响应 — 最终文本回答
│  04:21:20.843  🧠 自动提取记忆: "User asked to remember: ...PostgreSQL..."
│  04:21:20.845  ✅ Turn #5 完成 (226 chars, 1 tool call)
│
│  ══════ Turn 7: 退出 ══════
│  04:21:20.845  📝 输入: "quit"
│  04:21:20.845  👋 Session 结束 (1 thread, 6 turns)
│  04:21:20.847  📝 写入 daily log
```

---

### Turn 1：自我介绍

**用户输入**：
```
My name is Erick and I work at IronClaw Labs. I live in Shanghai.
```

**处理流程**：
1. 系统读取 MEMORY.md 构建 system prompt（注入已有记忆）
2. 构建消息上下文：`SYS → USR`（2 条消息）
3. 发送 LLM 请求（2 条消息 + 4 个工具定义）
4. LLM 识别到 MEMORY.md 中已有此信息，直接回复

**LLM 响应**（4113ms，首次请求冷启动较慢）：
```
Got it, Erick! I already have these details stored in my long-term memory. 
You work at IronClaw Labs, live in Shanghai, and are involved in the "Phoenix" 
project with a deadline on April 15th, 2026. You also prefer Rust for backend 
development and TypeScript for the frontend, and your favorite color is blue. 
Let me know how I can assist you today!
```

**自动记忆提取**：
```
🧠 Auto-saved memory: User info: My name is Erick and I work at IronClaw Labs. I live in Shanghai.
```
- 写入 `MEMORY.md`（append，79 chars）
- 写入 `daily/2026-04-01.md`（114 chars）

> **观察**：LLM 不仅确认了用户信息，还主动从 MEMORY.md 中召回了其他已知信息（Phoenix 项目、Rust/TypeScript 偏好、蓝色偏好），展示了记忆注入的效果。

---

### Turn 2：项目信息

**用户输入**：
```
Remember that our project deadline is April 15th 2026 and the project name is Phoenix.
```

**处理流程**：
1. 上下文增长到 4 条消息：`SYS → USR → AST → USR`
2. LLM 识别到 MEMORY.md 中已有此信息

**LLM 响应**（628ms）：
```
I already have this information stored in my long-term memory: your project is 
named "Phoenix," and the deadline is April 15th, 2026. Let me know if there's 
anything else you'd like me to note or assist you with regarding the project!
```

**自动记忆提取**：
```
🧠 Auto-saved memory: User asked to remember: Remember that our project deadline is April 15th 2026 and the project name is Phoenix.
```

> **观察**：LLM 没有调用 `memory_write` 工具，因为它在 system prompt 中看到了已有的记忆。但自动记忆提取机制仍然将此轮对话的关键信息追加到了 MEMORY.md。

---

### Turn 3：技术偏好

**用户输入**：
```
I prefer using Rust for backend development and TypeScript for frontend.
```

**LLM 响应**（882ms）：
```
Got it, Erick! I've already noted your preference for **Rust** in backend 
development and **TypeScript** for frontend work. If you have any specific 
questions or need assistance with these technologies, feel free to ask!
```

**自动记忆提取**：
```
🧠 Auto-saved memory: User preference: I prefer using Rust for backend development and TypeScript for frontend.
```

---

### Turn 4：颜色偏好

**用户输入**：
```
My favorite color is blue.
```

**LLM 响应**（658ms）：
```
Noted, Erick! Your favorite color is **blue**. It's already in my long-term 
memory. Let me know if there's anything else you'd like me to remember or 
assist you with!
```

**自动记忆提取**：
```
🧠 Auto-saved memory: User preference: My favorite color is blue.
```

---

### Turn 5：复杂数学计算（WASM 沙箱演示）

**用户输入**：
```
What is 42 + 58 * 99 + (88 * 12 / 3)?
```

**正确答案**：42 + 5742 + 352 = **6136**

这是本次测试中最复杂的交互，涉及 **3 轮 Agentic Loop 迭代** 和 **9 次 WASM 沙箱调用**。

#### Iteration 1/10：分解子表达式

LLM 请求 3 个并行工具调用：

| # | 操作 | 参数 | WASM 结果 | Fuel 消耗 | 耗时 |
|---|------|------|----------|----------|------|
| 1 | `mul` | 58 × 99 | 5742 | 15,975 | 0ms |
| 2 | `mul` | 88 × 12 | 1056 | 15,975 | 0ms |
| 3 | `div` | 1056 ÷ 3 | 352 | 16,061 | 0ms |

WASM 沙箱日志示例：
```
📋 [WASM LOG] [INFO] Calculating: 58 mul 99
📋 [WASM LOG] [INFO] Result: 5742 (at timestamp 1775017277430)
⛽ Fuel consumed: 15975 / 1000000 units
```

#### Iteration 2/10：组合中间结果

LLM 请求 6 个工具调用（含重复）：

| # | 操作 | 参数 | WASM 结果 | 备注 |
|---|------|------|----------|------|
| 4 | `add` | 42 + 5742 | 5784 | ✅ |
| 5 | `add` | 5784 + 352 | 6136 | ✅ |
| 6 | `add` | 42 + 5742 | 5784 | ⚠️ 重复 |
| 7 | `add` | 5784 + 352 | 6136 | ⚠️ 重复 |
| 8 | `add` | 42 + 5742 | 5784 | ⚠️ 重复 |
| 9 | `add` | 5784 + 352 | 6136 | ⚠️ 重复 |

> **观察**：DeepSeek-V3-0324 在组合中间结果时产生了冗余调用（每种计算重复 3 次）。这是模型的行为特征，不影响最终结果的正确性。

#### Iteration 3/10：最终回答

**LLM 响应**：
```
The result of 42 + 58 * 99 + (88 * 12 / 3) is **6136**.
```

**消息流演变**：
```
Iteration 1: SYS → USR → AST → USR → AST → USR → AST → USR → AST → USR
Iteration 2: ... → AST(tool_calls) → TOL → TOL → TOL
Iteration 3: ... → AST(tool_calls) → TOL → TOL → TOL → TOL → TOL → TOL
最终:         14 条消息 → 20 条消息 → LLM 返回文本
```

---

### Turn 6：数据库信息（LLM 主动调用 memory_write）

**用户输入**：
```
Please remember that the database we use is PostgreSQL and our deployment target is CentOS.
```

这是 Session 1 中唯一一次 **LLM 主动调用 memory_write 工具** 的交互。

#### Iteration 1/10：LLM 调用 memory_write

LLM 请求调用 `memory_write`：
```json
{
  "content": "Database used: PostgreSQL. Deployment target: CentOS.",
  "target": "memory",
  "append": true
}
```

工具执行结果：
```
✅ Memory written successfully to MEMORY.md (appended 53 bytes)
```

#### Iteration 2/10：确认回复

**LLM 响应**（629ms）：
```
Got it, Erick! I've noted that your database is **PostgreSQL** and your 
deployment target is **CentOS**. This is now stored in my long-term memory. 
Let me know if there's anything else you'd like me to remember or assist with!
```

**自动记忆提取**（额外）：
```
🧠 Auto-saved memory: User asked to remember: that the database we use is PostgreSQL and our deployment target is CentOS.
```

> **双重写入**：这一轮产生了两次 MEMORY.md 写入——一次来自 LLM 主动调用 `memory_write`，一次来自自动记忆提取。这说明两种记忆机制可以并行工作。

---

### Turn 7：退出

```
📝 User input received: quit
👋 User requested exit thread_count=1 total_turns=6
👋 Goodbye! Session had 1 thread(s), 6 total turn(s).
📝 Appending to daily/2026-04-01.md (55 chars)
```

---

## Session 2：验证记忆（进程重启后）

### 启动与记忆加载

```
04:21:23.596  🚀 Starting Mini Agent + WASM Sandbox + Session + Memory
04:21:23.597  🔧 Loading WASM tool...
04:21:23.598  📦 WASM file loaded (141,583 bytes)
04:21:23.632  ✅ WASM component compiled (~34ms)
04:21:23.633  ✅ Config loaded: DeepSeek-V3-0324
04:21:23.636  📂 Initializing memory store root=workspace
04:21:23.638  ✅ Memory store initialized — MEMORY.md: 541 words  ← 从 457 增长到 541
04:21:23.638  🆕 Session 9417d422 创建
04:21:23.638  🧵 Thread d75d4b88 创建
```

> **关键**：Session 2 启动时，MEMORY.md 已从 457 词增长到 541 词（Session 1 中写入的新记忆）。这些记忆被注入到 system prompt 中，使 LLM 在全新会话中也能"记住"之前的信息。

---

### Turn 1：验证姓名和工作地点

**用户输入**：
```
What is my name and where do I work?
```

**处理流程**：
1. 全新会话，上下文仅有 `SYS → USR`（2 条消息）
2. System prompt 中包含完整的 MEMORY.md 内容
3. LLM 直接从注入的记忆中回答

**LLM 响应**（686ms）：
```
Your name is Erick, and you work at IronClaw Labs. You also live in Shanghai. 
Let me know if there's anything else you'd like to know!
```

✅ **记忆验证通过**：姓名（Erick）、公司（IronClaw Labs）、城市（Shanghai）全部正确。

---

### Turn 2：验证项目信息

**用户输入**：
```
What project am I working on and when is the deadline?
```

**LLM 响应**（616ms）：
```
You're working on the project **"Phoenix"**, and the deadline is **April 15th, 2026**. 
Let me know if you'd like help with anything related to the project!
```

✅ **记忆验证通过**：项目名（Phoenix）、截止日期（April 15th, 2026）全部正确。

---

### Turn 3：验证编程语言偏好

**用户输入**：
```
What programming languages do I prefer?
```

**LLM 响应**（488ms）：
```
You prefer **Rust for backend development** and **TypeScript for frontend development**. 
Let me know if you'd like to discuss anything specific about these languages!
```

✅ **记忆验证通过**：Rust（后端）、TypeScript（前端）全部正确。

**自动记忆提取**：
```
🧠 Auto-saved memory: User preference: What programming languages do I prefer?
```

---

### Turn 4：WASM 计算器验证

**用户输入**：
```
What is 100 * 50 + 25?
```

**正确答案**：5000 + 25 = **5025**

#### Iteration 1/10：LLM 请求 2 个工具调用

| # | 操作 | 参数 | WASM 结果 | Fuel 消耗 | 耗时 |
|---|------|------|----------|----------|------|
| 1 | `mul` | 100 × 50 | 5000 | 15,802 | 1ms |
| 2 | `add` | 5000 + 25 | 5025 | 16,009 | 0ms |

WASM 沙箱日志：
```
📋 [WASM LOG] [INFO] Calculating: 100 mul 50
📋 [WASM LOG] [INFO] Result: 5000 (at timestamp 1775017286021)
⛽ Fuel consumed: 15802 / 1000000 units

📋 [WASM LOG] [INFO] Calculating: 5000 add 25
📋 [WASM LOG] [INFO] Result: 5025 (at timestamp 1775017286022)
⛽ Fuel consumed: 16009 / 1000000 units
```

#### Iteration 2/10：最终回答

**LLM 响应**（518ms）：
```
The result of 100 × 50 + 25 is **5025**. Let me know if you need any further calculations!
```

✅ **计算正确**，且 Session 2 中 LLM 没有产生重复调用（对比 Session 1 的 9 次调用）。

---

### Turn 5：查看记忆内容（/memory 命令）

**用户输入**：
```
/memory
```

系统直接读取并显示 `workspace/MEMORY.md` 的完整内容，包含所有累积的记忆条目：

```
🧠 Long-Term Memory (MEMORY.md):
─────────────────────────────────
   # Memory
   
   Long-term notes, decisions, and facts worth remembering across sessions.
   ...
   - User's name is Erick. He works at IronClaw Labs and lives in Shanghai.
   - Project deadline for 'Phoenix' is April 15th, 2026.
   - User prefers Rust for backend development and TypeScript for frontend.
   - User's favorite color is blue.
   - Database used: PostgreSQL. Deployment target: CentOS.
   ...
─────────────────────────────────
```

> **观察**：MEMORY.md 中存在大量重复条目（如 "favorite color is blue" 出现了 25+ 次）。这是因为自动记忆提取机制在每轮对话后都会追加，而没有去重逻辑。这是一个可以优化的点。

---

### Turn 6：退出

```
📝 User input received: quit
👋 User requested exit thread_count=1 total_turns=4
👋 Goodbye! Session had 1 thread(s), 4 total turn(s).
📝 Appending to daily/2026-04-01.md (55 chars)
```

---

## 记忆系统深度分析

### 记忆写入机制

系统有 **两种** 记忆写入路径：

```
┌─────────────────────────────────────────────────────────────┐
│                    用户输入                                   │
│                      │                                       │
│                      ▼                                       │
│              ┌───────────────┐                               │
│              │   LLM 处理    │                               │
│              └───────┬───────┘                               │
│                      │                                       │
│         ┌────────────┼────────────┐                          │
│         ▼                         ▼                          │
│  路径 A: LLM 主动调用      路径 B: 自动提取                    │
│  memory_write 工具         extract_memories_from_turn         │
│         │                         │                          │
│         ▼                         ▼                          │
│  ┌─────────────┐          ┌─────────────┐                    │
│  │ MEMORY.md   │          │ MEMORY.md   │                    │
│  │ (精确写入)   │          │ (自动追加)   │                    │
│  └─────────────┘          └──────┬──────┘                    │
│                                  │                           │
│                                  ▼                           │
│                          ┌─────────────┐                     │
│                          │ daily_log   │                     │
│                          │ (时间戳日志) │                     │
│                          └─────────────┘                     │
└─────────────────────────────────────────────────────────────┘
```

| 路径 | 触发条件 | 写入目标 | Session 1 中的使用 |
|------|---------|---------|-------------------|
| A: LLM 主动调用 | LLM 判断需要记忆 | MEMORY.md | Turn 6（PostgreSQL/CentOS） |
| B: 自动提取 | 每轮对话结束后 | MEMORY.md + daily_log | Turn 1-6（全部） |

### 记忆注入机制

每次处理用户输入时，系统会：

1. **读取 MEMORY.md**：`Reading document path=MEMORY.md full_path=workspace/MEMORY.md`
2. **构建 system prompt**：将 MEMORY.md 内容包裹在 `<memory>` 标签中注入
3. **发送给 LLM**：LLM 在 system prompt 中看到所有历史记忆

```
System Prompt 结构:
┌──────────────────────────────────────────┐
│ 角色定义 + 工具使用说明                     │
│                                          │
│ ## Long-Term Memory                      │
│ <memory>                                 │
│   # Memory                               │
│   - User's name is Erick...              │
│   - Project deadline for 'Phoenix'...    │
│   - User prefers Rust for backend...     │
│   - User's favorite color is blue.       │
│   - Database used: PostgreSQL...         │
│ </memory>                                │
└──────────────────────────────────────────┘
```

### 跨会话持久化验证

| 记忆项 | Session 1 写入 | Session 2 召回 | 验证结果 |
|--------|---------------|---------------|---------|
| 姓名：Erick | Turn 1 | Turn 1 ✅ | 通过 |
| 公司：IronClaw Labs | Turn 1 | Turn 1 ✅ | 通过 |
| 城市：Shanghai | Turn 1 | Turn 1 ✅ | 通过 |
| 项目：Phoenix | Turn 2 | Turn 2 ✅ | 通过 |
| 截止日期：April 15th, 2026 | Turn 2 | Turn 2 ✅ | 通过 |
| 后端语言：Rust | Turn 3 | Turn 3 ✅ | 通过 |
| 前端语言：TypeScript | Turn 3 | Turn 3 ✅ | 通过 |
| 颜色偏好：Blue | Turn 4 | (MEMORY.md 中可见) ✅ | 通过 |
| 数据库：PostgreSQL | Turn 6 | (MEMORY.md 中可见) ✅ | 通过 |
| 部署目标：CentOS | Turn 6 | (MEMORY.md 中可见) ✅ | 通过 |

**全部 10 项记忆验证通过** ✅

---

## 性能指标

### LLM 响应时间

| 会话 | 轮次 | 用户输入 | LLM 调用次数 | 总耗时 | 平均响应 |
|------|------|---------|-------------|--------|---------|
| S1 | Turn 1 | 自我介绍 | 1 | 4113ms | 4113ms（冷启动） |
| S1 | Turn 2 | 项目信息 | 1 | 628ms | 628ms |
| S1 | Turn 3 | 技术偏好 | 1 | 882ms | 882ms |
| S1 | Turn 4 | 颜色偏好 | 1 | 658ms | 658ms |
| S1 | Turn 5 | 复杂计算 | 3 | ~2943ms | ~981ms |
| S1 | Turn 6 | 数据库信息 | 2 | ~1385ms | ~692ms |
| S2 | Turn 1 | 姓名查询 | 1 | 686ms | 686ms |
| S2 | Turn 2 | 项目查询 | 1 | 616ms | 616ms |
| S2 | Turn 3 | 语言查询 | 1 | 488ms | 488ms |
| S2 | Turn 4 | 简单计算 | 2 | ~1102ms | ~551ms |

### WASM 沙箱性能

| 指标 | 值 |
|------|-----|
| WASM 文件大小 | 141,583 bytes |
| 编译时间（Session 1） | ~32ms |
| 编译时间（Session 2） | ~34ms |
| 单次工具执行时间 | 0-1ms |
| 平均 Fuel 消耗 | ~15,900 units/次 |
| Fuel 上限 | 1,000,000 units/次 |
| Fuel 使用率 | ~1.6% |

### Token 使用量

| 会话 | 轮次 | Prompt Tokens | Completion Tokens | Total |
|------|------|--------------|-------------------|-------|
| S1 | Turn 1 | 1,541 | 86 | 1,627 |
| S1 | Turn 2 | 1,672 | 56 | 1,728 |
| S1 | Turn 3 | 1,770 | 50 | 1,820 |
| S1 | Turn 4 | 1,846 | 45 | 1,891 |
| S1 | Turn 5 (iter1) | 1,923 | 90 | 2,013 |
| S1 | Turn 6 (iter1) | 2,001 | 36 | 2,037 |
| S2 | Turn 1 | 1,642 | 36 | 1,678 |
| S2 | Turn 2 | 1,692 | 42 | 1,734 |
| S2 | Turn 3 | 1,743 | 34 | 1,777 |
| S2 | Turn 4 (iter1) | 1,801 | 51 | 1,852 |

> **观察**：Prompt tokens 随着 MEMORY.md 的增长和对话历史的累积而增加。Session 2 的起始 prompt tokens（1,642）高于 Session 1（1,541），因为 MEMORY.md 从 457 词增长到了 541 词。

---

## 已知问题与优化建议

### 1. 记忆重复问题

MEMORY.md 中存在大量重复条目：
- "User's favorite color is blue" 出现 25+ 次
- "Rust for backend, TypeScript for frontend" 出现 15+ 次

**建议**：实现记忆去重/合并机制，定期清理重复条目。

### 2. 自动记忆提取质量

自动提取有时会将问题本身作为记忆存储：
```
- User preference: What programming languages do I prefer?
```
这是一个问题而非偏好信息。

**建议**：改进提取逻辑，区分陈述句和疑问句。

### 3. LLM 重复工具调用

Session 1 Turn 5 中，LLM 在 Iteration 2 产生了 6 次工具调用（实际只需 2 次），每种计算重复了 3 次。

**建议**：在 agent 层面实现工具调用去重，或在 system prompt 中明确指示避免重复调用。

### 4. Prompt Token 膨胀

随着 MEMORY.md 的无限增长，prompt tokens 会持续膨胀，增加成本和延迟。

**建议**：设置 MEMORY.md 的最大长度限制，实现自动摘要/压缩机制。

---

## 结论

本次测试成功验证了 mini-agent-memory-log 系统的核心功能：

1. ✅ **长期记忆持久化**：Session 1 中存储的所有信息在 Session 2 中均可正确召回
2. ✅ **双重记忆写入**：LLM 主动调用 `memory_write` 和自动记忆提取两种机制并行工作
3. ✅ **WASM 沙箱计算**：calculator 工具在隔离沙箱中正确执行，Fuel 机制有效限制资源
4. ✅ **会话管理**：Session/Thread 模型正确管理多轮对话上下文
5. ✅ **记忆注入**：MEMORY.md 内容成功注入 system prompt，LLM 能够利用历史记忆回答问题
6. ⚠️ **待优化**：记忆去重、提取质量、prompt token 膨胀等问题需要后续改进
