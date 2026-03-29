# Mini Agent WASM — 逐行精读教程

> **读者画像**：你是一个 C/C++ 程序员，懂 Python 和 Erlang，已经读过 mini-agent-loop 的教程，现在想搞懂 WASM 沙箱是怎么回事。
> 本教程会用 C/C++ 的概念做类比，重点讲解 **WASM 沙箱**这个新增部分，对于与 mini-agent-loop 相同的部分会简要带过。

---

## 目录

1. [项目全景：从 mini-agent-loop 到 WASM 沙箱](#1-项目全景)
2. [工程结构：Host / Guest / WIT 三件套](#2-工程结构)
3. [WIT 接口定义：tool.wit](#3-wit-接口定义)
4. [Guest 端：WASM 插件 — guest/src/lib.rs](#4-guest-端)
5. [Host 端全景：host/src/main.rs 的六大部分](#5-host-端全景)
6. [PART 1: LLM 类型与 Provider Trait（复用层）](#6-part-1-llm-层)
7. [PART 2: Tool Trait 与 WASM 沙箱基础设施（核心新增）](#7-part-2-wasm-沙箱)
8. [PART 3: Agentic Loop（不变的引擎）](#8-part-3-agentic-loop)
9. [PART 4: Mock LLM Provider（测试利器）](#9-part-4-mock-llm)
10. [PART 5: OpenAI 兼容 Provider（真实 API）](#10-part-5-openai-provider)
11. [PART 6: Main 入口（组装一切）](#11-part-6-main)
12. [数据流全景图](#12-数据流全景图)
13. [WASM 沙箱安全模型深度解析](#13-wasm-沙箱安全模型)
14. [LLDB 调试指南](#14-lldb-调试指南)
15. [与 mini-agent-loop 的对比总结](#15-对比总结)

---

## 1. 项目全景

### 它在 mini-agent-loop 基础上做了什么？

mini-agent-loop 的工具（Calculator）是 **原生 Rust 代码**，和 Agent 跑在同一个进程里，共享内存空间。这意味着：
- 工具可以访问文件系统、网络、环境变量
- 工具可以无限循环，卡死整个 Agent
- 工具可以读取 Agent 的内存（API Key 等敏感信息）

mini-agent-wasm 把工具放进了 **WASM 沙箱**：

```
mini-agent-loop:
  Agent 进程 ──直接调用──▶ CalculatorTool::execute()  (同一进程)

mini-agent-wasm:
  Agent 进程 ──Wasmtime VM──▶ guest_tool.wasm::execute()  (隔离沙箱)
```

### 用 C 的思维理解 WASM 沙箱

想象你在写一个 **插件系统**。在 C 中，你有几种选择：

| 方案 | 隔离级别 | 性能 | 复杂度 |
|------|---------|------|--------|
| `dlopen` + 共享库 | 无隔离（共享地址空间） | 最快 | 低 |
| `fork` + 子进程 | 进程级隔离 | 中等 | 中 |
| `seccomp` + 沙箱 | 系统调用过滤 | 快 | 高 |
| **WASM 沙箱** | **完全隔离（无系统调用）** | **接近原生** | **中** |

WASM 沙箱相当于：**你给插件一个完全空白的地址空间，没有任何系统调用，连 `malloc` 都是沙箱内部自己管理的。插件唯一能做的就是调用你显式提供的函数。**

### 用 Erlang 的思维理解

Erlang 的进程天然隔离——每个进程有独立的堆，不能直接访问其他进程的内存。WASM 沙箱提供了类似的隔离，但在单进程内实现：

```erlang
%% Erlang: 进程隔离
ToolPid = spawn(fun() -> calculator:execute(Params) end),
Result = gen_server:call(ToolPid, execute, 30000).

%% WASM: 沙箱隔离（概念等价）
Store = wasmtime:new_store(Engine),
Result = wasmtime:call(Store, "execute", Params).
```

### 架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                        Host Process                             │
│                                                                 │
│  ┌──────────┐    ┌──────────┐    ┌──────────────────────────┐  │
│  │  stdin    │───▶│  Agent   │───▶│  LLM (DeepSeek/Qwen)    │  │
│  │  stdout   │◀──│  Loop    │◀──│  via ~/HAI_WOA.json      │  │
│  └──────────┘    └────┬─────┘    └──────────────────────────┘  │
│                       │                                         │
│                       │ tool_call(calculator, params)           │
│                       ▼                                         │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              WASM Sandbox (Wasmtime)                      │  │
│  │  ┌────────────────────────────────────────────────────┐  │  │
│  │  │  Guest Tool (.wasm)                                │  │  │
│  │  │                                                    │  │  │
│  │  │  • Can ONLY call host::log() and host::now_millis()│  │  │
│  │  │  • Cannot access filesystem, network, env vars     │  │  │
│  │  │  • Fuel-limited (prevents infinite loops)          │  │  │
│  │  │  • Fresh instance per execution (no state leak)    │  │  │
│  │  └────────────────────────────────────────────────────┘  │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## 2. 工程结构

### 目录结构

```
mini-agent-wasm/
├── Makefile                    # 构建脚本（编排 guest + host 的编译）
├── wit/
│   └── tool.wit                # WIT 接口定义（guest 和 host 之间的"合同"）
├── guest/                      # WASM 插件端（编译成 .wasm）
│   ├── Cargo.toml
│   ├── Cargo.lock
│   └── src/
│       └── lib.rs              # Calculator 工具的 WASM 实现
└── host/                       # 宿主端（编译成原生二进制）
    ├── Cargo.toml
    ├── Cargo.lock
    └── src/
        └── main.rs             # Agent Loop + WASM 运行时（1192 行，全部在一个文件）
```

### 与 mini-agent-loop 的结构对比

```
mini-agent-loop/                    mini-agent-wasm/
├── src/                            ├── wit/tool.wit          ← 新增：接口合同
│   ├── main.rs                     ├── guest/src/lib.rs      ← 新增：WASM 插件
│   ├── agent.rs                    └── host/src/main.rs      ← 合并：所有 host 代码
│   ├── llm/
│   │   ├── mod.rs
│   │   ├── provider.rs
│   │   └── openai.rs
│   └── tools/
│       ├── mod.rs
│       └── calculator.rs
```

mini-agent-wasm 把 host 端的所有代码合并到了一个 `main.rs` 里（1192 行），分成 6 个 PART，用注释清晰分隔。这是教学项目的常见做法——一个文件从头读到尾，不用在文件间跳转。

### 构建流程

```
                    ┌─────────────────┐
                    │   make build    │
                    └────────┬────────┘
                             │
              ┌──────────────┴──────────────┐
              ▼                             ▼
    ┌─────────────────┐          ┌─────────────────┐
    │  build-guest    │          │  build-host     │
    │  cargo build    │          │  cargo build    │
    │  --target       │          │  (native)       │
    │  wasm32-wasip2  │          │                 │
    │  --release      │          │                 │
    └────────┬────────┘          └────────┬────────┘
             │                            │
             ▼                            ▼
    guest_tool.wasm              mini-agent-wasm
    (WASM 字节码)                 (原生二进制)
```

**两步编译**：
1. Guest 编译到 `wasm32-wasip2` 目标 → 产出 `.wasm` 文件
2. Host 编译到本机目标 → 产出原生可执行文件，运行时加载 `.wasm`

**C 类比**：这就像编译一个 `.so` 共享库（guest），然后主程序（host）用 `dlopen` 加载它。区别是 `.wasm` 跑在沙箱里，`.so` 共享地址空间。

### Makefile 解读

```makefile
GUEST_DIR     := guest
HOST_DIR      := host
GUEST_WASM    := $(GUEST_DIR)/target/wasm32-wasip2/release/guest_tool.wasm
HOST_BIN      := $(HOST_DIR)/target/release/mini-agent-wasm
```

关键路径：
- Guest 的编译产物在 `guest/target/wasm32-wasip2/release/guest_tool.wasm`
- Host 的编译产物在 `host/target/release/mini-agent-wasm`

```makefile
run: build
	cd $(HOST_DIR) && cargo run --release -- ../$(GUEST_WASM)
```

Host 通过命令行参数接收 `.wasm` 文件路径。`--` 后面的参数传给程序本身（不是传给 cargo）。

```makefile
check-deps:
	@echo -n "  wasm32-wasip2:  " && \
	  (rustup target list --installed | grep -q wasm32-wasip2 && \
	   echo "✅ installed" || \
	   echo "❌ NOT FOUND — run: rustup target add wasm32-wasip2")
```

编译 WASM 需要安装 `wasm32-wasip2` 编译目标。这是 Rust 的交叉编译——在 x86/ARM 机器上编译出 WASM 字节码。

---

## 3. WIT 接口定义

文件：`wit/tool.wit`（66 行，整个项目最重要的"合同"）

### 什么是 WIT？

**WIT（WebAssembly Interface Types）** 是 WASM 组件模型的接口描述语言，类似于：
- C 的 `.h` 头文件
- Erlang 的 `-callback` 声明
- gRPC 的 `.proto` 文件
- COM/CORBA 的 IDL

它定义了 **guest 和 host 之间能互相调用什么函数、传什么数据**。

### 包声明

```wit
package demo:sandbox@0.1.0;
```

WIT 的包名格式是 `namespace:package@version`。这里是 `demo` 命名空间下的 `sandbox` 包，版本 `0.1.0`。

**C 类比**：相当于 C++ 的 `namespace demo::sandbox`。

### Host 接口 — 宿主提供给 Guest 的能力

```wit
interface host {
    enum log-level {
        info,
        warn,
        error,
    }

    log: func(level: log-level, message: string);
    now-millis: func() -> u64;
}
```

这定义了 **Guest 能调用的全部函数**。只有两个：
1. `log` — 输出日志（Guest 没有 stdout/stderr）
2. `now-millis` — 获取当前时间（Guest 没有系统时钟）

**这就是沙箱的核心思想**：Guest 的"世界"里只有这两个函数。没有 `open()`、没有 `socket()`、没有 `getenv()`。Host 完全控制 Guest 能做什么。

**C 类比**：想象你在写一个嵌入式系统的 HAL（硬件抽象层）。Guest 就像运行在 MCU 上的固件，只能通过 HAL 接口访问硬件：

```c
// 这是 Guest 能看到的全部 "系统调用"
void hal_log(LogLevel level, const char* message);
uint64_t hal_now_millis(void);
// 没有 open()、read()、write()、socket()...
```

### Tool 接口 — Guest 必须实现的函数

```wit
interface tool {
    record request {
        params: string,
    }

    record response {
        output: option<string>,
        error: option<string>,
    }

    execute: func(req: request) -> response;
    schema: func() -> string;
    description: func() -> string;
}
```

- `record` — 类似 C 的 `struct`，定义数据结构
- `option<string>` — 可选值，类似 Rust 的 `Option<String>` 或 C 的 `char*`（可以为 NULL）
- `execute` — 执行工具，接收 JSON 参数，返回 JSON 结果或错误
- `schema` — 返回参数的 JSON Schema（告诉 LLM 这个工具接受什么参数）
- `description` — 返回工具描述

**Erlang 类比**：
```erlang
-callback execute(Request :: #{params := binary()}) ->
    #{output => binary(), error => binary()}.
-callback schema() -> binary().
-callback description() -> binary().
```

### World 定义 — 把 import 和 export 组合起来

```wit
world sandboxed-tool {
    import host;       // Guest can call these (provided by host)
    export tool;       // Guest must implement these (called by host)
}
```

`world` 是 WIT 的顶层概念，定义了一个完整的"世界"：
- `import host` — Guest **导入**（可以调用）host 接口
- `export tool` — Guest **导出**（必须实现）tool 接口

**C 类比**：
```c
// Guest 的 "链接规范"
// 这些函数由 Host 提供（Guest 调用）：
extern void host_log(LogLevel level, const char* message);
extern uint64_t host_now_millis(void);

// 这些函数由 Guest 实现（Host 调用）：
Response tool_execute(Request req);
const char* tool_schema(void);
const char* tool_description(void);
```

### WIT 类型映射

| WIT 类型 | Rust 类型 | C 类型 |
|----------|----------|--------|
| `string` | `String` | `char*` + length |
| `u64` | `u64` | `uint64_t` |
| `option<string>` | `Option<String>` | `char*` (nullable) |
| `enum` | Rust `enum` | C `enum` |
| `record` | Rust `struct` | C `struct` |

---

## 4. Guest 端

文件：`guest/src/lib.rs`（149 行）

### Cargo.toml 关键配置

```toml
[lib]
crate-type = ["cdylib"]
```

`cdylib` 表示编译为 **C 动态库**。对于 WASM 目标，这会生成一个 `.wasm` 文件而不是 `.so`。

```toml
[dependencies]
wit-bindgen = "=0.36"
```

`wit-bindgen` 是 Guest 端的代码生成工具，从 WIT 文件生成 Rust 绑定。`=0.36` 表示精确版本锁定——WIT 工具链版本敏感，不同版本生成的代码可能不兼容。

```toml
[profile.release]
opt-level = "s"     # 优化体积（不是速度）
lto = true          # 链接时优化（Link-Time Optimization）
strip = true        # 去掉调试符号
codegen-units = 1   # 单编译单元（更好的优化，更慢的编译）
```

WASM 文件需要通过网络传输或嵌入到应用中，所以体积很重要。这些选项把 `.wasm` 文件压到最小。

**C 类比**：相当于 `gcc -Os -flto -s -fwhole-program`。

### wit_bindgen::generate! — 代码生成宏

```rust
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../wit/tool.wit",
});
```

这个宏在 **编译期** 读取 `tool.wit` 文件，生成以下 Rust 代码（你看不到，但它们存在于编译器的中间表示中）：

```rust
// 自动生成的（概念性展示，实际代码更复杂）

// Guest 可以调用的 Host 函数
mod demo {
    pub mod sandbox {
        pub mod host {
            pub enum LogLevel { Info, Warn, Error }
            pub fn log(level: LogLevel, message: &str) { /* FFI 调用 */ }
            pub fn now_millis() -> u64 { /* FFI 调用 */ }
        }
    }
}

// Guest 必须实现的 trait
mod exports {
    pub mod demo {
        pub mod sandbox {
            pub mod tool {
                pub struct Request { pub params: String }
                pub struct Response { pub output: Option<String>, pub error: Option<String> }
                pub trait Guest {
                    fn execute(req: Request) -> Response;
                    fn schema() -> String;
                    fn description() -> String;
                }
            }
        }
    }
}
```

**C 类比**：这就像 protobuf 的 `protoc` 代码生成，但在编译期自动完成，不需要单独的代码生成步骤。

### 导入生成的类型

```rust
use exports::demo::sandbox::tool::{Guest, Request, Response};
use demo::sandbox::host;
```

- `Guest` — 我们要实现的 trait
- `Request`, `Response` — WIT 中定义的 record 类型
- `host` — 包含 `log()` 和 `now_millis()` 函数的模块

### CalculatorInput / CalculatorOutput

```rust
#[derive(Deserialize)]
struct CalculatorInput {
    operation: String,
    a: f64,
    b: f64,
}

#[derive(Serialize)]
struct CalculatorOutput {
    expression: String,
    result: f64,
    timestamp_ms: u64,
}
```

这些是 **Guest 内部的类型**，用于 JSON 序列化/反序列化。WIT 接口传递的是 JSON 字符串（`params: string`），Guest 内部再解析成结构体。

注意 `CalculatorOutput` 多了一个 `timestamp_ms` 字段——这个时间戳来自 `host::now_millis()`，展示了 Guest 如何通过 Host 提供的能力获取外部信息。

### Guest trait 实现

```rust
struct CalculatorTool;

impl Guest for CalculatorTool {
    fn execute(req: Request) -> Response {
```

注意这里的函数签名：**没有 `&self`**。WIT 生成的 trait 方法是 **关联函数（associated function）**，不是方法。这是因为 WASM 组件模型中，Guest 没有"实例"的概念——它就是一组函数。

**C 类比**：这些就是普通的 C 函数，不是 C++ 的成员函数：
```c
Response tool_execute(Request req);  // 不是 this->execute(req)
```

### 调用 Host 函数

```rust
host::log(
    host::LogLevel::Info,
    &format!("Calculating: {} {} {}", input.a, input.operation, input.b),
);
```

当 Guest 调用 `host::log()` 时，执行流程是：

```
Guest (WASM) → WASM VM 边界 → Host (native Rust) → StoreData::log()
```

这个调用会 **跨越 WASM 沙箱边界**。WASM VM 会：
1. 从 Guest 的线性内存中读取字符串参数
2. 转换为 Host 的 Rust `String`
3. 调用 Host 实现的 `log()` 函数
4. 返回结果（如果有的话）

**C 类比**：类似系统调用 `syscall(SYS_write, ...)`——从用户态陷入内核态，内核处理后返回。WASM 的 Host 函数就是 Guest 的"系统调用"。

### 获取时间

```rust
let now = host::now_millis();
```

Guest 没有系统时钟，不能调用 `std::time::SystemTime::now()`（那需要系统调用）。它只能通过 Host 提供的 `now_millis()` 获取时间。Host 可以返回真实时间，也可以返回假时间（用于测试）。

### export! 宏

```rust
export!(CalculatorTool);
```

这个宏把 `CalculatorTool` 注册为 WIT `tool` 接口的实现。它生成 WASM 导出函数，让 Host 可以调用。

**C 类比**：相当于在共享库中用 `__attribute__((visibility("default")))` 导出函数符号。

### Guest 的限制 — 它不能做什么

```rust
// ❌ 以下代码在 Guest 中会编译失败或运行时 panic：
// std::fs::read_to_string("secret.txt")     // 没有文件系统
// reqwest::get("http://evil.com")            // 没有网络
// std::env::var("API_KEY")                   // 没有环境变量
// std::thread::spawn(|| { ... })             // 没有线程
// loop {}                                     // 会被 fuel 限制终止
```

这不是靠"约定"或"代码审查"来保证的——是 **WASM VM 在硬件级别强制执行的**。Guest 的代码运行在一个隔离的线性内存空间中，没有任何系统调用接口。

---

## 5. Host 端全景

文件：`host/src/main.rs`（1192 行）

这个文件分为 6 个 PART，用大块注释分隔：

```
PART 1: LLM Types & Provider Trait        (行 ~60-160)   — 与 mini-agent-loop 相同
PART 2: Tool Trait & WASM Sandbox          (行 ~160-420)  — ★ 核心新增
PART 3: Agentic Loop                       (行 ~420-540)  — 与 mini-agent-loop 相同
PART 4: Mock LLM Provider                  (行 ~540-660)  — 新增：测试用
PART 5: Real OpenAI-Compatible Provider    (行 ~660-960)  — 与 mini-agent-loop 相同
PART 6: Main Entry Point                   (行 ~960-1192) — 略有不同
```

### Host Cargo.toml 关键依赖

```toml
# WASM sandbox runtime
wasmtime = { version = "28", features = ["component-model"] }
wasmtime-wasi = "28"
```

- `wasmtime` — Mozilla 开发的 WASM 运行时，类似 JVM 之于 Java。`component-model` 特性开启 WASM 组件模型支持。
- `wasmtime-wasi` — WASI（WebAssembly System Interface）的 Wasmtime 实现。提供基础的系统接口（但我们会限制 Guest 能用哪些）。

```toml
dirs = "5"
```

`dirs` 库用于获取用户主目录（`~/`）。这个项目从 `~/HAI_WOA.json` 读取配置，而不是当前目录。

---

## 6. PART 1: LLM 层

这部分与 mini-agent-loop 完全相同，定义了：
- `Role` 枚举（System/User/Assistant/Tool）
- `ChatMessage` 结构体及其构造方法
- `ToolCall`、`ToolDefinition`、`FinishReason`、`LlmOutput`、`LlmResponse`
- `LlmProvider` trait

**已在 mini-agent-loop 教程中详细讲解，这里不再重复。**

唯一值得注意的是：这些类型被直接定义在 `main.rs` 中，而不是拆分到独立模块。这是因为教学项目追求"一个文件读完"的体验。

---

## 7. PART 2: WASM 沙箱基础设施

**这是整个项目最重要的新增部分。** 从这里开始，每一行都值得仔细看。

### Tool Trait（与 mini-agent-loop 相同）

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String>;
}
```

这个 trait 没有变。Agent Loop 不关心工具是原生的还是 WASM 的——它只调用 `execute()`。这就是 **抽象的力量**。

### ToolRegistry（与 mini-agent-loop 相同）

```rust
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}
```

也没有变。注册的时候，mini-agent-loop 注册 `Box::new(CalculatorTool)`，mini-agent-wasm 注册 `Box::new(WasmTool { ... })`。对 Registry 来说，它们都是 `Box<dyn Tool>`。

### Step 1: wasmtime::component::bindgen! — Host 端代码生成

```rust
wasmtime::component::bindgen!({
    path: "../wit/tool.wit",
    world: "sandboxed-tool",
    async: false,
    with: {},
});
```

这个宏和 Guest 端的 `wit_bindgen::generate!` 是 **镜像关系**：

| | Guest 端 | Host 端 |
|---|---------|---------|
| 宏 | `wit_bindgen::generate!` | `wasmtime::component::bindgen!` |
| 生成什么 | 调用 Host 函数的桩代码 | 调用 Guest 函数的桩代码 |
| `import host` | 生成 `host::log()` 等可调用函数 | 生成 `Host` trait（我们要实现） |
| `export tool` | 生成 `Guest` trait（Guest 要实现） | 生成 `SandboxedTool::instantiate()` 等调用方法 |

**C 类比**：Guest 端生成的是 `extern` 声明（调用外部函数），Host 端生成的是函数指针表（提供给 Guest 调用）。

`async: false` — WASM 执行是同步的（WASM 没有 async 概念）。我们后面会用 `spawn_blocking` 把它包装成异步。

生成的关键类型：
- `demo::sandbox::host::Host` — Host trait，我们要实现 `log()` 和 `now_millis()`
- `demo::sandbox::host::LogLevel` — 日志级别枚举
- `SandboxedTool` — 可以实例化 Guest 并调用其导出函数
- `exports::demo::sandbox::tool::Request` / `Response` — WIT 中定义的数据类型

### Step 2: StoreData — 每次执行的状态

```rust
struct StoreData {
    wasi: WasiCtx,
    table: ResourceTable,
    logs: Vec<(String, String)>,
}
```

Wasmtime 的 `Store` 是 **WASM 实例的状态容器**。每个 Store 包含：
- WASM 实例的线性内存
- 全局变量
- 函数表
- **以及我们自定义的数据**（`StoreData`）

`StoreData` 包含：
- `wasi: WasiCtx` — WASI 上下文（提供基础系统接口的配置）
- `table: ResourceTable` — WASI 资源表（管理文件描述符等资源）
- `logs: Vec<(String, String)>` — 收集 Guest 的日志消息

**C 类比**：`Store` 类似一个虚拟机的"进程控制块（PCB）"。每次工具执行创建一个新的 PCB，执行完就销毁。

```rust
impl WasiView for StoreData {
    fn ctx(&mut self) -> &mut WasiCtx { &mut self.wasi }
    fn table(&mut self) -> &mut ResourceTable { &mut self.table }
}
```

`WasiView` trait 让 Wasmtime 知道如何从 `StoreData` 中获取 WASI 上下文。这是 Wasmtime 的要求——你的 Store 数据类型必须实现这个 trait。

### Step 3: Host Trait 实现 — 沙箱的核心

```rust
impl demo::sandbox::host::Host for StoreData {
    fn log(&mut self, level: demo::sandbox::host::LogLevel, message: String) {
        let level_str = match level {
            demo::sandbox::host::LogLevel::Info => "INFO",
            demo::sandbox::host::LogLevel::Warn => "WARN",
            demo::sandbox::host::LogLevel::Error => "ERROR",
        };
        println!("    📋 [WASM LOG] [{level_str}] {message}");
        self.logs.push((level_str.to_string(), message));
    }

    fn now_millis(&mut self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}
```

**这是整个沙箱最关键的代码。** 当 Guest 调用 `host::log("INFO", "hello")` 时，执行流程是：

```
Guest WASM 代码
  → WASM VM 捕获 import 调用
  → 查找 Linker 中注册的函数
  → 调用 StoreData::log(&mut self, level, message)
  → 打印到 stdout + 存入 self.logs
  → 返回给 Guest
```

**Host 完全控制这些函数的行为**：
- `log()` 可以写文件、发网络、或者直接丢弃
- `now_millis()` 可以返回假时间（用于确定性测试）
- 在 IronClaw 的完整版本中，`http_request()` 会检查 URL 白名单

**Erlang 类比**：这就像 Erlang 的 NIF（Native Implemented Function）的反向版本——不是 Erlang 调用 C，而是 WASM 调用 Host。Host 就是 WASM 的"BIF（Built-in Function）"。

### Step 4: WasmToolEngine — 共享引擎

```rust
struct WasmToolEngine {
    engine: Engine,
    component: wasmtime::component::Component,
    tool_name: String,
    tool_description: String,
    tool_schema: serde_json::Value,
    fuel_limit: u64,
}
```

这个结构体缓存了 **昂贵的一次性操作** 的结果：
- `engine` — Wasmtime 引擎（包含 JIT 编译器配置）
- `component` — 编译后的 WASM 组件（从 `.wasm` 字节码编译成本机代码）
- 工具元数据（name, description, schema）
- `fuel_limit` — 每次执行的燃料上限

**C 类比**：`Engine` 类似 JVM 实例，`Component` 类似编译后的 `.class` 文件。创建它们很慢，但可以复用。

### WasmToolEngine::new() — 初始化

```rust
fn new(wasm_path: &str, tool_name: &str, fuel_limit: u64) -> Result<Self, String> {
    // 1. 创建引擎
    let mut config = Config::new();
    config.wasm_component_model(true);  // 开启组件模型
    config.consume_fuel(true);          // 开启燃料计量
    let engine = Engine::new(&config)
        .map_err(|e| format!("Failed to create WASM engine: {}", e))?;
```

**燃料（Fuel）机制**：Wasmtime 的 fuel 是一种 **计算量限制**。每条 WASM 指令消耗一定量的 fuel。当 fuel 耗尽时，执行被强制终止。这防止了：
- 无限循环（`while(1) {}`）
- 指数级递归
- 任何形式的 DoS 攻击

**C 类比**：类似 `setrlimit(RLIMIT_CPU, ...)` 限制 CPU 时间，但粒度更细（指令级别而不是秒级别）。

```rust
    // 2. 加载并编译 WASM
    let wasm_bytes = std::fs::read(wasm_path)
        .map_err(|e| format!("Failed to read WASM file '{}': {}", wasm_path, e))?;
    let component = wasmtime::component::Component::new(&engine, &wasm_bytes)
        .map_err(|e| format!("Failed to compile WASM component: {}", e))?;
```

`Component::new()` 做了两件事：
1. 验证 WASM 字节码的合法性（类型检查、内存安全检查）
2. JIT 编译成本机代码（x86/ARM 机器码）

编译后的代码可以反复使用，不需要每次执行都重新编译。

```rust
    // 3. 设置 Linker（函数注册表）
    let mut linker: Linker<StoreData> = Linker::new(&engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker)
        .map_err(|e| format!("Failed to add WASI to linker: {}", e))?;
    demo::sandbox::host::add_to_linker(&mut linker, |state| state)
        .map_err(|e| format!("Failed to add host functions to linker: {}", e))?;
```

`Linker` 是 **函数注册表**，告诉 WASM VM "当 Guest 调用某个 import 函数时，应该调用哪个 Host 函数"。

- `wasmtime_wasi::add_to_linker_sync()` — 注册 WASI 基础函数（但我们的 WasiCtx 配置很严格，大部分功能被禁用）
- `demo::sandbox::host::add_to_linker()` — 注册我们自定义的 `log()` 和 `now_millis()`
- `|state| state` — 闭包，告诉 Linker 如何从 Store 数据中获取 Host trait 的实现者。这里 `StoreData` 本身就实现了 `Host` trait，所以直接返回自身。

**C 类比**：`Linker` 类似 `dlsym` 的反向操作——不是查找符号，而是注册符号。相当于构建一个函数指针表：

```c
struct ImportTable {
    void (*log)(LogLevel, const char*);
    uint64_t (*now_millis)(void);
};
ImportTable imports = {
    .log = host_log_impl,
    .now_millis = host_now_millis_impl,
};
```

```rust
    // 4. 创建临时 Store，实例化 Guest，读取元数据
    let mut store = Store::new(&engine, StoreData { ... });
    store.set_fuel(fuel_limit).map_err(...)?;

    let instance = SandboxedTool::instantiate(&mut store, &component, &linker)
        .map_err(|e| format!("Failed to instantiate WASM tool: {}", e))?;

    let description = instance.demo_sandbox_tool().call_description(&mut store)
        .map_err(|e| format!("Failed to call description(): {}", e))?;
    let schema_str = instance.demo_sandbox_tool().call_schema(&mut store)
        .map_err(|e| format!("Failed to call schema(): {}", e))?;
```

`SandboxedTool::instantiate()` 做了什么：
1. 在 Store 中分配 WASM 线性内存
2. 执行 WASM 的 `_start` 或 `_initialize` 函数（如果有）
3. 解析 Guest 的导出函数表
4. 返回一个可以调用 Guest 函数的句柄

`instance.demo_sandbox_tool()` 返回 Guest 导出的 `tool` 接口的代理对象。通过它可以调用 `call_description()`、`call_schema()`、`call_execute()` 等。

**注意**：这个临时 Store 只用于读取元数据，读完就丢弃。真正执行工具时会创建新的 Store。

### WasmToolEngine::execute_in_sandbox() — 每次执行

```rust
fn execute_in_sandbox(&self, params: &str) -> Result<String, String> {
    // Fresh linker for this execution
    let mut linker: Linker<StoreData> = Linker::new(&self.engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker).map_err(...)?;
    demo::sandbox::host::add_to_linker(&mut linker, |state| state).map_err(...)?;

    // Fresh store — complete isolation from previous executions
    let mut store = Store::new(
        &self.engine,
        StoreData {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            logs: Vec::new(),
        },
    );
    store.set_fuel(self.fuel_limit).map_err(...)?;
```

**每次执行都创建全新的 Store**。这意味着：
- Guest 的内存是全新的（没有上次执行的残留数据）
- Fuel 预算是满的
- 日志列表是空的

**这就是"Fresh instance per execution"模式**——完全消除了状态泄漏的可能性。

**Erlang 类比**：这就像每次调用都 `spawn` 一个新进程。进程结束后，所有状态自动清理。

**C 类比**：这就像每次调用都 `fork` 一个子进程执行，执行完 `_exit`。子进程的内存修改不会影响父进程。

```rust
    // Instantiate the guest in the sandbox
    let instance = SandboxedTool::instantiate(&mut store, &self.component, &linker)
        .map_err(|e| format!("Instantiation error: {}", e))?;

    // Call the guest's execute() function
    let request = exports::demo::sandbox::tool::Request {
        params: params.to_string(),
    };
    let response = instance.demo_sandbox_tool().call_execute(&mut store, &request)
        .map_err(|e| format!("WASM execution error: {}", e))?;
```

`call_execute()` 是关键调用——它跨越 WASM 边界，进入 Guest 代码执行。在执行过程中：
- 每条 WASM 指令消耗 fuel
- Guest 调用 `host::log()` 时，控制权暂时回到 Host
- 如果 fuel 耗尽，Wasmtime 会返回一个 trap 错误

```rust
    // Report fuel consumption
    let remaining = store.get_fuel().unwrap_or(0);
    let consumed = self.fuel_limit - remaining;
    println!("    ⛽ Fuel consumed: {} / {} units", consumed, self.fuel_limit);
```

执行完后，检查消耗了多少 fuel。这对于监控和调优很有用。

```rust
    // Check response
    if let Some(error) = response.error {
        return Err(format!("Tool error: {}", error));
    }
    response.output.ok_or_else(|| "Tool returned no output".to_string())
```

WIT 的 `Response` 有两个 `option` 字段：`output` 和 `error`。这里先检查 error，再取 output。

### Step 5: WasmTool — 桥接 Tool Trait 和 WASM 沙箱

```rust
struct WasmTool {
    engine: Arc<WasmToolEngine>,
}
```

`Arc<WasmToolEngine>` — 原子引用计数的共享指针。

**为什么用 `Arc` 而不是 `Box`？** 因为 `execute()` 方法需要在 `spawn_blocking` 中使用 engine，而 `spawn_blocking` 会把闭包发送到另一个线程。`Arc` 允许多个线程安全地共享同一个 engine。

**C++ 类比**：`Arc` = `std::shared_ptr`（但用原子操作保证线程安全）。

```rust
#[async_trait]
impl Tool for WasmTool {
    // name(), description(), parameters_schema() 直接委托给 engine

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let params_str = serde_json::to_string(&params)
            .map_err(|e| format!("Failed to serialize params: {}", e))?;

        // Execute in WASM sandbox (synchronous — WASM execution is sync)
        // We use spawn_blocking to avoid blocking the async runtime
        let engine = self.engine.clone();   // Arc::clone，只增加引用计数
        let result = tokio::task::spawn_blocking(move || {
            engine.execute_in_sandbox(&params_str)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))??;
```

**这里有一个重要的设计决策**：WASM 执行是同步的（阻塞当前线程），但 Agent Loop 是异步的。如果直接在 async 上下文中执行 WASM，会阻塞 tokio 的工作线程，影响其他异步任务。

`tokio::task::spawn_blocking` 把同步操作放到 **专门的阻塞线程池** 中执行，不会阻塞 async runtime。

**Erlang 类比**：这就像用 `dirty_nif` 而不是普通 NIF——长时间运行的操作不应该阻塞 scheduler。

注意 `??` — 两个问号。这是因为 `spawn_blocking` 返回 `Result<Result<String, String>, JoinError>`，两层 Result 需要两次 `?` 解包：
1. 第一个 `?` 处理 `JoinError`（线程 panic 等）
2. 第二个 `?` 处理 `execute_in_sandbox` 的错误

```rust
        // Parse the JSON output from the guest
        let value: serde_json::Value = serde_json::from_str(&result)
            .map_err(|e| format!("Failed to parse tool output: {}", e))?;

        Ok(ToolOutput { result: value })
    }
}
```

Guest 返回的是 JSON 字符串，这里解析成 `serde_json::Value`，包装成 `ToolOutput` 返回。

---

## 8. PART 3: Agentic Loop

与 mini-agent-loop 完全相同。核心循环：

```rust
for iteration in 1..=config.max_iterations {
    let response = llm.chat(messages, &tool_defs).await?;

    match response.result {
        LlmOutput::Text(text) => {
            return Ok(LoopOutcome::Response(text));  // 直接回答
        }
        LlmOutput::ToolCalls { tool_calls, content } => {
            // 执行工具 → 结果加入 messages → 继续循环
        }
    }
}
```

**关键点**：Agentic Loop 调用 `execute_tool_with_safety()`，后者调用 `tool.execute()`。它完全不知道底层是原生 Rust 还是 WASM 沙箱。这就是 **多态（polymorphism）** 的价值。

---

## 9. PART 4: Mock LLM Provider

这是 mini-agent-wasm 新增的一个测试工具：

```rust
struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        _tools: &[ToolDefinition],
    ) -> Result<LlmResponse, String> {
```

MockLlmProvider 不调用任何 API，而是用简单的规则模拟 LLM 的行为：
1. 如果用户输入包含数学关键词（+、-、*、/、calculate 等），返回 `ToolCalls`
2. 如果最后一条消息是 Tool 结果，返回包含结果的文本
3. 否则返回一段介绍文字

**为什么需要 Mock？**
- 不需要 API Key 就能测试 WASM 沙箱功能
- 测试确定性（真实 LLM 的响应不确定）
- 离线开发

### parse_math_intent — 简单的意图解析

```rust
fn parse_math_intent(input: &str) -> (&str, f64, f64) {
    let numbers: Vec<f64> = input
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();

    let a = numbers.first().copied().unwrap_or(0.0);
    let b = numbers.get(1).copied().unwrap_or(0.0);
```

这段代码从用户输入中提取数字：
1. `split(|c| ...)` — 按非数字字符分割字符串
2. `filter_map(|s| s.parse::<f64>().ok())` — 尝试把每段解析为浮点数，失败的跳过
3. `collect()` — 收集成 Vec

`.first().copied()` — `first()` 返回 `Option<&f64>`，`.copied()` 把 `&f64` 拷贝为 `f64`。对于 `Copy` 类型（如 `f64`），`.copied()` 等价于 `.map(|x| *x)`。

---

## 10. PART 5: OpenAI 兼容 Provider

与 mini-agent-loop 基本相同，有两个小区别：

### 配置文件位置

```rust
impl HaiConfig {
    fn load() -> Result<Self, String> {
        let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
        let path = home.join("HAI_WOA.json");
```

mini-agent-loop 从 **当前目录** 读取 `./HAI_WOA.json`，mini-agent-wasm 从 **用户主目录** 读取 `~/HAI_WOA.json`。这是因为 mini-agent-wasm 的工作目录是 `host/`，配置文件不在那里。

### list_models — 列出可用模型

```rust
fn list_models(&self) -> Vec<(&str, &str)> {
    let mut models: Vec<_> = self.models.iter()
        .map(|(k, v)| (k.as_str(), v.id.as_str()))
        .collect();
    models.sort_by_key(|(k, _)| *k);
    models
}
```

新增的辅助方法，用于 `/models` 命令显示所有可用的模型快捷方式。

---

## 11. PART 6: Main 入口

### Step 1: 加载 WASM 工具

```rust
let wasm_path = std::env::args().nth(1).unwrap_or_else(|| {
    "../guest/target/wasm32-wasip2/release/guest_tool.wasm".to_string()
});
```

从命令行参数获取 `.wasm` 文件路径，如果没有提供就用默认路径。

`std::env::args()` 返回命令行参数的迭代器。`.nth(1)` 获取第二个参数（第一个是程序名）。

```rust
let wasm_engine = match WasmToolEngine::new(&wasm_path, "calculator", 1_000_000) {
    Ok(engine) => { ... }
    Err(e) => { ... std::process::exit(1); }
};
```

`1_000_000` 是 fuel 上限。Rust 允许在数字中使用 `_` 作为分隔符（类似 Python 的 `1_000_000`），提高可读性。

### Step 2: 设置 LLM Provider（带 fallback）

```rust
let hai_config = match HaiConfig::load() {
    Ok(c) => { ... Some(c) }
    Err(e) => {
        println!("⚠️  Failed to load ~/HAI_WOA.json: {}", e);
        println!("   Falling back to MockLlmProvider (no real API calls)");
        None
    }
};
```

与 mini-agent-loop 不同，这里 **不会在配置加载失败时退出**，而是 fallback 到 MockLlmProvider。这让你不需要 API Key 也能测试 WASM 沙箱。

### Step 3: 注册 WASM 工具

```rust
// Instead of: registry.register(Box::new(CalculatorTool))  // native Rust
// We do:      registry.register(Box::new(WasmTool { ... })) // WASM sandbox
let mut registry = ToolRegistry::new();
registry.register(Box::new(WasmTool {
    engine: Arc::new(wasm_engine),
}));
```

**这就是 mini-agent-loop 和 mini-agent-wasm 的核心区别**——只有这一行不同。Agent Loop、LLM Provider、Tool Registry 都不需要改。

### Step 4 & 5: 系统提示 + 主循环

与 mini-agent-loop 基本相同，但增加了更多斜杠命令：

| 命令 | 功能 |
|------|------|
| `/models` | 列出所有可用模型 |
| `/model <name>` | 切换模型 |
| `/mock` | 切换到 Mock LLM（不调 API） |
| `/real` | 切换回真实 LLM |
| `quit` / `exit` | 退出 |

---

## 12. 数据流全景图

### 完整的一次工具调用

```mermaid
sequenceDiagram
    participant User as 用户 (stdin)
    participant Main as main.rs
    participant Loop as Agentic Loop
    participant LLM as LLM API / Mock
    participant Tool as WasmTool
    participant Engine as WasmToolEngine
    participant VM as Wasmtime VM
    participant Guest as guest_tool.wasm

    User->>Main: "What is 42 + 58?"
    Main->>Loop: run_agentic_loop(messages)

    Loop->>LLM: llm.chat(messages, tool_defs)
    LLM-->>Loop: ToolCalls [{calculator, {add, 42, 58}}]

    Loop->>Tool: execute({operation: "add", a: 42, b: 58})
    Note over Tool: spawn_blocking (避免阻塞 async runtime)
    Tool->>Engine: execute_in_sandbox(params_json)

    Note over Engine: 创建 Fresh Store + Linker
    Engine->>VM: SandboxedTool::instantiate()
    Engine->>VM: call_execute(request)

    VM->>Guest: execute({params: "..."})
    Guest->>VM: host::log(INFO, "Calculating: 42 add 58")
    VM->>Engine: StoreData::log() → println!
    Guest->>VM: host::now_millis()
    VM->>Engine: StoreData::now_millis() → SystemTime::now()
    Guest-->>VM: Response {output: "{...}", error: None}
    VM-->>Engine: response

    Note over Engine: 报告 fuel 消耗
    Engine-->>Tool: Ok(json_string)
    Tool-->>Loop: Ok(ToolOutput)

    Loop->>Loop: messages.push(tool_result)
    Loop->>LLM: llm.chat(messages, tool_defs)
    LLM-->>Loop: Text("42 + 58 = 100")
    Loop-->>Main: LoopOutcome::Response("42 + 58 = 100")
    Main->>User: "🤖 Agent: 42 + 58 = 100"
```

### messages 数组的演变

```
第 1 轮 LLM 调用前：
  [0] System: "You are a helpful assistant with access to a calculator tool..."
  [1] User: "What is 42 + 58?"

第 1 轮 LLM 返回 ToolCalls 后：
  [0] System: "You are a helpful assistant..."
  [1] User: "What is 42 + 58?"
  [2] Assistant: {tool_calls: [{id: "call_001", name: "calculator", args: {...}}]}
  [3] Tool: {tool_call_id: "call_001", content: "<tool_output>{\"expression\":\"42 + 58 = 100\",\"result\":100,\"timestamp_ms\":1711234567890}</tool_output>"}

第 2 轮 LLM 返回 Text：
  → "42 + 58 = 100"
  → 循环结束
```

---

## 13. WASM 沙箱安全模型深度解析

### 三层防御

```
┌─────────────────────────────────────────────────────┐
│ Layer 1: WASM 类型系统 + 内存隔离                     │
│   • Guest 只能访问自己的线性内存                       │
│   • 不能读写 Host 的内存                              │
│   • 所有内存访问都有边界检查                           │
├─────────────────────────────────────────────────────┤
│ Layer 2: Capability-based Security（能力安全）         │
│   • Guest 只能调用 Host 显式提供的函数                 │
│   • 没有 import 的函数 = 不存在                       │
│   • WIT 定义了完整的能力边界                          │
├─────────────────────────────────────────────────────┤
│ Layer 3: Fuel Metering（燃料计量）                     │
│   • 每条指令消耗 fuel                                 │
│   • Fuel 耗尽 → 执行终止                              │
│   • 防止无限循环和 DoS                                │
└─────────────────────────────────────────────────────┘
```

### Layer 1: 内存隔离

WASM 的线性内存是一个 **连续的字节数组**，Guest 只能在这个数组内读写。所有内存访问都会被 WASM VM 检查边界。

```
Host 进程内存空间：
┌──────────────────────────────────────────────────┐
│  Host 代码 + 数据                                 │
│  ┌─────────────────────────────────────────────┐ │
│  │  API Key: "sk-abc123..."                    │ │  ← Guest 看不到
│  │  用户数据: [...]                             │ │  ← Guest 看不到
│  └─────────────────────────────────────────────┘ │
│                                                  │
│  WASM 线性内存（Guest 的"世界"）：                  │
│  ┌─────────────────────────────────────────────┐ │
│  │  0x0000: Guest 的栈                         │ │
│  │  0x1000: Guest 的堆                         │ │
│  │  0x2000: Guest 的全局变量                    │ │
│  │  ...                                        │ │
│  │  0xFFFF: 线性内存边界                        │ │  ← 越界 = trap
│  └─────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────┘
```

**C 类比**：这就像 `mmap` 了一块固定大小的内存，Guest 只能在这块内存里操作。任何越界访问都会被 WASM VM 捕获（不是 segfault，而是可控的 trap）。

### Layer 2: 能力安全

传统的安全模型是 **黑名单**（禁止做什么）。WASM 的安全模型是 **白名单**（只允许做什么）。

```
传统沙箱（seccomp）：
  默认：所有系统调用都可用
  安全策略：禁止 open(), socket(), exec(), ...
  风险：可能遗漏某个危险的系统调用

WASM 沙箱：
  默认：什么都不能做
  安全策略：只提供 log() 和 now_millis()
  风险：几乎为零（除非 Host 函数本身有 bug）
```

### Layer 3: Fuel 计量

```rust
store.set_fuel(1_000_000)?;  // 给 100 万单位的 fuel

// Guest 执行过程中：
// add i32 → 消耗 1 fuel
// call func → 消耗 1 fuel
// loop iteration → 消耗 1 fuel
// ...
// 当 fuel 降到 0 → trap!

let remaining = store.get_fuel()?;
let consumed = 1_000_000 - remaining;
```

Fuel 的消耗量大致和执行的指令数成正比。100 万 fuel 大约够执行几十万条 WASM 指令——对于一个计算器来说绰绰有余，但对于无限循环来说远远不够。

### Fresh Instance 模式

```
执行 1:  Store₁ [fuel=1M, memory=clean, logs=[]]
         → execute(42+58) → result=100
         → Store₁ 被丢弃

执行 2:  Store₂ [fuel=1M, memory=clean, logs=[]]
         → execute(100/3) → result=33.33
         → Store₂ 被丢弃

执行 3:  Store₃ [fuel=1M, memory=clean, logs=[]]
         → execute(0/0) → error="Division by zero"
         → Store₃ 被丢弃
```

每次执行都是全新的 Store，没有任何状态残留。即使 Guest 在内存中写入了恶意数据，下次执行也看不到。

---

## 14. LLDB 调试指南

### 编译 Debug 版本

```bash
make debug
# 或者分别编译：
cd guest && cargo build --target wasm32-wasip2
cd host && cargo build
```

### 启动 LLDB

```bash
cd host
lldb target/debug/mini-agent-wasm -- ../guest/target/wasm32-wasip2/debug/guest_tool.wasm
```

### 关键断点

```lldb
# WASM 引擎初始化
b mini_agent_wasm::WasmToolEngine::new

# WASM 沙箱执行（最重要的断点）
b mini_agent_wasm::WasmToolEngine::execute_in_sandbox

# Host 函数实现（Guest 调用 host::log 时会命中）
b "<mini_agent_wasm::StoreData as mini_agent_wasm::demo::sandbox::host::Host>::log"
b "<mini_agent_wasm::StoreData as mini_agent_wasm::demo::sandbox::host::Host>::now_millis"

# WasmTool 的 async execute（spawn_blocking 之前）
b "<mini_agent_wasm::WasmTool as mini_agent_wasm::Tool>::execute"

# Agentic Loop
b mini_agent_wasm::run_agentic_loop

# 工具执行管线
b mini_agent_wasm::execute_tool_with_safety
```

### 调试 WASM 沙箱执行

```lldb
# 在 execute_in_sandbox 设断点
b mini_agent_wasm::WasmToolEngine::execute_in_sandbox
run

# 命中后，单步执行到 call_execute
n    # next（不进入函数）
n
n    # 直到 call_execute 那一行

# 查看 fuel 状态
p store.get_fuel()

# 查看 request 参数
p request
p params

# 进入 call_execute（会进入 Wasmtime 内部）
s    # step into

# 查看 Guest 的日志
p store.data().logs
```

### 调试 Host 函数调用

当 Guest 调用 `host::log()` 时，你可以在 Host 端的实现处设断点：

```lldb
b "<mini_agent_wasm::StoreData as mini_agent_wasm::demo::sandbox::host::Host>::log"
run

# 命中后查看参数
p level
p message

# 查看调用栈（会看到 Wasmtime 的内部帧）
bt
```

调用栈会类似：

```
frame #0: StoreData::log(...)
frame #1: wasmtime::component::...  (Wasmtime 内部)
frame #2: <JIT compiled WASM code>  (Guest 的 WASM 代码)
frame #3: wasmtime::component::...  (call_execute 的入口)
frame #4: WasmToolEngine::execute_in_sandbox(...)
```

### 调试 spawn_blocking

由于 `WasmTool::execute` 使用了 `spawn_blocking`，WASM 执行实际上在 tokio 的阻塞线程池中运行。要调试它：

```lldb
# 查看所有线程
thread list

# 找到 tokio-runtime-worker 或 blocking-* 线程
# 通常是 "tokio-runtime-worker" 或 "blocking-0"
thread select 3    # 选择对应的线程
bt                 # 查看调用栈
```

### 调试 WASM fuel 耗尽

如果 Guest 消耗了过多 fuel，Wasmtime 会返回一个 trap。要调试这种情况：

```lldb
# 在 fuel 检查处设断点
b mini_agent_wasm::WasmToolEngine::execute_in_sandbox
run

# 执行到 call_execute 之后
n

# 检查是否有错误
p response
# 如果 fuel 耗尽，response 会是 Err(...)

# 查看剩余 fuel
p store.get_fuel()
```

---

## 15. 与 mini-agent-loop 的对比总结

### 代码量对比

| | mini-agent-loop | mini-agent-wasm |
|---|---|---|
| 总行数 | ~600 行（6 个文件） | ~1340 行（3 个源文件 + 1 个 WIT） |
| LLM 层 | ~350 行 | ~350 行（相同） |
| Tool 层 | ~100 行 | ~350 行（+250 行 WASM 基础设施） |
| Agent Loop | ~150 行 | ~120 行（相同逻辑，略简化） |
| 新增 | — | WIT 66 行 + Guest 149 行 + Mock LLM 120 行 |

### 架构对比

```
mini-agent-loop:
  main.rs → agent.rs → tools/calculator.rs (直接调用)
                     → llm/openai.rs (HTTP)

mini-agent-wasm:
  host/main.rs → Agentic Loop → WasmTool → WasmToolEngine → Wasmtime VM
                              → OpenAI Provider (HTTP)     ↕
                                                    guest/lib.rs (WASM)
                                                           ↕
                                                    wit/tool.wit (合同)
```

### 安全性对比

| 威胁 | mini-agent-loop | mini-agent-wasm |
|------|----------------|-----------------|
| 工具读取 API Key | ✅ 可以（同一进程） | ❌ 不可能（内存隔离） |
| 工具访问文件系统 | ✅ 可以 | ❌ 不可能（无系统调用） |
| 工具发起网络请求 | ✅ 可以 | ❌ 不可能（无网络接口） |
| 工具无限循环 | ⚠️ 靠 timeout | ❌ fuel 耗尽自动终止 |
| 工具状态泄漏 | ⚠️ 可能 | ❌ Fresh Store 每次执行 |

### 性能对比

| | mini-agent-loop | mini-agent-wasm |
|---|---|---|
| 工具调用开销 | ~0（直接函数调用） | ~1ms（Store 创建 + 实例化） |
| 首次加载 | 无 | ~100ms（WASM 编译） |
| 内存开销 | 无额外开销 | 每次执行 ~几 KB（线性内存） |

对于 AI Agent 来说，这些开销完全可以忽略——LLM API 调用通常需要 1-10 秒，工具执行的 1ms 开销微不足道。

### 什么时候用哪个？

- **mini-agent-loop**：学习 Agentic Loop 的核心概念，快速原型开发
- **mini-agent-wasm**：需要运行不受信任的工具代码，生产环境的安全要求

### 我的理解

这个项目展示了一个优雅的架构演进：

1. **抽象先行**：mini-agent-loop 定义了 `Tool` trait，但只有原生实现
2. **实现替换**：mini-agent-wasm 用 `WasmTool` 替换了 `CalculatorTool`，但 Agent Loop 完全不变
3. **关注点分离**：WIT 定义合同，Guest 实现工具逻辑，Host 控制安全策略

这就是 **开闭原则（OCP）** 的完美体现：对扩展开放（新增 WASM 执行方式），对修改关闭（Agent Loop 不需要改一行代码）。

从 C 程序员的角度看，WASM 沙箱就是一个 **用户态的进程隔离**——不需要内核支持，不需要容器，不需要虚拟机。它在单进程内实现了类似 `fork` + `seccomp` 的隔离效果，但更轻量、更可移植。

从 Erlang 程序员的角度看，WASM 沙箱提供了 Erlang 进程隔离的 **子集**——内存隔离和计算量限制有了，但没有 Erlang 的抢占式调度和 "let it crash" 哲学。不过对于工具执行这个场景，这已经足够了。
