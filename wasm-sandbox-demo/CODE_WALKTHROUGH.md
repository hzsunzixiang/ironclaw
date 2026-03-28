# WASM Sandbox Demo — 代码详解

> 本文档是对 `wasm-sandbox-demo` 项目的逐步代码解读，帮助理解 WASM 沙箱机制的核心原理和实现细节。

---

## 目录

- [项目概览](#项目概览)
- [项目结构](#项目结构)
- [核心概念：WASM 沙箱模型](#核心概念wasm-沙箱模型)
- [第一部分：WIT 接口定义（合约）](#第一部分wit-接口定义合约)
- [第二部分：Guest 端代码（沙箱内的插件）](#第二部分guest-端代码沙箱内的插件)
- [第三部分：Host 端代码（沙箱运行时）](#第三部分host-端代码沙箱运行时)
- [构建与运行](#构建与运行)
- [调试指南](#调试指南)
- [深入理解：Host 如何调用 Guest](#深入理解host-如何调用-guest)
- [关键机制总结](#关键机制总结)

---

## 项目概览

这是一个 **WASM（WebAssembly）沙箱演示项目**，展示了如何使用 Wasmtime 运行时将不受信任的代码隔离在安全沙箱中执行。

核心思想：**Guest（插件）只能做 Host（宿主）明确允许的事情**。没有文件系统访问、没有网络请求、没有环境变量读取——只有 Host 通过 WIT 接口暴露的能力。

---

## 项目结构

```
wasm-sandbox-demo/
├── wit/
│   └── tool.wit          # WIT 接口定义 — Host 和 Guest 之间的"合约"
├── guest/
│   ├── Cargo.toml        # Guest 依赖（wit-bindgen, serde, serde_json）
│   └── src/
│       └── lib.rs        # Guest 实现 — 编译为 .wasm，在沙箱中运行
├── host/
│   ├── Cargo.toml        # Host 依赖（wasmtime, wasmtime-wasi, serde_json）
│   └── src/
│       └── main.rs       # Host 实现 — 编译为原生二进制，加载并运行 .wasm
└── Makefile              # 构建脚本
```

**构建流程**：

```
guest/src/lib.rs  ──cargo build──→  guest_tool.wasm（WASM 字节码）
                                         │
host/src/main.rs  ──cargo build──→  wasm-host（原生二进制）
                                         │
                                    wasm-host 加载 guest_tool.wasm
                                    创建沙箱 → 执行 guest 函数
```

---

## 核心概念：WASM 沙箱模型

```
┌─────────────────────────────────────────────────────────┐
│                    Host（宿主进程）                       │
│                                                         │
│  ┌───────────────────────────────────────────────────┐  │
│  │              WASM 沙箱（虚拟机）                    │  │
│  │                                                   │  │
│  │   Guest 代码在这里运行                              │  │
│  │   ✅ 可以做纯计算（数学、字符串操作）                 │  │
│  │   ✅ 可以调用 Host 提供的函数（log, now_millis）     │  │
│  │   ❌ 不能访问文件系统                               │  │
│  │   ❌ 不能访问网络                                   │  │
│  │   ❌ 不能读取环境变量                               │  │
│  │   ❌ 不能访问 Host 内存                             │  │
│  │   ❌ 不能无限执行（Fuel 限制）                       │  │
│  │                                                   │  │
│  └──────────────────────┬────────────────────────────┘  │
│                         │ WIT 接口（唯一通道）            │
│                         ▼                               │
│  Host 实现的函数：                                       │
│    • log(level, message) — 收集日志                      │
│    • now_millis() — 返回当前时间                         │
│                                                         │
│  Host 调用 Guest 的函数：                                │
│    • description() — 获取工具描述                        │
│    • schema() — 获取参数 schema                         │
│    • execute(request) — 执行工具                        │
└─────────────────────────────────────────────────────────┘
```

---

## 第一部分：WIT 接口定义（合约）

**文件**：`wit/tool.wit`

WIT（WebAssembly Interface Types）是 Host 和 Guest 之间的**合约**。双方都读取同一个 WIT 文件来生成代码，确保接口一致。

### 包声明

```wit
package demo:sandbox@0.1.0;
```

定义了包名 `demo:sandbox`，版本 `0.1.0`。这个包名会影响后续生成的 Rust 模块路径（如 `demo::sandbox::host`）。

### Host 接口（Guest 可以调用的函数）

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

这定义了 Guest 能调用的**全部能力**——仅此两个函数，别无其他：

| 函数 | 作用 | 安全意义 |
|------|------|---------|
| `log` | 发送日志消息给 Host | Guest 无法直接 println，只能通过 Host 决定如何处理日志 |
| `now-millis` | 获取当前时间戳 | Guest 无法直接访问系统时钟，Host 甚至可以返回假时间 |

### Tool 接口（Guest 必须实现的函数）

```wit
interface tool {
    record request {
        params: string,    // JSON 编码的参数
    }

    record response {
        output: option<string>,  // 成功时的 JSON 输出
        error: option<string>,   // 失败时的错误信息
    }

    execute: func(req: request) -> response;
    schema: func() -> string;
    description: func() -> string;
}
```

Guest 必须导出这三个函数供 Host 调用：

| 函数 | 作用 |
|------|------|
| `description()` | 返回工具的人类可读描述 |
| `schema()` | 返回参数的 JSON Schema |
| `execute(req)` | 执行工具的核心逻辑 |

### World 定义

```wit
world sandboxed-tool {
    import host;       // Guest 可以调用这些（由 Host 提供）
    export tool;       // Guest 必须实现这些（由 Host 调用）
}
```

`world` 把 `import` 和 `export` 组合在一起，定义了完整的沙箱边界。

---

## 第二部分：Guest 端代码（沙箱内的插件）

**文件**：`guest/src/lib.rs`

Guest 代码编译为 `.wasm` 文件，在沙箱中运行。它实现了一个简单的**计算器工具**。

### 代码生成宏

```rust
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../wit/tool.wit",
});
```

`wit_bindgen::generate!` 是 **Guest 端**的代码生成宏。它读取 WIT 文件，自动生成：

- `exports::demo::sandbox::tool::Guest` trait（Guest 必须实现）
- `exports::demo::sandbox::tool::{Request, Response}` 类型
- `demo::sandbox::host` 模块（可以调用 Host 函数）

### 数据结构

```rust
#[derive(Deserialize)]
struct CalculatorInput {
    operation: String,  // "add", "sub", "mul", "div"
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

- `CalculatorInput` — 从 JSON 反序列化输入参数
- `CalculatorOutput` — 序列化为 JSON 作为输出

### Guest trait 实现

```rust
struct CalculatorTool;

impl Guest for CalculatorTool {
    fn execute(req: Request) -> Response { ... }
    fn schema() -> String { ... }
    fn description() -> String { ... }
}
```

这是 Guest 的核心——实现 WIT 中定义的 `tool` 接口。

#### `execute()` 函数详解

```rust
fn execute(req: Request) -> Response {
    // 1. 解析 JSON 输入
    let input: CalculatorInput = match serde_json::from_str(&req.params) {
        Ok(i) => i,
        Err(e) => return Response { output: None, error: Some(format!("Invalid input: {}", e)) },
    };

    // 2. 调用 Host 函数记录日志（这是 Guest 唯一能产生"副作用"的方式）
    host::log(host::LogLevel::Info, &format!("Calculating: {} {} {}", input.a, input.operation, input.b));

    // 3. 纯计算
    let (result, expression) = match input.operation.as_str() {
        "add" => (input.a + input.b, format!("{} + {} = {}", ...)),
        "sub" => (input.a - input.b, format!("{} - {} = {}", ...)),
        "mul" => (input.a * input.b, format!("{} × {} = {}", ...)),
        "div" => {
            if input.b == 0.0 {
                host::log(host::LogLevel::Error, "Division by zero!");
                return Response { output: None, error: Some("Division by zero".to_string()) };
            }
            (input.a / input.b, format!("{} ÷ {} = {}", ...))
        }
        _ => return Response { output: None, error: Some(format!("Unknown operation: '{}'", ...)) },
    };

    // 4. 调用 Host 函数获取时间（Guest 无法直接访问系统时钟）
    let now = host::now_millis();

    // 5. 构造并返回响应
    Response {
        output: Some(serde_json::to_string(&output).unwrap()),
        error: None,
    }
}
```

关键点：
- Guest **不能** `println!`，只能通过 `host::log()` 输出信息
- Guest **不能**直接获取时间，只能通过 `host::now_millis()`
- 所有"副作用"都必须经过 Host 提供的接口

### 导出注册

```rust
export!(CalculatorTool);
```

这个宏将 `CalculatorTool` 注册为 WIT `tool` 接口的实现，使 Host 能够调用它。

---

## 第三部分：Host 端代码（沙箱运行时）

**文件**：`host/src/main.rs`

Host 是原生 Rust 程序，负责创建 WASM 沙箱、加载 Guest、提供 Host 函数、调用 Guest 函数。

### Step 1：代码生成宏（Host 端）

```rust
wasmtime::component::bindgen!({
    path: "../wit/tool.wit",
    world: "sandboxed-tool",
    async: false,
    with: {},
});
```

注意：Guest 用 `wit_bindgen::generate!`，Host 用 `wasmtime::component::bindgen!`。**两者读取同一个 WIT 文件**，但生成的代码角色相反：

| | Guest 端 | Host 端 |
|---|---|---|
| 宏 | `wit_bindgen::generate!` | `wasmtime::component::bindgen!` |
| `import host` | 生成可调用的函数 | 生成需要实现的 trait |
| `export tool` | 生成需要实现的 trait | 生成可调用的方法 |

Host 端的 `bindgen!` 自动生成了：
- `demo::sandbox::host::Host` trait（Host 必须实现）
- `demo::sandbox::host::add_to_linker()` 函数（注册 Host 函数）
- `SandboxedTool` 结构体（用于实例化和调用 Guest）
- `SandboxedTool::demo_sandbox_tool()` 方法（获取 `tool` 接口的访问器）

其中 `demo_sandbox_tool()` 的命名规则是：**包名 + 接口名**，冒号和横线替换为下划线：

```
demo:sandbox/tool  →  demo_sandbox_tool()
```

### Step 2：Store 数据（运行时状态）

```rust
struct StoreData {
    wasi: WasiCtx,
    table: ResourceTable,
    logs: Vec<(String, String)>,  // (level, message)
}
```

`Store` 是 Wasmtime 的核心概念，持有每次执行的**全部运行时状态**：

| 字段 | 作用 |
|------|------|
| `wasi` | WASI 上下文（wasm32-wasip2 目标需要） |
| `table` | 资源表（WASI 资源管理） |
| `logs` | Host 收集的 Guest 日志 |

每次 WASM 执行都会创建一个新的 Store，确保**执行间完全隔离**。

### Step 3：实现 Host trait

```rust
impl demo::sandbox::host::Host for StoreData {
    fn log(&mut self, level: demo::sandbox::host::LogLevel, message: String) {
        let level_str = match level { ... };
        println!("  📋 [GUEST LOG] [{level_str}] {message}");
        self.logs.push((level_str.to_string(), message));
    }

    fn now_millis(&mut self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        now
    }
}
```

**这是沙箱的核心**。当 Guest 调用 `host::log()` 或 `host::now_millis()` 时，执行会跨越 WASM 边界到达这里。Host 拥有**完全控制权**：

- `log()` — Host 选择打印并收集日志。也可以选择丢弃、限流、或发送到日志服务
- `now_millis()` — Host 选择返回真实时间。也可以返回假时间用于确定性测试

Guest **无法绕过**这些实现——这是由 WASM 虚拟机在硬件级别强制保证的。

### Step 4：主函数流程

#### 4.1 创建 Engine（WASM 虚拟机）

```rust
let mut config = Config::new();
config.wasm_component_model(true);  // 启用 Component Model
config.consume_fuel(true);           // 启用 Fuel 计量（CPU 限制）
let engine = Engine::new(&config)?;
```

Engine 是 Wasmtime 的 WASM 虚拟机实例。`consume_fuel(true)` 开启燃料机制，用于限制 Guest 的执行量。

#### 4.2 加载并编译 WASM 组件

```rust
let wasm_bytes = std::fs::read(&wasm_path)?;
let component = wasmtime::component::Component::new(&engine, &wasm_bytes)?;
```

读取 `.wasm` 文件并编译为原生机器码。类比：把磁盘上的 `.exe` 加载到内存。

#### 4.3 设置 Linker（函数注册表）

```rust
let mut linker: Linker<StoreData> = Linker::new(&engine);
wasmtime_wasi::add_to_linker_sync(&mut linker)?;
demo::sandbox::host::add_to_linker(&mut linker, |state| state)?;
```

Linker 就像一个**电话簿**——Guest 查找 `host::log`，Linker 告诉它对应的 Host 实现在哪里。

关于 `|state| state` 闭包：这是一个**访问器**，告诉框架"怎样从 Store 的数据中拿到实现了 Host trait 的对象"。因为 `StoreData` 自身就实现了 `Host` trait，所以闭包直接原样返回。

用其他语言类比：

```python
# Python — 直接传对象
linker.register_host(store_data)

# 如果 Python 也需要访问器模式：
linker.register_host(lambda state: state)  # ← 等价于 |state| state
```

```java
// Java — 直接传对象
linker.registerHost(new StoreData());

// 如果 Java 也需要访问器模式：
linker.registerHost(storeData -> storeData);  // ← 等价于 |state| state
```

注意：`state` 不是提前定义的变量，而是**闭包的形参**，由 Wasmtime 框架在 Guest 调用 Host 函数时自动传入，传入的值就是 Store 中存储的 `StoreData` 实例。

#### 4.4 创建 Store 并设置 Fuel

```rust
let mut store = Store::new(
    &engine,
    StoreData {
        wasi: WasiCtxBuilder::new().build(),
        table: ResourceTable::new(),
        logs: Vec::new(),
    },
);
store.set_fuel(1_000_000)?;
```

创建 Store（运行时状态容器），并设置 **100 万单位**的燃料上限。

**Fuel 机制**类似以太坊的 Gas：

```
WASM 指令 1: 消耗 1 fuel → 剩余 999,999
WASM 指令 2: 消耗 1 fuel → 剩余 999,998
...
指令 1,000,000: 消耗 1 fuel → 剩余 0
指令 1,000,001: 💥 Trap! "out of fuel" — 执行被强制终止
```

这防止了死循环和 CPU 耗尽攻击。

#### 4.5 实例化 Guest

```rust
let tool = SandboxedTool::instantiate(&mut store, &component, &linker)?;
```

把前面准备好的所有东西**组装在一起**，创建可调用的 Guest 实例。类比操作系统启动程序：

| 参数 | 类比 |
|------|------|
| `store` | 进程的内存空间 |
| `component` | 磁盘上的可执行文件 |
| `linker` | 动态链接库的符号表 |
| `instantiate()` | 操作系统的 `exec()` 系统调用 |

#### 4.6 调用 Guest 函数

**调用 `description()` 和 `schema()`**：

```rust
let desc = tool.demo_sandbox_tool().call_description(&mut store)?;
let schema: serde_json::Value = serde_json::from_str(
    &tool.demo_sandbox_tool().call_schema(&mut store)?
)?;
```

- `tool.demo_sandbox_tool()` — 获取 WIT 中 `tool` 接口的访问器
- `.call_description(&mut store)` — 跨越 WASM 边界调用 Guest 的 `description()` 函数
- 每次调用都需要传 `&mut store`，因为 Store 持有 Guest 的内存、Fuel 计数器等状态

**调用 `execute()` 并处理结果**：

```rust
let test_cases = vec![
    r#"{"operation": "add", "a": 42, "b": 58}"#,    // 正常：加法
    r#"{"operation": "mul", "a": 3.14, "b": 2}"#,   // 正常：乘法
    r#"{"operation": "div", "a": 100, "b": 3}"#,    // 正常：除法
    r#"{"operation": "div", "a": 1, "b": 0}"#,      // 边界：除以零
    r#"{"operation": "sqrt", "a": 9, "b": 0}"#,     // 无效：未知操作
];
```

> 💡 `r#"..."#` 是 Rust 的 **Raw String Literal**（原始字符串），内部不需要转义双引号，特别适合写 JSON。等价于 Python 的单引号字符串 `'{"key": "value"}'` 或 Go 的反引号字符串。

```rust
for (i, params) in test_cases.iter().enumerate() {
    let request = exports::demo::sandbox::tool::Request {
        params: params.to_string(),
    };
    let response = tool.demo_sandbox_tool().call_execute(&mut store, &request)?;

    if let Some(output) = &response.output {
        // 成功：解析并美化输出 JSON
    }
    if let Some(error) = &response.error {
        // 失败：打印错误信息
    }
}
```

数据流向：

```
Host                              WASM 沙箱边界                    Guest
  │                                    │                              │
  │  Request { params: JSON }          │                              │
  │───────────────────────────────────→│─────────────────────────────→│
  │                                    │                              │ 解析 JSON
  │                                    │                              │ 调用 host::log()
  │  ← log("info", "Calculating...")   │←─────────────────────────────│
  │  收集到 self.logs                   │                              │
  │                                    │                              │ 调用 host::now_millis()
  │  ← 返回时间戳                       │←─────────────────────────────│
  │                                    │                              │ 执行计算
  │  Response { output, error }        │                              │
  │←───────────────────────────────────│←─────────────────────────────│
```

#### 4.7 查看 Fuel 消耗和日志

```rust
let remaining_fuel = store.get_fuel()?;
println!("⛽ Fuel remaining: {} / 1,000,000", remaining_fuel);
println!("   Consumed: {} units", 1_000_000 - remaining_fuel);

println!("📋 Total guest log entries collected by host: {}", store.data().logs.len());
```

- `store.get_fuel()` — 查询剩余燃料，计算消耗量
- `store.data().logs.len()` — 查看 Host 收集到的 Guest 日志总数

这体现了沙箱的**可观测性**：Host 完全掌控 Guest 的资源使用和输出。

---

## 构建与运行

### 前置条件

```bash
# 检查依赖
make check-deps

# 需要安装 wasm32-wasip2 目标
rustup target add wasm32-wasip2
```

### 构建

```bash
make build          # 构建 Guest（WASM）+ Host（原生）
make build-guest    # 仅构建 Guest
make build-host     # 仅构建 Host
```

### 运行

```bash
make run            # 构建并运行演示
```

### 其他命令

```bash
make inspect        # 查看 WASM 组件信息（需要 wasm-tools）
make size           # 查看 WASM 二进制大小
make clean          # 清理构建产物
```

---

## 调试指南

本项目支持多种调试方式，由简到深：

### 方式一：`println!` / `dbg!` 调试（最简单）

Host 端可以直接使用 `println!` 或 `dbg!` 宏：

```rust
// host/src/main.rs 中
dbg!(&response);
```

Guest 端**不能**直接 `println!`，只能通过 Host 提供的 `log` 函数输出：

```rust
// guest/src/lib.rs 中
host::log(host::LogLevel::Info, &format!("debug: input = {:?}", some_var));
```

### 方式二：Debug 构建 + `RUST_BACKTRACE`（查看错误堆栈）

```bash
# 构建 debug 版本
make debug

# 运行 debug 版本
make run-debug

# 启用完整堆栈追踪
RUST_BACKTRACE=1 make run-debug
RUST_BACKTRACE=full make run-debug    # 更详细
```

### 方式三：LLDB 命令行调试（断点调试）⭐

Host 是原生 Rust 程序，可以用标准调试器。Rust 使用 LLVM 后端，生成标准 DWARF 调试信息，因此 **lldb 调试体验与 C/C++ 完全一致**。

#### 启动调试器

```bash
cd host
lldb -- target/debug/wasm-host ../guest/target/wasm32-wasip2/debug/guest_tool.wasm
```

- `lldb` — macOS 默认调试器
- `--` — 分隔符，告诉 lldb 后面是程序及其参数
- `target/debug/wasm-host` — 要调试的可执行文件
- `../guest/...guest_tool.wasm` — 传给程序的命令行参数（Guest WASM 文件路径）

#### 常用调试命令

```
(lldb) b main                # 在 main 函数设断点（会匹配多个位置）
(lldb) b wasm_host::main     # 精确匹配 Rust 的 main 函数
(lldb) b src/main.rs:120     # 在指定文件行号设断点
(lldb) r                     # 运行程序（run）
(lldb) n                     # 单步执行（step over）
(lldb) s                     # 步入函数（step into）
(lldb) c                     # 继续运行到下一个断点（continue）
(lldb) p variable            # 打印变量值
(lldb) bt                    # 查看调用栈（backtrace）
(lldb) list                  # 显示当前位置附近的源码
(lldb) frame variable        # 显示当前栈帧中所有局部变量
(lldb) thread list           # 列出所有线程
(lldb) thread backtrace all  # 查看所有线程的调用栈
(lldb) process status        # 查看进程状态
(lldb) q                     # 退出调试器
```

#### 调试实战示例

```
(lldb) b main
Breakpoint 1: 14 locations        ← main 匹配了 14 个位置（含 wasmtime 内部）

(lldb) r
Process 55867 launched
Process 55867 stopped
* thread #1, stop reason = breakpoint 1.2
    frame #0: wasm-host`main       ← 第一次停在 C runtime 的 main（汇编层）

(lldb) c                           ← 继续运行
Process 55867 stopped
* thread #1, stop reason = breakpoint 1.1
    frame #0: wasm_host::main at main.rs:106:5
-> 106      println!("╔═══...═══╗");  ← 第二次停在 Rust 源码的 fn main() ✅
```

> 💡 **注意**：`b main` 第一次会停在 C runtime 入口（显示 ARM64 汇编），这是因为 Rust 的 `fn main()` 被包装在一层 C 的 `main` 函数中。输入 `c` 继续运行后才会到达 Rust 源码。更精确的方式是直接使用 `b wasm_host::main`。

#### 进程与线程观察

本程序是**单进程**运行，WASM Guest 不是独立进程，而是在同一进程内的沙箱中执行。但线程数会随程序执行阶段变化：

**程序启动时（2 个线程）**：

```
(lldb) thread list
* thread #1: ... name = 'main' ...           ← 主线程，执行你的 Rust 代码
  thread #2: ... mach_msg2_trap ...           ← macOS 系统消息线程
```

**加载编译 WASM 后（14 个线程）**：

```
(lldb) thread list
* thread #1: ... main.rs:156 ...              ← 主线程
  thread #2: ... mach_msg2_trap ...            ← 系统线程
  thread #3 ~ #14: ... __psynch_cvwait ...     ← 12 个 rayon 编译线程池
```

线程增长的原因：Wasmtime 引擎内部使用 **rayon 线程池**并行编译 WASM 字节码为原生机器码。编译完成后这些线程进入空闲等待状态（`__psynch_cvwait`），不参与后续的 WASM 执行。

```
程序启动：  2 个线程（主线程 + 系统线程）
    ↓
编译 WASM：14 个线程（+ 12 个 rayon 编译线程池）
    ↓
执行 Guest：仍然在主线程上同步运行，编译线程池空闲
```

> **关键结论**：你的业务代码始终是单线程的。WASM Guest 代码的执行在主线程上同步完成，Guest 调用 Host 函数（如 `log`、`now_millis`）也是在主线程上直接回调。多出来的线程都是 wasmtime 引擎的内部实现细节。

### 方式四：VS Code + CodeLLDB 插件（图形化调试）

安装 VS Code 的 `CodeLLDB` 插件，然后在 `host/.vscode/launch.json` 中添加：

```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "lldb",
            "request": "launch",
            "name": "Debug Host",
            "cargo": {
                "args": ["build", "--manifest-path", "${workspaceFolder}/host/Cargo.toml"]
            },
            "program": "${workspaceFolder}/host/target/debug/wasm-host",
            "args": ["${workspaceFolder}/guest/target/wasm32-wasip2/debug/guest_tool.wasm"],
            "cwd": "${workspaceFolder}/host",
            "env": { "RUST_BACKTRACE": "1" }
        }
    ]
}
```

在代码中点击行号左侧设置断点，按 F5 启动调试。

### 方式五：`WASMTIME_LOG` 环境变量（查看运行时内部日志）

```bash
# 查看 wasmtime 的 debug 日志
WASMTIME_LOG=debug make run-debug

# 只看特定模块
WASMTIME_LOG=wasmtime_wasi=debug make run-debug
```

### Rust 调试与 C/C++ 的对比

由于 Rust 和 C/C++ 共享同一套底层工具链（LLVM + DWARF），调试体验几乎完全一致：

| 方面 | C/C++ | Rust |
|------|-------|------|
| 调试器 | `lldb` / `gdb` | `lldb` / `gdb` ✅ 相同 |
| 编译产物 | 原生机器码 | 原生机器码 ✅ 相同 |
| 调试信息格式 | DWARF | DWARF ✅ 相同 |
| 断点/单步/查看变量 | `b` / `n` / `p` | `b` / `n` / `p` ✅ 相同 |

---

## 深入理解：Host 如何调用 Guest

### 为什么 `CalculatorTool` 在 Host 中看不到？

**因为 Host 根本不需要知道 `CalculatorTool` 的存在。** 这是 WASM 沙箱架构的核心设计——Host 和 Guest 之间通过 **WIT 接口（合约）** 通信，而不是直接引用对方的类型。

| | Guest 端 (`lib.rs`) | Host 端 (`main.rs`) |
|---|---|---|
| 知道 `CalculatorTool` 吗？ | ✅ 知道，自己定义的 | ❌ 完全不知道 |
| 知道 `CalculatorInput` 吗？ | ✅ 知道，自己定义的 | ❌ 完全不知道 |
| 知道 WIT 接口吗？ | ✅ `execute`, `schema`, `description` | ✅ `call_execute`, `call_schema`, `call_description` |

Host 只知道：**"有一个 WASM 组件，它导出了 `execute`、`schema`、`description` 三个函数"**。至于这些函数背后是 `CalculatorTool` 还是 `WeatherTool` 还是别的什么，Host 完全不关心。

这就像 **HTTP 客户端和服务端** 的关系：

| HTTP 类比 | WASM 对应 |
|-----------|-----------|
| URL + JSON Schema | WIT 接口定义 |
| HTTP 协议 | WASM Component Model |
| 服务端的 `class UserController` | Guest 的 `struct CalculatorTool` |
| 客户端看不到 `UserController` | Host 看不到 `CalculatorTool` |
| 客户端只知道 `POST /api/users` | Host 只知道 `call_execute()` |

**Host 加载的是编译后的 `.wasm` 二进制文件**，不是 Guest 的 Rust 源码。所有 Rust 类型信息（`CalculatorTool`、`CalculatorInput` 等）在编译成 `.wasm` 后就消失了，只剩下 WIT 合约定义的导出函数。这正是 WASM 的优势之一——**Guest 可以用任何语言编写**（Rust、Go、C、Python），Host 完全不需要知道。

### Host 调用 Guest 的完整入口流程

Host 调用 Guest 分为 **3 步**：

#### Step 1：加载 `.wasm` 文件并实例化

```rust
// 加载编译好的 .wasm 二进制文件（不是 Rust 源码！）
let wasm_bytes = std::fs::read(&wasm_path)?;
let component = wasmtime::component::Component::new(&engine, &wasm_bytes)?;

// 实例化 — 这是入口的起点
let tool = SandboxedTool::instantiate(&mut store, &component, &linker)?;
```

`SandboxedTool` 是 `wasmtime::component::bindgen!` 宏根据 WIT 文件自动生成的类型，**不是 Guest 的 `CalculatorTool`**。

#### Step 2：获取接口访问器

```rust
tool.demo_sandbox_tool()  // 返回 tool 接口的访问器
```

`demo_sandbox_tool()` 的命名来自 WIT 的包名 + 接口名：`demo:sandbox/tool` → `demo_sandbox_tool()`。

#### Step 3：调用导出函数

```rust
// 调用 description()
let desc = tool.demo_sandbox_tool().call_description(&mut store)?;

// 调用 schema()
let schema_str = tool.demo_sandbox_tool().call_schema(&mut store)?;

// 调用 execute()
let request = exports::demo::sandbox::tool::Request {
    params: r#"{"operation": "add", "a": 42, "b": 58}"#.to_string(),
};
let response = tool.demo_sandbox_tool().call_execute(&mut store, &request)?;
```

### 完整调用链

```
Host 端 (main.rs)                    WASM 边界                     Guest 端 (lib.rs)
─────────────────                    ─────────                     ────────────────
SandboxedTool::instantiate()
        │
        ▼
tool.demo_sandbox_tool()  ──→  .wasm 文件的导出表  ──→  export!(CalculatorTool)
        │                                                       │
        ▼                                                       ▼
call_execute(&store, &req)  ──→  跨 WASM 边界调用  ──→  CalculatorTool::execute()
                                                               │
                                                               ▼
                                                        impl Guest for CalculatorTool
                                                               │
                                                               ▼
                                                        fn execute(req) -> Response
```

关键点：
- `export!(CalculatorTool)` 在 Guest 编译时将 `CalculatorTool` 的方法注册到 `.wasm` 文件的导出表中
- Host 通过 `SandboxedTool`（自动生成的类型）访问这些导出，完全不知道背后是 `CalculatorTool`
- 整个过程类似于：**编译后的 `.wasm` 文件就是一个"黑盒"，Host 只能通过 WIT 定义的接口与之交互**

---

## 关键机制总结

### 1. 安全隔离

| 机制 | 说明 |
|------|------|
| **WASM 沙箱** | Guest 运行在虚拟机中，无法访问 Host 内存、文件系统、网络 |
| **WIT 接口** | Guest 只能调用 Host 明确暴露的函数，没有其他途径 |
| **Fuel 限制** | 防止死循环和 CPU 耗尽，类似以太坊 Gas 机制 |

### 2. Host 的完全控制权

Host 对 Guest 的每个"副作用"都有完全控制：

- **日志** — Host 可以收集、丢弃、限流、审计
- **时间** — Host 可以返回真实时间或假时间
- **执行量** — Host 通过 Fuel 控制 Guest 能执行多少指令

### 3. 代码生成对称性

```
                    WIT 文件（合约）
                   ╱              ╲
          Guest 端                  Host 端
    wit_bindgen::generate!    wasmtime::component::bindgen!
          │                          │
    生成 Host 调用函数          生成 Host trait（需实现）
    生成 Guest trait（需实现）   生成 Guest 调用方法
```

双方从同一个 WIT 文件生成代码，保证接口一致性。

### 4. 类比总结

| WASM 沙箱概念 | 操作系统类比 | 区块链类比 |
|--------------|-------------|-----------|
| Engine | 操作系统内核 | EVM |
| Component | 磁盘上的 .exe | 智能合约字节码 |
| Linker | 动态链接器 | 预编译合约注册表 |
| Store | 进程地址空间 | 合约存储 |
| Fuel | ulimit CPU 时间限制 | Gas |
| instantiate() | exec() 系统调用 | 部署合约 |
| call_execute() | 进程间通信 | 调用合约方法 |
