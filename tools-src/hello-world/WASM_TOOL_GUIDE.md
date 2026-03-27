# IronClaw WASM Tool 最简示例指南

本文档以一个 **hello-world** 工具为例，演示 IronClaw WASM Tool 的完整开发→构建→安装→使用流程。

---

## 0. 前置条件

```bash
# 安装 wasm32-wasip2 编译目标（一次性）
rustup target add wasm32-wasip2
```

> 不需要 `cargo-component`，直接用 `cargo build --target wasm32-wasip2` 即可。

---

## 1. 项目结构

示例代码位于 `tools-src/hello-world/`：

```
tools-src/hello-world/
├── Cargo.toml
├── hello-world-tool.capabilities.json   # 权限声明（命名约定：<dir>-tool.capabilities.json）
└── src/
    └── lib.rs                             # 工具实现
```

### 关键文件说明

| 文件 | 作用 |
|------|------|
| `Cargo.toml` | 声明 `crate-type = ["cdylib"]`，依赖 `wit-bindgen`、`serde`、`serde_json` |
| `src/lib.rs` | 实现 `Guest` trait 的 3 个方法：`execute`、`schema`、`description` |
| `*.capabilities.json` | 声明工具描述、WIT 版本和权限（HTTP、workspace、secrets 等） |
| `../../wit/tool.wit` | WIT 接口定义（由 IronClaw 提供，不需要修改） |

---

## 2. 核心代码解读

```rust
// 从 WIT 接口生成 Rust 绑定
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",   // 指向 IronClaw 的 WIT 定义
});

// 导入生成的类型
use exports::near::agent::tool::{Guest, Request, Response};
use near::agent::host;            // 宿主提供的能力（log、now_millis 等）

struct HelloWorldTool;

impl Guest for HelloWorldTool {
    // 工具执行入口
    fn execute(req: Request) -> Response { ... }

    // 参数的 JSON Schema（告诉 LLM 如何调用，必须包含 required 字段）
    fn schema() -> String { ... }

    // 工具描述（告诉 LLM 这个工具做什么）
    fn description() -> String { ... }
}

export!(HelloWorldTool);
```

### 可用的宿主函数

| 函数 | 说明 | 是否需要权限 |
|------|------|-------------|
| `host::log(level, msg)` | 输出日志 | 不需要 |
| `host::now_millis()` | 获取当前时间戳 | 不需要 |
| `host::workspace_read(path)` | 读取工作区文件 | 需要 workspace 权限 |
| `host::http_request(...)` | 发起 HTTP 请求 | 需要 http 权限 |
| `host::tool_invoke(alias, params)` | 调用其他工具 | 需要 tool_invoke 权限 |
| `host::secret_exists(name)` | 检查密钥是否存在 | 需要 secrets 权限 |

---

## 3. Capabilities 文件格式

`hello-world-tool.capabilities.json` 是工具的元数据和权限声明文件：

```json
{
  "description": "A simple hello-world WASM tool that greets a person by name.",
  "version": "0.1.0",
  "wit_version": "0.3.0"
}
```

### 字段说明

| 字段 | 必须 | 说明 |
|------|------|------|
| `description` | 推荐 | 工具描述，告诉 Agent 这个工具做什么（缺失时用 fallback） |
| `version` | 可选 | 工具版本号（semver） |
| `wit_version` | 推荐 | 编译时使用的 WIT 接口版本，当前为 `"0.3.0"` |
| `http` | 按需 | HTTP 请求权限白名单（不需要 HTTP 则不写） |
| `secrets` | 按需 | 允许访问的密钥名列表 |
| `workspace` | 按需 | 工作区文件读取权限（值为对象 `{"allowed_prefixes": [...]}` ） |
| `auth` | 按需 | 认证配置（API key 等） |
| `setup` | 按需 | 安装时需要用户提供的密钥 |

> ⚠️ **注意**：`workspace` 字段的值必须是**对象**（如 `{"allowed_prefixes": ["data/"]}`），
> **不能**是布尔值 `false`，否则 JSON 解析会失败导致工具无法加载。
> 不需要的权限字段直接不写即可。

### ⚠️ schema() 中必须声明 `required` 字段

IronClaw 会对 WASM 工具的 schema 进行 **compact 压缩**：只保留 `required` 中列出的属性和带 `enum`/`const` 的属性。
如果 schema 中没有 `required` 字段，所有属性都会被压缩掉，导致 LLM 看到的是一个空的 permissive schema，
此时 LLM 可能无法正确识别和调用你的工具。

```rust
// ❌ 错误：没有 required，LLM 看不到参数定义
fn schema() -> String {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        }
    }).to_string()
}

// ✅ 正确：声明 required，LLM 能看到完整参数
fn schema() -> String {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "Name to greet" }
        },
        "required": ["name"]
    }).to_string()
}
```

### 完整权限示例（参考 web-search）

```json
{
  "description": "Search the web using Brave Search.",
  "version": "0.2.0",
  "wit_version": "0.3.0",
  "http": {
    "allowlist": [
      { "host": "api.search.brave.com", "path_prefix": "/res/v1/web/search", "methods": ["GET"] }
    ],
    "credentials": {
      "brave_api_key": {
        "secret_name": "brave_api_key",
        "location": { "type": "header", "name": "X-Subscription-Token" },
        "host_patterns": ["api.search.brave.com"]
      }
    }
  },
  "secrets": { "allowed_names": ["brave_api_key"] },
  "auth": {
    "secret_name": "brave_api_key",
    "display_name": "Brave Search",
    "instructions": "Get a free API key at brave.com/search/api/",
    "env_var": "BRAVE_API_KEY"
  }
}
```

---

## 4. 构建

```bash
cd tools-src/hello-world

# 编译为 WASM 组件
cargo build --target wasm32-wasip2 --release

# 产物位于：
# target/wasm32-wasip2/release/hello_world_tool.wasm
```

---

## 5. 加载到 IronClaw

IronClaw 有两种方式加载 WASM tool：

### 方式 A：Dev Mode（开发模式，仅限本地源码编译运行）

如果 IronClaw 是从源码 `cargo run` 运行的，它会自动扫描 `tools-src/*/` 目录下的构建产物。
**只需编译，重启 IronClaw 即可，无需手动复制文件。**

```bash
cd tools-src/hello-world
cargo build --target wasm32-wasip2 --release
# 然后重启 IronClaw 服务
```

> ⚠️ **重要限制**：Dev mode 依赖编译时的 `CARGO_MANIFEST_DIR` 路径来定位 `tools-src/`。
> 如果你在机器 A 编译 IronClaw 二进制，然后复制到机器 B 运行，dev mode **不会工作**，
> 因为机器 B 上不存在机器 A 的源码路径。此时请使用**方式 B**。
>
> 可以通过设置环境变量 `IRONCLAW_TOOLS_SRC` 来覆盖 dev tools 的搜索路径。

> **命名约定**（dev mode 自动发现规则）：
> - 目录名：`hello-world`
> - crate 名：`hello_world_tool`（Cargo.toml 中的 `name`）
> - WASM 产物：`target/wasm32-wasip2/release/hello_world_tool.wasm`
> - capabilities 文件：`hello-world-tool.capabilities.json`（格式：`<dir>-tool.capabilities.json`）
> - **注册名**：`hello-world-tool`（格式：`<dir>-tool`）

### 方式 B：文件复制（生产部署 / 跨机器部署，推荐）

```bash
mkdir -p ~/.ironclaw/tools
cp target/wasm32-wasip2/release/hello_world_tool.wasm ~/.ironclaw/tools/hello-world-tool.wasm
cp hello-world-tool.capabilities.json ~/.ironclaw/tools/hello-world-tool.capabilities.json
# 然后重启 IronClaw 服务
```

> **命名约定**（`~/.ironclaw/tools/` 目录加载规则）：
> - wasm 文件名（去掉 `.wasm`）就是工具的注册名
> - capabilities 文件名 = wasm 文件名 + `.capabilities.json`（即 `<name>.wasm` 对应 `<name>.capabilities.json`）
> - 例如：`hello-world-tool.wasm` + `hello-world-tool.capabilities.json` → 工具名 `hello-world-tool`

启动时 IronClaw 会自动从 `~/.ironclaw/tools/` 加载所有 `.wasm` 文件，无需额外激活。

### 方式 C：CLI 安装（推荐）

```bash
# 从源码目录安装（自动编译 + 安装）
ironclaw tool install ./tools-src/hello-world --name hello-world-tool

# 或者从已编译的 .wasm 文件安装
ironclaw tool install ./target/wasm32-wasip2/release/hello_world_tool.wasm \
  --name hello-world-tool --skip-build
```

### 验证安装

```bash
# 列出已安装的工具
ironclaw tool list

# 查看工具信息
ironclaw tool info hello-world-tool
```

---

## 6. 使用

安装后，IronClaw 的 Agent 会自动发现这个工具。在对话中：

```
请用 hello-world-tool 工具跟 Alice 打个招呼
```

Agent 会调用：
```json
{"name": "Alice"}
```

返回：
```json
{
  "greeting": "Hello, Alice! 👋 I'm a WASM tool running inside IronClaw's sandbox.",
  "timestamp_ms": 1774644672123
}
```

### 实际验证结果（2026-03-27，服务器 172.16.117.160）

```json
{
  "tool_calls": [
    {
      "name": "hello-world-tool",
      "has_result": true,
      "has_error": false,
      "result_preview": "Hello, Alice! 👋 I'm a WASM tool running inside IronClaw's sandbox."
    }
  ]
}
```

---

## 7. 进阶：开发自己的工具

基于 hello-world 修改即可：

1. **复制目录**：`cp -r tools-src/hello-world tools-src/my-tool`
2. **修改 `Cargo.toml`**：改 `name`
3. **修改 `src/lib.rs`**：
   - 改 `Input`/`Output` 结构体
   - 改 `execute` 逻辑
   - 改 `schema` 和 `description`
4. **修改 `capabilities.json`**：按需添加 HTTP、secrets 等权限
5. **构建安装**：`ironclaw tool install ./tools-src/my-tool --name my-tool`

### 需要 HTTP 的工具示例

参考 `tools-src/slack/` — 它演示了如何：
- 调用外部 API（Slack API）
- 使用 credential injection（密钥注入，WASM 永远看不到真实密钥）
- 在 capabilities.json 中声明 HTTP 白名单

---

## 附：与 peerclaw/ironclaw-plugin 模板的区别

| | hello-world (Tool) | peerclaw/ironclaw-plugin (Channel) |
|---|---|---|
| WIT World | `sandboxed-tool` | `sandboxed-channel` |
| 用途 | Agent 调用的工具 | 消息通道（如 Telegram、P2P） |
| 接口 | `execute` / `schema` / `description` | `on-start` / `on-http-request` / `on-respond` 等 |
| 安装位置 | `~/.ironclaw/tools/` | `~/.ironclaw/extensions/` |
| 安装命令 | `ironclaw tool install` | `ironclaw extension install` |
