# WIT 接口定义逐句解读

> 本文档对 `wit/tool.wit` 文件进行**逐句解释**，帮助理解 WIT（WebAssembly Interface Types）语言的语法和语义。
>
> WIT 是 Host 和 Guest 之间的"合约"——双方都读取同一个 WIT 文件来生成代码，确保接口一致。

---

## 文件全貌

```wit
package demo:sandbox@0.1.0;

interface host { ... }
interface tool { ... }

world sandboxed-tool {
    import host;
    export tool;
}
```

整个文件定义了：
- **1 个包声明** — 命名空间和版本
- **2 个接口** — `host`（Host 提供给 Guest 的能力）和 `tool`（Guest 必须实现的功能）
- **1 个 world** — 把 import 和 export 组合在一起，定义完整的沙箱边界

---

## 第 1 行：包声明

```wit
package demo:sandbox@0.1.0;
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `package` | 关键字，声明这是一个 WIT 包 |
| `demo` | **命名空间**（namespace），类似 Java 的 `com.example` 或 npm 的 `@scope` |
| `:` | 分隔符，分隔命名空间和包名 |
| `sandbox` | **包名**（package name） |
| `@0.1.0` | **语义化版本号**，遵循 semver 规范 |
| `;` | 语句结束符 |

### 这行的影响

这个包名会直接影响 Rust 代码中生成的**模块路径**：

```
demo:sandbox  →  demo::sandbox::host    (Host 接口的模块路径)
                  demo::sandbox::tool    (Tool 接口的模块路径)
```

所以在 Guest 代码中你会看到：

```rust
use demo::sandbox::host;                           // 调用 Host 函数
use exports::demo::sandbox::tool::{Guest, Request}; // 实现 Guest trait
```

### 类比

| WIT | Java | npm |
|-----|------|-----|
| `package demo:sandbox@0.1.0` | `package com.demo.sandbox` | `@demo/sandbox@0.1.0` |

---

## 第 3-11 行：模块注释

```wit
// Minimal WASM Tool Sandbox Interface
//
// This is a simplified version of IronClaw's WIT interface,
// designed to demonstrate the core WASM sandbox mechanism.
//
// Key idea:
//   - The HOST provides functions (imports) that the guest can call
//   - The GUEST exports functions that the host can call
//   - The guest CANNOT do anything the host doesn't explicitly provide
```

### 逐句翻译

| 原文 | 翻译 |
|------|------|
| Minimal WASM Tool Sandbox Interface | 最小化的 WASM 工具沙箱接口 |
| This is a simplified version of IronClaw's WIT interface | 这是 IronClaw 项目 WIT 接口的简化版本 |
| designed to demonstrate the core WASM sandbox mechanism | 用于演示 WASM 沙箱的核心机制 |
| The HOST provides functions (imports) that the guest can call | Host 提供函数（imports），Guest 可以调用 |
| The GUEST exports functions that the host can call | Guest 导出函数（exports），Host 可以调用 |
| The guest CANNOT do anything the host doesn't explicitly provide | Guest 不能做任何 Host 没有明确提供的事情 |

### 注释语法

WIT 支持两种注释：
- `//` — 普通注释，不会出现在生成的代码中
- `///` — **文档注释**，会被转换为生成代码中的文档（如 Rust 的 `///` doc comment）

这里用的是 `//`，所以只是给人看的说明，不影响代码生成。

---

## 第 13-17 行：`host` 接口的文档注释

```wit
/// Host-provided capabilities for sandboxed tools.
///
/// These are the ONLY ways a sandboxed tool can interact with the outside world.
/// If a function isn't listed here, the WASM guest simply cannot do it.
/// No filesystem, no network, no system calls — only what the host allows.
```

### 逐句翻译

| 原文 | 翻译 |
|------|------|
| Host-provided capabilities for sandboxed tools | Host 为沙箱工具提供的能力 |
| These are the ONLY ways a sandboxed tool can interact with the outside world | 这是沙箱工具与外部世界交互的**唯一途径** |
| If a function isn't listed here, the WASM guest simply cannot do it | 如果一个函数没有列在这里，WASM Guest 就完全做不了这件事 |
| No filesystem, no network, no system calls — only what the host allows | 没有文件系统、没有网络、没有系统调用——只有 Host 允许的 |

### 关键理解

这段注释强调了 WASM 沙箱的**白名单安全模型**：不是"禁止某些危险操作"，而是"只允许明确列出的操作"。这比传统的黑名单安全模型（如 Linux seccomp）更安全，因为不可能遗漏。

---

## 第 18 行：`interface host` 声明

```wit
interface host {
```

### 解释

| 部分 | 含义 |
|------|------|
| `interface` | 关键字，定义一个接口（一组类型和函数的集合） |
| `host` | 接口名称 |
| `{` | 接口体开始 |

`interface` 在 WIT 中类似于：
- Rust 的 `trait`
- Java 的 `interface`
- Go 的 `interface`
- TypeScript 的 `interface`

但有一个关键区别：WIT 的 `interface` 不仅定义函数签名，还可以定义**类型**（enum、record 等）。

### 生成的代码

这个 `interface host` 会在两端生成不同的代码：

| 端 | 生成内容 |
|----|---------|
| Guest 端 | `demo::sandbox::host` 模块，包含可调用的函数（`host::log()`、`host::now_millis()`） |
| Host 端 | `demo::sandbox::host::Host` trait，Host 必须实现 |

---

## 第 19-20 行：`log-level` 枚举的文档注释

```wit
    /// Log levels for structured logging.
    enum log-level {
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `///` | 文档注释，会出现在生成的 Rust 代码的 doc comment 中 |
| `enum` | 关键字，定义一个枚举类型 |
| `log-level` | 枚举名称。注意 WIT 使用**kebab-case**（短横线命名法） |
| `{` | 枚举体开始 |

### WIT 命名规范 → Rust 命名转换

WIT 统一使用 kebab-case，生成 Rust 代码时会自动转换：

| WIT (kebab-case) | Rust (PascalCase / snake_case) |
|-------------------|-------------------------------|
| `log-level` | `LogLevel`（类型名 → PascalCase） |
| `now-millis` | `now_millis`（函数名 → snake_case） |

---

## 第 21-23 行：枚举变体

```wit
        info,
        warn,
        error,
```

### 解释

定义了三个枚举变体（variant），表示日志级别：

| 变体 | 含义 | 生成的 Rust 代码 |
|------|------|-----------------|
| `info` | 信息级别 | `LogLevel::Info` |
| `warn` | 警告级别 | `LogLevel::Warn` |
| `error` | 错误级别 | `LogLevel::Error` |

WIT 的 `enum` 是**简单枚举**（无关联数据），类似 C 的 enum。如果需要带数据的枚举，WIT 使用 `variant` 关键字。

### 第 24 行：枚举结束

```wit
    }
```

---

## 第 26-27 行：`log` 函数

```wit
    /// Emit a log message. The host decides what to do with it.
    log: func(level: log-level, message: string);
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `///` | 文档注释："发送一条日志消息。Host 决定如何处理它。" |
| `log` | 函数名 |
| `:` | 类型标注分隔符 |
| `func` | 关键字，表示这是一个函数 |
| `(` | 参数列表开始 |
| `level: log-level` | 第一个参数：名为 `level`，类型为 `log-level` 枚举 |
| `,` | 参数分隔符 |
| `message: string` | 第二个参数：名为 `message`，类型为 `string`（WIT 内置类型） |
| `)` | 参数列表结束 |
| `;` | 语句结束。**没有返回值**（void 函数） |

### 生成的代码

**Guest 端**（可以调用）：

```rust
// Guest 调用这个函数
host::log(host::LogLevel::Info, "hello");
```

**Host 端**（必须实现）：

```rust
// Host 实现这个函数
impl demo::sandbox::host::Host for StoreData {
    fn log(&mut self, level: LogLevel, message: String) {
        println!("[{:?}] {}", level, message);
    }
}
```

### 关键理解

注释说 "The host decides what to do with it"——这是沙箱的核心。Guest 只能**请求**记录日志，Host 可以选择：
- ✅ 打印到终端
- ✅ 写入文件
- ✅ 发送到日志服务
- ✅ 静默丢弃
- ✅ 限流（每秒最多 N 条）

Guest 无法控制 Host 的行为。

---

## 第 29-31 行：`now-millis` 函数

```wit
    /// Get the current timestamp in milliseconds since Unix epoch.
    /// The guest has no other way to know the time!
    now-millis: func() -> u64;
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `///` | 文档注释（两行）："获取自 Unix 纪元以来的毫秒时间戳。Guest 没有其他方式知道时间！" |
| `now-millis` | 函数名（kebab-case，生成 Rust 代码时变为 `now_millis`） |
| `func()` | 无参数的函数 |
| `->` | 返回值标注 |
| `u64` | 返回类型：64 位无符号整数 |
| `;` | 语句结束 |

### WIT 内置类型

WIT 提供了一组内置的基本类型：

| WIT 类型 | Rust 类型 | 说明 |
|----------|----------|------|
| `u8`, `u16`, `u32`, `u64` | `u8`, `u16`, `u32`, `u64` | 无符号整数 |
| `s8`, `s16`, `s32`, `s64` | `i8`, `i16`, `i32`, `i64` | 有符号整数 |
| `f32`, `f64` | `f32`, `f64` | 浮点数 |
| `bool` | `bool` | 布尔值 |
| `string` | `String` | UTF-8 字符串 |
| `char` | `char` | Unicode 字符 |

### 关键理解

注释 "The guest has no other way to know the time!" 强调了沙箱的隔离性——Guest 无法直接调用 `SystemTime::now()`（那是操作系统 API），只能通过 Host 提供的这个函数获取时间。Host 甚至可以返回假时间用于确定性测试。

---

## 第 32 行：`host` 接口结束

```wit
}
```

至此，`host` 接口定义完毕。它总共包含：
- 1 个枚举类型（`log-level`）
- 2 个函数（`log`、`now-millis`）

这就是 Guest 与外部世界交互的**全部能力**。

---

## 第 34-36 行：`tool` 接口的文档注释和声明

```wit
/// Tool interface that sandboxed tools must implement (export).
///
/// The host calls these functions to interact with the tool.
interface tool {
```

### 翻译

- "沙箱工具必须实现（导出）的工具接口。"
- "Host 通过调用这些函数与工具交互。"

### 与 `host` 接口的对比

| | `interface host` | `interface tool` |
|---|---|---|
| 谁提供？ | Host 提供 | Guest 提供 |
| 谁调用？ | Guest 调用 | Host 调用 |
| 在 world 中的角色 | `import`（导入） | `export`（导出） |

---

## 第 38-41 行：`request` 记录类型

```wit
    /// Request payload for tool execution.
    record request {
        /// JSON-encoded parameters.
        params: string,
    }
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `///` | 文档注释："工具执行的请求载荷" |
| `record` | 关键字，定义一个记录类型（类似 Rust 的 struct） |
| `request` | 类型名称 |
| `params: string` | 字段：名为 `params`，类型为 `string` |

### `record` vs 其他语言

| WIT | Rust | Java | TypeScript | Go |
|-----|------|------|------------|-----|
| `record` | `struct` | `class` / `record` | `interface` | `struct` |

### 生成的 Rust 代码

```rust
pub struct Request {
    pub params: String,
}
```

### 设计选择

为什么 `params` 是 `string` 而不是更结构化的类型？因为 WIT 层面不关心具体的业务参数格式——不同的工具有不同的参数结构。用 JSON 字符串作为"万能容器"，让每个 Guest 自己解析，保持了接口的通用性。

---

## 第 43-48 行：`response` 记录类型

```wit
    /// Response from tool execution.
    record response {
        /// JSON-encoded output on success.
        output: option<string>,
        /// Error message on failure.
        error: option<string>,
    }
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `record response` | 定义名为 `response` 的记录类型 |
| `output: option<string>` | 可选字段：成功时的 JSON 输出 |
| `error: option<string>` | 可选字段：失败时的错误信息 |

### `option<T>` 类型

WIT 的 `option<T>` 等价于：

| WIT | Rust | Java | TypeScript | Go |
|-----|------|------|------------|-----|
| `option<string>` | `Option<String>` | `Optional<String>` | `string \| null` | `*string` |

表示"这个值可能存在，也可能不存在"。

### 生成的 Rust 代码

```rust
pub struct Response {
    pub output: Option<String>,
    pub error: Option<String>,
}
```

### 设计模式

`output` 和 `error` 都是 `option`，形成了四种可能的组合：

| `output` | `error` | 含义 |
|----------|---------|------|
| `Some(json)` | `None` | ✅ 成功，有输出 |
| `None` | `Some(msg)` | ❌ 失败，有错误信息 |
| `Some(json)` | `Some(msg)` | ⚠️ 部分成功（有输出但也有警告） |
| `None` | `None` | ⚠️ 成功但无输出（理论上不应出现） |

这种设计比 `result<string, string>` 更灵活，允许同时返回输出和错误信息。

---

## 第 50-51 行：`execute` 函数

```wit
    /// Execute the tool with the given request.
    execute: func(req: request) -> response;
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `execute` | 函数名 |
| `func(req: request)` | 接受一个参数 `req`，类型为前面定义的 `request` 记录 |
| `-> response` | 返回前面定义的 `response` 记录 |

这是工具的**核心函数**——Host 调用它来执行工具的业务逻辑。

### 生成的代码

**Guest 端**（必须实现）：

```rust
impl Guest for CalculatorTool {
    fn execute(req: Request) -> Response { ... }
}
```

**Host 端**（可以调用）：

```rust
let response = tool.demo_sandbox_tool().call_execute(&mut store, &request)?;
```

---

## 第 53-54 行：`schema` 函数

```wit
    /// Get the JSON Schema for this tool's parameters.
    schema: func() -> string;
```

### 解释

| 部分 | 含义 |
|------|------|
| `schema` | 函数名 |
| `func()` | 无参数 |
| `-> string` | 返回一个字符串（JSON Schema 格式） |

这个函数让 Host 能够在**不执行工具**的情况下，了解工具需要什么参数。返回的是 [JSON Schema](https://json-schema.org/) 格式的字符串，例如：

```json
{
    "type": "object",
    "properties": {
        "operation": { "type": "string", "enum": ["add", "sub", "mul", "div"] },
        "a": { "type": "number" },
        "b": { "type": "number" }
    },
    "required": ["operation", "a", "b"]
}
```

### 用途

在 AI Agent 场景中，Host 可以把这个 schema 传给 LLM，让 LLM 知道如何构造正确的参数来调用这个工具。

---

## 第 56-57 行：`description` 函数

```wit
    /// Get a human-readable description of what this tool does.
    description: func() -> string;
```

### 解释

| 部分 | 含义 |
|------|------|
| `description` | 函数名 |
| `func()` | 无参数 |
| `-> string` | 返回一个人类可读的描述字符串 |

例如返回：`"A sandboxed calculator tool. Performs basic math operations (add, sub, mul, div) inside a WASM sandbox."`

### 用途

同样用于 AI Agent 场景——LLM 通过 `description()` 了解工具的功能，决定是否使用这个工具。

---

## 第 58 行：`tool` 接口结束

```wit
}
```

至此，`tool` 接口定义完毕。它总共包含：
- 2 个记录类型（`request`、`response`）
- 3 个函数（`execute`、`schema`、`description`）

---

## 第 60-61 行：`world` 定义的文档注释和声明

```wit
/// World definition: what the guest imports and exports.
world sandboxed-tool {
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `world` | 关键字，定义一个"世界"——WASM 组件的完整接口描述 |
| `sandboxed-tool` | world 名称 |

### 什么是 `world`？

`world` 是 WIT 中最顶层的概念。它把所有的 `import` 和 `export` 组合在一起，定义了一个 WASM 组件的**完整边界**——它能用什么（import），它提供什么（export）。

类比：
- `interface` 像是一份**菜单**（列出可用的菜品）
- `world` 像是一份**合同**（规定餐厅必须提供哪些菜单，顾客可以点哪些菜单）

### 这个名称的影响

`world` 的名称会在代码生成宏中引用：

```rust
// Guest 端
wit_bindgen::generate!({
    world: "sandboxed-tool",  // ← 引用这个 world
    path: "../wit/tool.wit",
});

// Host 端
wasmtime::component::bindgen!({
    world: "sandboxed-tool",  // ← 引用同一个 world
    path: "../wit/tool.wit",
});
```

---

## 第 62 行：`import host`

```wit
    import host;       // Guest can call these (provided by host)
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `import` | 关键字，声明 Guest **导入**（使用）的接口 |
| `host` | 引用前面定义的 `interface host` |
| `;` | 语句结束 |

### 含义

"Guest 可以调用 `host` 接口中定义的所有函数。"

具体来说，Guest 可以调用：
- `host::log(level, message)` — 发送日志
- `host::now_millis()` — 获取时间

**但仅此而已**。Guest 不能调用任何其他函数。

### 谁负责提供实现？

**Host 负责**。Host 必须实现 `host` 接口中的所有函数，并通过 Linker 注册：

```rust
// Host 端：实现 trait
impl demo::sandbox::host::Host for StoreData {
    fn log(&mut self, level: LogLevel, message: String) { ... }
    fn now_millis(&mut self) -> u64 { ... }
}

// Host 端：注册到 Linker
demo::sandbox::host::add_to_linker(&mut linker, |state| state)?;
```

---

## 第 63 行：`export tool`

```wit
    export tool;       // Guest must implement these (called by host)
```

### 逐词解释

| 部分 | 含义 |
|------|------|
| `export` | 关键字，声明 Guest **导出**（提供）的接口 |
| `tool` | 引用前面定义的 `interface tool` |
| `;` | 语句结束 |

### 含义

"Guest 必须实现 `tool` 接口中定义的所有函数，供 Host 调用。"

具体来说，Guest 必须实现：
- `execute(req) -> response` — 执行工具
- `schema() -> string` — 返回参数 schema
- `description() -> string` — 返回工具描述

### 谁负责提供实现？

**Guest 负责**。Guest 必须实现 `tool` 接口中的所有函数：

```rust
// Guest 端：实现 trait
struct CalculatorTool;
impl Guest for CalculatorTool {
    fn execute(req: Request) -> Response { ... }
    fn schema() -> String { ... }
    fn description() -> String { ... }
}

// Guest 端：注册导出
export!(CalculatorTool);
```

---

## 第 64 行：`world` 结束

```wit
}
```

---

## 总结：`import` 与 `export` 的对称性

```
                    ┌─────────────────────┐
                    │   WIT 文件 (合约)     │
                    │                     │
                    │  world sandboxed-tool│
                    │    import host;     │
                    │    export tool;     │
                    └──────────┬──────────┘
                               │
              ┌────────────────┼────────────────┐
              ▼                                 ▼
    ┌──────────────────┐              ┌──────────────────┐
    │   Guest 端        │              │   Host 端         │
    │                  │              │                  │
    │ import host →    │              │ import host →    │
    │   可以调用的函数   │              │   必须实现的 trait  │
    │                  │              │                  │
    │ export tool →    │              │ export tool →    │
    │   必须实现的 trait │              │   可以调用的方法    │
    └──────────────────┘              └──────────────────┘
```

同一个 WIT 文件，在两端生成**镜像对称**的代码：
- Guest 的 `import` = Host 的 `export`（Guest 调用，Host 实现）
- Guest 的 `export` = Host 的 `import`（Guest 实现，Host 调用）

这就是 WIT 的精髓——**一份合约，两端对称，接口一致**。
