# WIT ↔ Host ↔ Guest 联动映射详解

> 本文档解释 WIT 文件中的每个定义是如何在 Host 端和 Guest 端"落地"的，
> 以及 `demo_sandbox_tool`、`SandboxedTool`、`exports::demo::sandbox::tool` 等命名的由来。
>
> 核心原则：**一份 WIT 合约，两端镜像对称生成代码**。

---

## 目录

- [命名转换总规则](#命名转换总规则)
- [符号映射总表](#符号映射总表)
- [详解 1：`demo_sandbox_tool()` 的由来](#详解-1demo_sandbox_tool-的由来)
- [详解 2：`SandboxedTool` 的由来](#详解-2sandboxedtool-的由来)
- [详解 3：模块路径 `demo::sandbox::host` 的由来](#详解-3模块路径-demosandboxhost-的由来)
- [详解 4：`exports::` 前缀的含义](#详解-4exports-前缀的含义)
- [详解 5：`add_to_linker` 的由来](#详解-5add_to_linker-的由来)
- [详解 6：`call_` 前缀的由来](#详解-6call_-前缀的由来)
- [详解 7：类型名称的转换](#详解-7类型名称的转换)
- [完整联动图：一次 execute 调用的全链路](#完整联动图一次-execute-调用的全链路)

---

## 命名转换总规则

WIT 统一使用 **kebab-case**（短横线命名法），生成 Rust 代码时会自动转换：

| 转换场景 | WIT 命名 | Rust 命名 | 规则 |
|---------|---------|----------|------|
| 类型名 | `log-level` | `LogLevel` | kebab-case → PascalCase |
| 函数名 | `now-millis` | `now_millis` | kebab-case → snake_case |
| 模块路径 | `demo:sandbox` | `demo::sandbox` | 冒号 → 双冒号 |
| 方法名 | `demo:sandbox/tool` | `demo_sandbox_tool()` | 冒号和斜杠 → 下划线 |
| world 名 | `sandboxed-tool` | `SandboxedTool` | kebab-case → PascalCase |

---

## 符号映射总表

下表展示了 WIT 中的每个定义在 Host 和 Guest 两端分别生成了什么：

### 包和 World

| WIT 定义 | Host 端生成 | Guest 端生成 |
|---------|-----------|------------|
| `package demo:sandbox@0.1.0` | 模块前缀 `demo::sandbox::` | 模块前缀 `demo::sandbox::` |
| `world sandboxed-tool` | `SandboxedTool` 结构体（用于实例化） | 无直接类型（宏参数引用） |

### `interface host`（import 方向：Host 实现，Guest 调用）

| WIT 定义 | Host 端生成 | Guest 端生成 |
|---------|-----------|------------|
| `interface host` | `demo::sandbox::host::Host` trait（**必须实现**） | `demo::sandbox::host` 模块（**可以调用**） |
| `enum log-level` | `demo::sandbox::host::LogLevel` 枚举 | `demo::sandbox::host::LogLevel` 枚举 |
| `log: func(...)` | `fn log(&mut self, level, message)` trait 方法 | `host::log(level, message)` 可调用函数 |
| `now-millis: func() -> u64` | `fn now_millis(&mut self) -> u64` trait 方法 | `host::now_millis()` 可调用函数 |
| — | `demo::sandbox::host::add_to_linker()` 注册函数 | — |

### `interface tool`（export 方向：Guest 实现，Host 调用）

| WIT 定义 | Host 端生成 | Guest 端生成 |
|---------|-----------|------------|
| `interface tool` | `.demo_sandbox_tool()` 访问器方法 | `exports::demo::sandbox::tool::Guest` trait（**必须实现**） |
| `record request` | `exports::demo::sandbox::tool::Request` 结构体 | `exports::demo::sandbox::tool::Request` 结构体 |
| `record response` | `exports::demo::sandbox::tool::Response` 结构体 | `exports::demo::sandbox::tool::Response` 结构体 |
| `execute: func(...)` | `.call_execute(&mut store, &req)` 方法（**可以调用**） | `fn execute(req: Request) -> Response`（**必须实现**） |
| `schema: func() -> string` | `.call_schema(&mut store)` 方法 | `fn schema() -> String` |
| `description: func() -> string` | `.call_description(&mut store)` 方法 | `fn description() -> String` |

---

## 详解 1：`demo_sandbox_tool()` 的由来

### 在 Host 代码中的出现位置

```rust
// host/src/main.rs
let desc = tool.demo_sandbox_tool().call_description(&mut store)?;
let response = tool.demo_sandbox_tool().call_execute(&mut store, &request)?;
```

### 命名推导过程

```
WIT 包名:     demo:sandbox
WIT 接口名:   tool
完整限定名:   demo:sandbox/tool
                │    │       │
                ▼    ▼       ▼
Rust 方法名:  demo_sandbox_tool()
```

**规则**：`wasmtime::component::bindgen!` 宏将 WIT 的完整限定接口路径转换为方法名，把 `:` 和 `/` 全部替换为 `_`。

| WIT 路径 | 生成的方法名 |
|---------|------------|
| `demo:sandbox/tool` | `demo_sandbox_tool()` |
| `demo:sandbox/host` | `demo_sandbox_host()`（如果 host 也是 export 的话） |
| `wasi:http/handler` | `wasi_http_handler()` |
| `my-org:my-pkg/my-iface` | `my_org_my_pkg_my_iface()` |

### 它返回什么？

`demo_sandbox_tool()` 返回一个**接口访问器**（interface accessor），通过它可以调用 Guest 导出的函数：

```rust
tool.demo_sandbox_tool()              // 获取 tool 接口的访问器
    .call_execute(&mut store, &req)    // 调用 Guest 的 execute()
    .call_schema(&mut store)           // 调用 Guest 的 schema()
    .call_description(&mut store)      // 调用 Guest 的 description()
```

### 为什么不直接 `tool.call_execute()`？

因为一个 `world` 可以 export **多个** interface。如果 WIT 是这样的：

```wit
world my-tool {
    export tool;        // 工具接口
    export admin;       // 管理接口
    export metrics;     // 指标接口
}
```

那么 Host 端就需要通过不同的访问器来区分：

```rust
tool.demo_sandbox_tool().call_execute(...)     // 调用 tool 接口
tool.demo_sandbox_admin().call_reset(...)       // 调用 admin 接口
tool.demo_sandbox_metrics().call_collect(...)   // 调用 metrics 接口
```

所以 `demo_sandbox_tool()` 本质上是一个**命名空间选择器**。

---

## 详解 2：`SandboxedTool` 的由来

### 在 Host 代码中的出现位置

```rust
// host/src/main.rs
let tool = SandboxedTool::instantiate(&mut store, &component, &linker)?;
```

### 命名推导过程

```
WIT world 名:   sandboxed-tool
                    │
                    ▼  (kebab-case → PascalCase)
Rust 结构体名:  SandboxedTool
```

**规则**：`wasmtime::component::bindgen!` 宏将 `world` 的名称从 kebab-case 转换为 PascalCase，作为生成的顶层结构体名。

| WIT world 名 | 生成的 Rust 结构体 |
|-------------|------------------|
| `sandboxed-tool` | `SandboxedTool` |
| `my-agent` | `MyAgent` |
| `http-proxy` | `HttpProxy` |

### `SandboxedTool` 有什么方法？

```rust
// 由 bindgen! 自动生成
struct SandboxedTool { ... }

impl SandboxedTool {
    // 实例化：加载 WASM 组件，创建可调用的实例
    fn instantiate(
        store: &mut Store<StoreData>,
        component: &Component,
        linker: &Linker<StoreData>,
    ) -> Result<Self>;

    // 获取 tool 接口的访问器
    fn demo_sandbox_tool(&self) -> /* 接口访问器 */;
}
```

### 与 Guest 的 `CalculatorTool` 的关系

**没有直接关系！**

| | `SandboxedTool`（Host 端） | `CalculatorTool`（Guest 端） |
|---|---|---|
| 定义位置 | `bindgen!` 宏自动生成 | Guest 开发者手动定义 |
| 来源 | WIT 的 `world sandboxed-tool` | Guest 开发者自己起的名字 |
| 作用 | 加载和调用 WASM 组件 | 实现 `Guest` trait |
| 对方是否知道？ | Host 不知道 `CalculatorTool` | Guest 不知道 `SandboxedTool` |

---

## 详解 3：模块路径 `demo::sandbox::host` 的由来

### 在两端代码中的出现

```rust
// Guest 端 (lib.rs) — 调用 Host 函数
use demo::sandbox::host;
host::log(host::LogLevel::Info, "hello");

// Host 端 (main.rs) — 实现 Host trait
impl demo::sandbox::host::Host for StoreData { ... }
demo::sandbox::host::add_to_linker(&mut linker, |state| state)?;
```

### 命名推导过程

```
WIT 包名:       demo:sandbox
WIT 接口名:     host
                  │     │        │
                  ▼     ▼        ▼
Rust 模块路径:  demo::sandbox::host
```

**规则**：WIT 的 `package namespace:name` 中的 `:` 变成 Rust 的 `::`，接口名追加在后面。

| WIT | Rust 模块路径 |
|-----|-------------|
| `package demo:sandbox` + `interface host` | `demo::sandbox::host` |
| `package demo:sandbox` + `interface tool` | `demo::sandbox::tool` |
| `package wasi:http` + `interface handler` | `wasi::http::handler` |

### 同一个路径，两端含义不同

虽然路径相同，但在两端的含义完全相反：

```
demo::sandbox::host
        │
        ├── Guest 端：这是一个模块，里面有可以调用的函数
        │     host::log(...)          ← 调用
        │     host::now_millis()      ← 调用
        │
        └── Host 端：这是一个模块，里面有必须实现的 trait
              Host trait              ← 实现
              add_to_linker()         ← 注册
```

---

## 详解 4：`exports::` 前缀的含义

### 在两端代码中的出现

```rust
// Guest 端 (lib.rs)
use exports::demo::sandbox::tool::{Guest, Request, Response};

// Host 端 (main.rs)
let request = exports::demo::sandbox::tool::Request { ... };
```

### 为什么有 `exports::` 前缀？

因为 WIT 中同一个类型名可能同时出现在 `import` 和 `export` 中。为了避免命名冲突，代码生成器用 `exports::` 前缀来区分：

```
没有 exports:: 前缀  →  import 方向的类型（Host 提供给 Guest 的）
有 exports:: 前缀    →  export 方向的类型（Guest 提供给 Host 的）
```

在本项目中：

| 路径 | 方向 | 含义 |
|------|------|------|
| `demo::sandbox::host::LogLevel` | import | Host 提供的日志级别枚举 |
| `exports::demo::sandbox::tool::Request` | export | Guest 导出的请求类型 |
| `exports::demo::sandbox::tool::Response` | export | Guest 导出的响应类型 |
| `exports::demo::sandbox::tool::Guest` | export | Guest 必须实现的 trait |

### 类比理解

```
import（进口）= 从外面拿进来的东西  →  不加前缀
export（出口）= 往外面送出去的东西  →  加 exports:: 前缀
```

---

## 详解 5：`add_to_linker` 的由来

### 在 Host 代码中的出现

```rust
// host/src/main.rs
demo::sandbox::host::add_to_linker(&mut linker, |state| state)?;
```

### 这是什么？

`add_to_linker` 是 `wasmtime::component::bindgen!` 宏为每个 **import 接口**自动生成的函数。它的作用是把 Host 的实现注册到 Linker 中，让 Guest 能找到并调用。

### 命名规则

每个 `import` 接口都会生成一个 `add_to_linker` 函数，位于对应的模块路径下：

| WIT import | 生成的注册函数 |
|-----------|---------------|
| `import host` | `demo::sandbox::host::add_to_linker()` |
| 如果有 `import logger` | `demo::sandbox::logger::add_to_linker()` |

### 第二个参数 `|state| state` 的含义

```rust
demo::sandbox::host::add_to_linker(&mut linker, |state| state)?;
//                                               ^^^^^^^^^^^^^^
//                                               访问器闭包
```

这个闭包告诉框架："怎样从 Store 的数据中拿到实现了 `Host` trait 的对象"。

因为 `StoreData` 自身就实现了 `Host` trait，所以闭包直接原样返回（恒等函数）。

如果 Host trait 是由 `StoreData` 的某个字段实现的，闭包会是这样：

```rust
// 假设 StoreData 有一个 host_impl 字段
demo::sandbox::host::add_to_linker(&mut linker, |state| &mut state.host_impl)?;
```

---

## 详解 6：`call_` 前缀的由来

### 在 Host 代码中的出现

```rust
// host/src/main.rs
tool.demo_sandbox_tool().call_execute(&mut store, &request)?;
tool.demo_sandbox_tool().call_schema(&mut store)?;
tool.demo_sandbox_tool().call_description(&mut store)?;
```

### 为什么有 `call_` 前缀？

`wasmtime::component::bindgen!` 宏为 Host 端生成的**调用 Guest 函数的方法**统一加上 `call_` 前缀，以区分于普通的 Rust 方法：

| WIT 函数定义 | Host 端生成的调用方法 |
|-------------|-------------------|
| `execute: func(req: request) -> response` | `call_execute(&mut store, &req)` |
| `schema: func() -> string` | `call_schema(&mut store)` |
| `description: func() -> string` | `call_description(&mut store)` |

**规则**：`call_` + 函数名（kebab-case → snake_case）

### 为什么需要传 `&mut store`？

每次跨 WASM 边界调用都需要 `&mut store`，因为：
1. **Fuel 消耗** — 每条 WASM 指令都会扣减 Store 中的 fuel 计数器
2. **内存访问** — Guest 的线性内存存储在 Store 中
3. **Host 回调** — Guest 可能在执行过程中回调 Host 函数（如 `log`），需要访问 Store 中的状态

### 与 Guest 端的对比

Guest 端**没有** `call_` 前缀，因为 Guest 直接实现这些函数：

```rust
// Guest 端 — 实现（没有 call_ 前缀，没有 store 参数）
impl Guest for CalculatorTool {
    fn execute(req: Request) -> Response { ... }
    fn schema() -> String { ... }
    fn description() -> String { ... }
}

// Host 端 — 调用（有 call_ 前缀，有 store 参数）
tool.demo_sandbox_tool().call_execute(&mut store, &request)?;
tool.demo_sandbox_tool().call_schema(&mut store)?;
tool.demo_sandbox_tool().call_description(&mut store)?;
```

---

## 详解 7：类型名称的转换

### `record` → `struct`

```
WIT:   record request { params: string, }
Rust:  pub struct Request { pub params: String, }
```

| WIT record 名 | Rust struct 名 | 规则 |
|--------------|---------------|------|
| `request` | `Request` | kebab-case → PascalCase |
| `response` | `Response` | kebab-case → PascalCase |
| `http-request` | `HttpRequest` | kebab-case → PascalCase |

### `enum` → `enum`

```
WIT:   enum log-level { info, warn, error }
Rust:  pub enum LogLevel { Info, Warn, Error }
```

| WIT 枚举名 | Rust 枚举名 | 变体转换 |
|-----------|-----------|--------|
| `log-level` | `LogLevel` | `info` → `Info` |
| | | `warn` → `Warn` |
| | | `error` → `Error` |

### `option<T>` → `Option<T>`

```
WIT:   output: option<string>
Rust:  pub output: Option<String>
```

### WIT 内置类型 → Rust 类型

| WIT | Rust | 说明 |
|-----|------|------|
| `string` | `String` | UTF-8 字符串 |
| `u64` | `u64` | 64 位无符号整数 |
| `bool` | `bool` | 布尔值 |
| `f64` | `f64` | 64 位浮点数 |
| `option<T>` | `Option<T>` | 可选值 |
| `list<T>` | `Vec<T>` | 列表 |
| `result<T, E>` | `Result<T, E>` | 结果类型 |

---

## 完整联动图：一次 execute 调用的全链路

以 Host 调用 `execute({"operation": "add", "a": 1, "b": 2})` 为例，展示 WIT、Host、Guest 三端的完整联动：

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        WIT 合约 (tool.wit)                              │
│                                                                         │
│  package demo:sandbox@0.1.0;                                            │
│                                                                         │
│  interface tool {                                                        │
│      record request { params: string }                                  │
│      record response { output: option<string>, error: option<string> }  │
│      execute: func(req: request) -> response;                           │
│  }                                                                      │
│                                                                         │
│  interface host {                                                        │
│      log: func(level: log-level, message: string);                      │
│      now-millis: func() -> u64;                                         │
│  }                                                                      │
│                                                                         │
│  world sandboxed-tool {                                                  │
│      import host;                                                        │
│      export tool;                                                        │
│  }                                                                      │
└──────────────────────────────────┬──────────────────────────────────────┘
                                   │
                  ┌────────────────┴────────────────┐
                  ▼                                 ▼
┌──────────────────────────┐      ┌──────────────────────────────┐
│  Host 端 (main.rs)        │      │  Guest 端 (lib.rs)            │
│                          │      │                              │
│  宏:                      │      │  宏:                          │
│  wasmtime::component::   │      │  wit_bindgen::generate!({    │
│    bindgen!({            │      │    world: "sandboxed-tool",  │
│      world:              │      │    path: "../wit/tool.wit",  │
│        "sandboxed-tool", │      │  });                         │
│      path:               │      │                              │
│        "../wit/tool.wit",│      │                              │
│    });                   │      │                              │
│                          │      │                              │
│  生成:                    │      │  生成:                        │
│  ┌────────────────────┐  │      │  ┌──────────────────────────┐│
│  │ SandboxedTool      │  │      │  │ exports::demo::sandbox:: ││
│  │   ::instantiate()  │  │      │  │   tool::Guest trait      ││
│  │   .demo_sandbox_   │  │      │  │   tool::Request struct   ││
│  │     tool()         │  │      │  │   tool::Response struct  ││
│  │     .call_execute()│  │      │  │                          ││
│  │     .call_schema() │  │      │  │ demo::sandbox::host 模块  ││
│  │     .call_desc..() │  │      │  │   host::log()            ││
│  │                    │  │      │  │   host::now_millis()     ││
│  │ demo::sandbox::    │  │      │  └──────────────────────────┘│
│  │   host::Host trait │  │      │                              │
│  │   host::add_to_    │  │      │  用户代码:                    │
│  │     linker()       │  │      │  struct CalculatorTool;      │
│  └────────────────────┘  │      │  impl Guest for              │
│                          │      │    CalculatorTool { ... }    │
│  用户代码:                │      │  export!(CalculatorTool);    │
│  impl Host for           │      │                              │
│    StoreData { ... }     │      │                              │
└──────────┬───────────────┘      └──────────────┬───────────────┘
           │                                      │
           │         ┌──────────────┐              │
           │         │  .wasm 文件   │              │
           │         │  (二进制边界)  │              │
           └────────►│              │◄─────────────┘
                     └──────┬───────┘
                            │
                     调用流程如下
                            ▼
```

### 调用时序

```
Host                              WASM 边界                    Guest
  │                                    │                          │
  │ ① tool.demo_sandbox_tool()         │                          │
  │    .call_execute(&store, &req)     │                          │
  │───────────────────────────────────►│                          │
  │                                    │  ② CalculatorTool        │
  │                                    │     ::execute(req)       │
  │                                    │─────────────────────────►│
  │                                    │                          │
  │                                    │  ③ Guest 解析 JSON       │
  │                                    │     serde_json::         │
  │                                    │       from_str(params)   │
  │                                    │                          │
  │  ④ host::log() 回调                │                          │
  │◄───────────────────────────────────│◄─────────────────────────│
  │  StoreData::log() 执行             │                          │
  │  println!("[GUEST LOG]...")        │                          │
  │───────────────────────────────────►│─────────────────────────►│
  │                                    │                          │
  │                                    │  ⑤ 纯计算: 1 + 2 = 3    │
  │                                    │                          │
  │  ⑥ host::now_millis() 回调         │                          │
  │◄───────────────────────────────────│◄─────────────────────────│
  │  StoreData::now_millis() 执行      │                          │
  │  返回时间戳                         │                          │
  │───────────────────────────────────►│─────────────────────────►│
  │                                    │                          │
  │                                    │  ⑦ 构造 Response         │
  │  ⑧ 收到 Response                   │                          │
  │◄───────────────────────────────────│◄─────────────────────────│
  │                                    │                          │
  │  response.output =                 │                          │
  │    Some("{\"result\":3}")          │                          │
```

### 每一步的代码对应

| 步骤 | 代码位置 | 代码 |
|------|---------|------|
| ① Host 发起调用 | `main.rs` | `tool.demo_sandbox_tool().call_execute(&mut store, &request)?` |
| ② Guest 收到调用 | `lib.rs` | `fn execute(req: Request) -> Response {` |
| ③ 解析 JSON | `lib.rs` | `let input: CalculatorInput = serde_json::from_str(&req.params)?` |
| ④ Guest 回调 Host | `lib.rs` → `main.rs` | `host::log(...)` → `StoreData::log()` |
| ⑤ 纯计算 | `lib.rs` | `let (result, expression) = match input.operation.as_str() { ... }` |
| ⑥ Guest 回调 Host | `lib.rs` → `main.rs` | `host::now_millis()` → `StoreData::now_millis()` |
| ⑦ 构造响应 | `lib.rs` | `Response { output: Some(serde_json::to_string(&output).unwrap()), ... }` |
| ⑧ Host 收到响应 | `main.rs` | `let response = ...?;` |

---

## 附：如果改了 WIT，两端会怎么变？

假设我们在 WIT 中新增一个 `http-request` 函数：

```wit
interface host {
    // ... 原有的 log, now-millis ...

    record http-response {
        status: u16,
        body: string,
    }

    http-request: func(url: string, method: string) -> http-response;
}
```

那么两端需要同步修改：

| 端 | 需要做什么 |
|----|----------|
| **Host 端** | 在 `impl Host for StoreData` 中新增 `fn http_request(&mut self, url: String, method: String) -> HttpResponse { ... }` |
| **Guest 端** | 可以直接调用 `host::http_request("https://...", "GET")` |
| **WIT** | 已经改好了 |

这就是 WIT 作为"合约"的价值——**改一处定义，两端的编译器会自动告诉你哪里需要跟着改**。如果 Host 忘了实现新函数，编译会报错；如果 Guest 调用了不存在的函数，编译也会报错。
