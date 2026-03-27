# Ironclaw 安装日志 — 服务器 172.16.117.160

> **日期**: 2026-03-27
> **用户**: ericksun
> **服务器**: ericksun@172.16.117.160
> **操作系统**: CentOS Stream 9 — aarch64 (ARM64)
> **内核**: 5.14.0-565.el9.aarch64

---

## 服务器基线环境

| 组件 | 状态 |
|------|--------|
| gcc | ✅ 11.5.0（预装） |
| cmake | ✅ 3.26.5（预装） |
| pkg-config | ✅ 1.7.3（预装） |
| openssl-devel | ✅ 3.2.2-6.el9（预装） |
| git | ✅ 2.47.1（预装） |
| Rust | ❌ 未安装 |
| PostgreSQL | ❌ 未安装 |
| Docker | ❌ 未安装 |

---

## 安装步骤

### 第 1 步：安装 Rust 工具链

```bash
# 全新安装（之前有一次中断的安装，需要先清理残留）
rm -rf ~/.rustup ~/.cargo
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
rustup default stable

# 修复不完整的组件（中断下载导致 cargo 和 rust-std 缺失）
rustup component remove cargo && rustup component add cargo
rustup component remove rust-std && rustup component add rust-std
```

**结果**：
- rustc 1.94.1 (e408947bf 2026-03-25)
- cargo 1.94.1 (29ea6fb6a 2026-03-24)

### 第 2 步：安装 WASM 编译目标和 wasm-tools

```bash
source ~/.cargo/env
rustup target add wasm32-wasip2
cargo install wasm-tools
```

**结果**：
- wasm-tools 1.245.1
- 编译目标: aarch64-unknown-linux-gnu, wasm32-wasip2

### 第 3 步：安装 PostgreSQL 17

```bash
# 添加 PGDG 官方仓库
sudo dnf install -y https://download.postgresql.org/pub/repos/yum/reporpms/EL-9-aarch64/pgdg-redhat-repo-latest.noarch.rpm

# 禁用系统自带的 PostgreSQL 模块（避免版本冲突）
sudo dnf -qy module disable postgresql

# 安装 PostgreSQL 17
sudo dnf install -y postgresql17-server

# 初始化数据库并启动服务
sudo /usr/pgsql-17/bin/postgresql-17-setup initdb
sudo systemctl start postgresql-17
sudo systemctl enable postgresql-17
```

**结果**：PostgreSQL 17.9 运行在端口 5432

### 第 4 步：安装 pgvector 扩展

```bash
sudo dnf install -y pgvector_17
```

**结果**：pgvector 0.8.2

### 第 5 步：创建数据库

```bash
sudo -u postgres createuser ericksun --createdb
sudo -u postgres createdb ironclaw -O ericksun
sudo -u postgres psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

**结果**：数据库 `ironclaw` 已创建，所有者为 `ericksun`，已启用 vector 扩展

### 第 6 步：克隆源码并编译

```bash
cd ~
git clone https://github.com/nearai/ironclaw.git
cd ironclaw
cargo build --release
```

**结果**：
- 编译耗时: 9 分 31 秒
- 二进制文件: `~/ironclaw/target/release/ironclaw` (61M, 已 strip)
- 版本: ironclaw 0.22.0
- Telegram WASM 通道: 自动编译 (379K)

### 第 7 步：将 ironclaw 加入 PATH

编译完成后，`ironclaw` 二进制文件位于 `~/ironclaw/target/release/ironclaw`，不在系统 PATH 中。
通过创建符号链接到 `~/.cargo/bin/`（已在 PATH 中）解决：

```bash
ln -sf ~/ironclaw/target/release/ironclaw ~/.cargo/bin/ironclaw
```

**结果**：可以直接使用 `ironclaw` 命令

### 第 8 步：运行 Onboard 引导向导

```bash
ironclaw onboard
```

引导向导共 9 个步骤，以下是各步骤的选择和配置：

#### 8.1 数据库配置
- **数据库后端**: PostgreSQL
- **连接地址**: `postgres://ericksun@localhost/ironclaw`

#### 8.2 LLM 提供商配置
- **后端**: OpenAI-compatible
- **模型**: DeepSeek-V3-0324
- **API 端点**: `https://api.haihub.cn/v1`

#### 8.3 安全/密钥存储
- **方式**: OS keychain（系统钥匙串）

#### 8.4 通道（Channels）选择
- ✅ CLI/TUI（始终启用）
- 其他通道（HTTP webhook、Signal、Discord、Feishu、Slack、Telegram、Whatsapp）均未启用

#### 8.5 扩展工具（Extensions）选择

已安装的工具：
| 工具 | 认证方式 | 说明 |
|------|---------|------|
| GitHub | manual | GitHub 集成（Issues、PR、代码搜索） |
| Gmail | oauth | Gmail 邮件读写管理 |
| Google Calendar | oauth | Google 日历事件管理 |
| Google Drive | oauth | Google Drive 文件管理 |
| LLM Context | manual | 通过 Brave Search 获取网页内容做 RAG |
| Slack Tool | oauth | Slack 消息读写 |
| Web Search | manual | Brave 搜索引擎 |

> 安装后需要单独认证的工具：
> ```bash
> ironclaw tool auth github        # GitHub 认证
> ironclaw tool auth gmail         # Gmail OAuth
> ironclaw tool auth llm-context   # Brave Search API key
> ironclaw tool auth slack-tool    # Slack OAuth
> ```

#### 8.6 Docker 沙箱
- **选择**: N（不启用）
- 服务器未安装 Docker，后续可通过 `SANDBOX_ENABLED=true` 开启

#### 8.7 心跳后台任务（Heartbeat）
- **选择**: Y（启用）
- **检查间隔**: 30 分钟（默认）
- **通知渠道**: 留空（默认通过 CLI 显示）

#### 8.8 Onboard 完成

```
✓ ironclaw is ready

  provider    OpenAI-compatible (DeepSeek-V3-0324)
  database    PostgreSQL
  security    OS keychain
```

### 第 9 步：修复 LLM 配置问题

Onboard 向导完成后首次启动，发现 LLM 配置未正确生效（显示为 NEAR AI 默认后端而非配置的 DeepSeek）。

**原因分析**：Onboard 向导将 LLM 设置保存在数据库 `settings` 表中，但启动时未能正确加载。配置加载优先级为：`环境变量 > .env 文件 > TOML 配置 > 数据库 settings 表 > 默认值`。

**解决方案**：在 `~/.ironclaw/.env` 中手动添加 LLM 配置（环境变量优先级最高）：

```bash
vim ~/.ironclaw/.env
```

添加以下内容：

```env
LLM_BACKEND="openai_compatible"
LLM_BASE_URL="https://api.haihub.cn/v1"
LLM_API_KEY="<你的 API Key>"
SANDBOX_ENABLED="false"

# ── HaiHub 平台可用模型（取消注释切换，同一时间只能启用一个） ──
LLM_MODEL="DeepSeek-V3-0324"
# LLM_MODEL="DeepSeek-V3.1"
# LLM_MODEL="Kimi-K2"
# LLM_MODEL="Qwen3-235B-A22B"
# LLM_MODEL="Qwen3-32B-FP8"
```

> ⚠️ **注意**：`LLM_BASE_URL` 必须使用 `https://`，ironclaw 对非 localhost 的远程端点强制要求 HTTPS，使用 `http://` 会报错：
> `Invalid configuration value for LLM_BASE_URL: HTTP (non-TLS) is only allowed for localhost`

### 第 10 步：配置 HaiHub 模型 Provider（providers.json）

ironclaw 原生支持用户自定义 provider 配置文件 `~/.ironclaw/providers.json`。启动时会自动加载内置 provider + 用户自定义 provider，通过 ID 或别名即可快速切换。

#### 10.1 创建 providers.json

在本地创建 `providers.json`，定义 HaiHub 平台的 5 个模型为独立 provider：

```json
[
  {
    "id": "haihub-ds",
    "aliases": ["ds", "deepseek-v3"],
    "protocol": "open_ai_completions",
    "default_base_url": "https://api.haihub.cn/v1",
    "base_url_env": "LLM_BASE_URL",
    "api_key_env": "LLM_API_KEY",
    "api_key_required": true,
    "model_env": "LLM_MODEL",
    "default_model": "DeepSeek-V3-0324",
    "description": "HaiHub - DeepSeek V3 (2024-03-24)",
    "setup": {
      "kind": "api_key",
      "secret_name": "llm_haihub_api_key",
      "display_name": "HaiHub DeepSeek-V3",
      "can_list_models": false
    }
  },
  {
    "id": "haihub-ds31",
    "aliases": ["ds31", "deepseek-v3.1"],
    "protocol": "open_ai_completions",
    "default_base_url": "https://api.haihub.cn/v1",
    "base_url_env": "LLM_BASE_URL",
    "api_key_env": "LLM_API_KEY",
    "api_key_required": true,
    "model_env": "LLM_MODEL",
    "default_model": "DeepSeek-V3.1",
    "description": "HaiHub - DeepSeek V3.1",
    "setup": { "kind": "api_key", "secret_name": "llm_haihub_api_key", "display_name": "HaiHub DeepSeek-V3.1", "can_list_models": false }
  },
  {
    "id": "haihub-kimi",
    "aliases": ["kimi", "kimi-k2"],
    "protocol": "open_ai_completions",
    "default_base_url": "https://api.haihub.cn/v1",
    "base_url_env": "LLM_BASE_URL",
    "api_key_env": "LLM_API_KEY",
    "api_key_required": true,
    "model_env": "LLM_MODEL",
    "default_model": "Kimi-K2",
    "description": "HaiHub - Moonshot Kimi K2",
    "setup": { "kind": "api_key", "secret_name": "llm_haihub_api_key", "display_name": "HaiHub Kimi-K2", "can_list_models": false }
  },
  {
    "id": "haihub-qwen",
    "aliases": ["qwen", "qwen3-235b"],
    "protocol": "open_ai_completions",
    "default_base_url": "https://api.haihub.cn/v1",
    "base_url_env": "LLM_BASE_URL",
    "api_key_env": "LLM_API_KEY",
    "api_key_required": true,
    "model_env": "LLM_MODEL",
    "default_model": "Qwen3-235B-A22B",
    "description": "HaiHub - Qwen3 235B MoE (22B active)",
    "setup": { "kind": "api_key", "secret_name": "llm_haihub_api_key", "display_name": "HaiHub Qwen3-235B", "can_list_models": false }
  },
  {
    "id": "haihub-qwen32",
    "aliases": ["qwen32", "qwen3-32b"],
    "protocol": "open_ai_completions",
    "default_base_url": "https://api.haihub.cn/v1",
    "base_url_env": "LLM_BASE_URL",
    "api_key_env": "LLM_API_KEY",
    "api_key_required": true,
    "model_env": "LLM_MODEL",
    "default_model": "Qwen3-32B-FP8",
    "description": "HaiHub - Qwen3 32B FP8",
    "setup": { "kind": "api_key", "secret_name": "llm_haihub_api_key", "display_name": "HaiHub Qwen3-32B", "can_list_models": false }
  }
]
```

#### 10.2 部署到服务器

```bash
rsync -avz providers.json ericksun@172.16.117.160:~/.ironclaw/providers.json
```

#### 10.3 切换模型

使用 `ironclaw models set-provider` 命令，支持 ID 或别名：

```bash
# 通过别名切换（推荐，简短好记）
ironclaw models set-provider ds          # DeepSeek-V3-0324（默认）
ironclaw models set-provider ds31        # DeepSeek-V3.1
ironclaw models set-provider kimi        # Kimi-K2
ironclaw models set-provider qwen        # Qwen3-235B-A22B
ironclaw models set-provider qwen32      # Qwen3-32B-FP8

# 通过完整 ID 切换
ironclaw models set-provider haihub-kimi

# 查看当前状态
ironclaw models status

# 查看所有可用 provider
ironclaw models list
```

切换后需重启服务生效：

```bash
sudo systemctl restart ironclaw
```

**结果**：

```
$ ironclaw models status
Provider: haihub-ds (HaiHub - DeepSeek V3 (2024-03-24))
Model:    DeepSeek-V3-0324
```

启动日志确认加载成功：
```
INFO Loaded user provider definitions count=5 path=/home/ericksun/.ironclaw/providers.json
```

#### HaiHub 平台可用模型一览

| Provider ID | 别名 | 默认模型 | 说明 |
|------------|------|---------|------|
| haihub-ds | ds, deepseek-v3 | DeepSeek-V3-0324 | DeepSeek V3 2024年3月24日版本（当前默认） |
| haihub-ds31 | ds31, deepseek-v3.1 | DeepSeek-V3.1 | DeepSeek V3.1 新版本 |
| haihub-kimi | kimi, kimi-k2 | Kimi-K2 | Moonshot Kimi K2 模型 |
| haihub-qwen | qwen, qwen3-235b | Qwen3-235B-A22B | 通义千问3 235B MoE（激活 22B） |
| haihub-qwen32 | qwen32, qwen3-32b | Qwen3-32B-FP8 | 通义千问3 32B FP8 量化版 |

### 第 11 步：验证启动

```bash
ironclaw
```

**结果**：

```
ironclaw v0.22.0

model       DeepSeek-V3-0324  via haihub-ds
gateway     http://0.0.0.0:3000/?token=...
features    db:postgres  tools:44  routines  heartbeat:30m  skills

ready in 492ms
```

成功使用 DeepSeek-V3-0324 模型启动（通过 `haihub-ds` provider），对话功能正常。

---

## 最终验证

| 组件 | 版本 | 状态 |
|------|------|------|
| Rust (rustc) | 1.94.1 | ✅ |
| Cargo | 1.94.1 | ✅ |
| wasm-tools | 1.245.1 | ✅ |
| wasm32-wasip2 target | — | ✅ |
| PostgreSQL | 17.9 | ✅ 运行中 |
| pgvector | 0.8.2 | ✅ |
| ironclaw 二进制 | 0.22.0 | ✅ |
| Telegram WASM | 379K | ✅ |
| LLM 模型 | DeepSeek-V3-0324（默认） | ✅ |
| LLM 可用模型 | DeepSeek-V3-0324, DeepSeek-V3.1, Kimi-K2, Qwen3-235B-A22B, Qwen3-32B-FP8 | ✅ |
| LLM 后端 | haihub-ds (HaiHub providers.json) | ✅ |
| providers.json | 5 个 HaiHub provider（用户自定义） | ✅ |
| 心跳任务 | 每 30 分钟 | ✅ |
| 已安装工具数 | 44 | ✅ |
| systemd 服务 | ironclaw.service | ✅ 运行中，开机自启 |
| Web Gateway | 0.0.0.0:3000 | ✅ 外部可访问 |
| 磁盘使用 | 28G/45G (62%) | ✅ |

---

### 第 12 步：配置 Web 外部访问

在 `~/.ironclaw/.env` 中添加 Gateway 配置，允许外部浏览器访问 Web UI：

```env
GATEWAY_HOST=0.0.0.0
GATEWAY_PORT=3000
GATEWAY_AUTH_TOKEN=9ec5f109867f6a6dd2327710c3881eb90d722e904f87b9baa3f8dd39b7819a0c
```

开放防火墙端口：

```bash
sudo firewall-cmd --permanent --add-port=3000/tcp
sudo firewall-cmd --reload
```

**结果**：可通过浏览器访问 Web UI

#### 12.1 浏览器访问 Web UI（推荐）

在本地电脑浏览器中打开以下地址即可进入聊天界面：

```
http://172.16.117.160:3000/?token=9ec5f109867f6a6dd2327710c3881eb90d722e904f87b9baa3f8dd39b7819a0c
```

Web UI 内置了完整的聊天界面，支持：
- 流式输出（SSE 实时推送 agent 回复）
- 多线程对话管理
- 工具调用审批
- 工作区文件浏览

> ⚠️ **安全提示**：URL 中的 token 即为认证凭据，请勿泄露。如需更换 token，修改 `~/.ironclaw/.env` 中的 `GATEWAY_AUTH_TOKEN` 后重启服务。

#### 12.2 API 交互方式

ironclaw 的 chat 接口是**异步设计**的：发送消息后立即返回 `message_id`，agent 的回复通过 SSE 事件流或 WebSocket 异步推送。

**发送消息**（POST）：

```bash
curl -X POST http://172.16.117.160:3000/api/chat/send \
  -H "Authorization: Bearer 9ec5f109867f6a6dd2327710c3881eb90d722e904f87b9baa3f8dd39b7819a0c" \
  -H "Content-Type: application/json" \
  -d '{"content": "你好，介绍一下你自己"}'
# 返回: {"message_id":"...","status":"accepted"}
```

**接收回复**（SSE 事件流）— 需要在另一个终端先开启监听：

```bash
curl -N "http://172.16.117.160:3000/api/chat/events?token=9ec5f109867f6a6dd2327710c3881eb90d722e904f87b9baa3f8dd39b7819a0c"
```

**常用 API 端点一览**：

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/health` | 健康检查（无需认证） |
| POST | `/api/chat/send` | 发送消息 |
| GET | `/api/chat/events` | SSE 事件流（接收 agent 回复） |
| GET | `/api/chat/ws` | WebSocket 双向通信 |
| GET | `/api/chat/history` | 对话历史 |
| GET | `/api/chat/threads` | 线程列表 |
| GET | `/api/gateway/status` | Gateway 状态信息 |

### 第 13 步：配置 systemd 服务管理

#### 13.1 创建 systemd 服务文件

```bash
sudo vim /etc/systemd/system/ironclaw.service
```

内容：

```ini
[Unit]
Description=IronClaw AI Assistant
Documentation=https://github.com/nearai/ironclaw
After=network-online.target postgresql-17.service
Wants=network-online.target
Requires=postgresql-17.service
StartLimitBurst=3
StartLimitIntervalSec=60

[Service]
Type=simple
User=ericksun
Group=ericksun
WorkingDirectory=/home/ericksun/ironclaw

EnvironmentFile=/home/ericksun/.ironclaw/.env
Environment=PATH=/home/ericksun/.cargo/bin:/usr/local/bin:/usr/bin:/bin

ExecStart=/home/ericksun/ironclaw/target/release/ironclaw

Restart=on-failure
RestartSec=5

StandardOutput=journal
StandardError=journal
SyslogIdentifier=ironclaw

NoNewPrivileges=true
PrivateTmp=true

KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=30

[Install]
WantedBy=multi-user.target
```

#### 13.2 修复 SELinux 上下文

CentOS 9 默认 SELinux 为 Enforcing 模式，`user_home_t` 类型的文件无法被 systemd 服务进程读取。需要修改 SELinux 上下文：

```bash
# 二进制文件标记为 bin_t
sudo semanage fcontext -a -t bin_t '/home/ericksun/ironclaw/target/release/ironclaw'
sudo restorecon -v /home/ericksun/ironclaw/target/release/ironclaw

# 环境变量文件标记为 etc_t
sudo semanage fcontext -a -t etc_t '/home/ericksun/.ironclaw/.env'
sudo restorecon -v /home/ericksun/.ironclaw/.env

# 放开 .env 文件权限（从 600 改为 644）
chmod 644 /home/ericksun/.ironclaw/.env
```

#### 13.3 启用并启动服务

```bash
sudo systemctl daemon-reload
sudo systemctl enable ironclaw
sudo systemctl start ironclaw
sudo systemctl status ironclaw
```

**结果**：

```
● ironclaw.service - IronClaw AI Assistant
     Active: active (running)
   Main PID: 40738 (ironclaw)
     Memory: 77.1M

  ironclaw v0.22.0
  model       DeepSeek-V3-0324  via openai_compatible
  gateway     http://0.0.0.0:3000/?token=...
  features    db:postgres  tools:44  routines  heartbeat:30m  skills
  ready in 1.5s
```

#### 13.4 常用管理命令

```bash
sudo systemctl status ironclaw      # 查看状态
sudo systemctl stop ironclaw        # 停止
sudo systemctl restart ironclaw     # 重启（修改 .env 后使用）
sudo journalctl -u ironclaw -f      # 查看实时日志
sudo journalctl -u ironclaw -n 100  # 查看最近 100 行日志
```

---

## 后续步骤

1. **配置工具认证**（按需执行）：
   ```bash
   ironclaw tool auth github        # GitHub 认证
   ironclaw tool auth gmail         # Gmail OAuth
   ironclaw tool auth llm-context   # Brave Search API key
   ironclaw tool auth slack-tool    # Slack OAuth
   ```

2. **查看系统状态**：
   ```bash
   ironclaw status
   ```

---

## WASM Tool 实验记录

> **日期**: 2026-03-27
> **目标**: 验证 IronClaw 的 WASM Tool 沙箱功能，从零开发一个 hello-world 工具并成功在对话中调用
> **参考文档**: `my_install/WASM_TOOL_GUIDE.md`

### 实验概述

在服务器 172.16.117.160 上完成了 WASM Tool 的完整开发→编译→部署→调试→验证流程。过程中遇到了 3 个关键问题，逐一排查解决后，最终成功在 Web UI 对话中通过 Agent 调用了自定义 WASM 工具。

### 实验步骤

#### 1. 创建 hello-world 工具源码

在本地 `tools-src/hello-world/` 目录下创建了最简 WASM 工具：

- `Cargo.toml` — 声明 `crate-type = ["cdylib"]`，依赖 `wit-bindgen`、`serde`、`serde_json`
- `src/lib.rs` — 实现 `Guest` trait 的 `execute`、`schema`、`description` 三个方法
- `hello-world-tool.capabilities.json` — 工具元数据和权限声明

#### 2. 同步到服务器并编译

```bash
# 同步源码到服务器
scp -r tools-src/hello-world/ ericksun@172.16.117.160:~/ironclaw/tools-src/hello-world/

# 在服务器上编译 WASM
ssh ericksun@172.16.117.160
cd ~/ironclaw/tools-src/hello-world
cargo build --target wasm32-wasip2 --release
# 产物: target/wasm32-wasip2/release/hello_world_tool.wasm (112 KB)
```

#### 3. 部署到 IronClaw

```bash
# 复制到 IronClaw 工具目录
cp target/wasm32-wasip2/release/hello_world_tool.wasm ~/.ironclaw/tools/hello-world-tool.wasm
cp hello-world-tool.capabilities.json ~/.ironclaw/tools/hello-world-tool.capabilities.json

# 验证安装
ironclaw tool list
# 输出: hello-world-tool (112.0 KB, caps: ✓)

# 重启服务
sudo systemctl restart ironclaw
```

#### 4. 遇到的问题与解决

##### 问题 1：capabilities 文件格式错误 — 工具加载失败

**现象**：`ironclaw tool list` 能看到工具，但运行时 Agent 报"找不到工具 'hello-world-tool'"。

**根因**：capabilities 文件中 `"workspace": false` 使用了布尔值，但代码期望的是 `Option<WorkspaceCapabilitySchema>` 对象。JSON 反序列化失败导致整个工具**静默加载失败**（无错误日志）。

```json
// ❌ 错误的 capabilities 文件
{
  "http": {},
  "workspace": false,    // 布尔值导致反序列化失败
  "secrets": {}
}
```

**修复**：不需要的权限字段直接不写，同时添加 `description` 和 `wit_version`：

```json
// ✅ 正确的 capabilities 文件
{
  "description": "A simple hello-world WASM tool that greets a person by name.",
  "version": "0.1.0",
  "wit_version": "0.3.0"
}
```

**关键教训**：
- `workspace` 字段只接受对象（如 `{"allowed_prefixes": ["data/"]}"`），不接受 `false`
- `wit_version` 应与 IronClaw 的 `WIT_TOOL_VERSION`（当前 `"0.3.0"`）匹配
- 不需要的权限字段**直接省略**，不要写 `false` 或 `{}`

##### 问题 2：schema() 缺少 required 字段 — LLM 看不到工具参数

**现象**：工具已成功加载（通过 API 确认在 54 个工具列表中），但 Agent 不调用它，而是选择了内置的 `message` 工具。

**根因**：IronClaw 对 WASM 工具的 schema 进行 **compact 压缩**（`compact_schema()` 方法），只保留 `required` 中列出的属性和带 `enum`/`const` 的属性。由于 `schema()` 没有声明 `required` 字段，所有属性被压缩掉，advertised schema 变成了 permissive（空参数），LLM 无法识别工具参数。

```rust
// ❌ 错误：没有 required，compact_schema 会压缩掉所有属性
fn schema() -> String {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "Name to greet" }
        }
    }).to_string()
}

// ✅ 正确：声明 required，LLM 能看到完整参数
fn schema() -> String {
    serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "Name of the person to greet" }
        },
        "required": ["name"]
    }).to_string()
}
```

**关键教训**：
- WASM 工具的 `schema()` **必须**包含 `required` 字段
- 没有 `required` → advertised schema 变成 permissive → description 被追加 `(call tool_info(...) for parameter schema)` 提示 → LLM 倾向于跳过该工具

##### 问题 3：Qwen3 模型 function calling 能力不足

**现象**：修复 schema 后，通过 API 直接调用验证成功，但在 Web UI 对话中 Agent 仍然不调用 `hello-world-tool`，回复"检测到您可能想使用不存在的工具"。

**根因**：当时使用的 LLM 模型是 **Qwen3-235B-A22B**（通过 HaiHub API），该模型在面对 54 个工具时 function calling 能力不足，无法正确匹配用户明确指定的工具名称。"检测到您可能想使用不存在的工具"是模型自己生成的文本（代码中无此硬编码），说明模型没有仔细检查工具列表。

**修复**：切换到 **DeepSeek-V3-0324** 模型：

```bash
# 通过 CLI 切换 provider 和模型
ironclaw models set-provider haihub-ds
ironclaw models set DeepSeek-V3-0324

# 重启服务
sudo systemctl restart ironclaw
```

> ⚠️ 切换模型后，**旧的对话 session 可能缓存了旧模型信息**，需要**新开对话**才能使用新模型。

**关键教训**：
- 不同 LLM 模型的 function calling 能力差异很大
- Qwen3-235B-A22B 在工具数量较多（54个）时 function calling 表现不佳
- DeepSeek-V3-0324 能正确处理大量工具的 function calling
- 切换模型后需要新开对话 session

#### 5. 服务重启问题

切换模型过程中遇到 systemd 重启失败：

**现象**：`sudo systemctl restart ironclaw` 报错 `control process exited with error code`。

**根因**：
1. 旧的 ironclaw 进程（通过 `ironclaw service start` 启动的）未被 systemd 管理，PID 文件残留导致新实例检测到冲突拒绝启动
2. 数据库中存储的 provider 设置优先级高于 `config.toml`，导致配置文件修改不生效

**修复**：
```bash
# 1. 杀掉所有残留进程
sudo pkill -f 'target/release/ironclaw'
rm -f ~/.ironclaw/ironclaw.pid

# 2. 通过 CLI 命令修改模型（写入数据库，优先级最高）
ironclaw models set-provider haihub-ds
ironclaw models set DeepSeek-V3-0324

# 3. 重置 systemd 状态并启动
sudo systemctl reset-failed ironclaw.service
sudo systemctl start ironclaw.service
```

### 最终验证结果 ✅

切换到 DeepSeek-V3-0324 后，在 Web UI 新对话中成功调用 hello-world-tool：

- **用户输入**：`用 hello-world-tool 工具跟 Alice 打个招呼`
- **Agent 行为**：正确调用 `hello-world-tool`，传入参数 `{"name": "Alice"}`
- **工具返回**：`Hello, Alice! 👋 I'm a WASM tool running inside IronClaw's sandbox.`

![WASM 工具调用成功截图](https://zhiyan-ai-agent-with-1258344702.cos.ap-guangzhou.tencentcos.cn/copilot/5de9242a-c118-4164-8e3e-c42bee5f4829/image-019d2f8e19fb711b82dec749ff784e17.png)

### 实验结论

| 项目 | 结果 |
|------|------|
| WASM 工具开发 | ✅ 使用 Rust + wit-bindgen，实现 Guest trait |
| WASM 编译 | ✅ `cargo build --target wasm32-wasip2 --release` |
| 工具部署 | ✅ 复制到 `~/.ironclaw/tools/` 目录 |
| 工具加载 | ✅ IronClaw 启动时自动加载 |
| Agent 调用 | ✅ DeepSeek-V3 正确识别并调用 |
| 沙箱隔离 | ✅ WASM 运行在沙箱中，无法直接访问系统资源 |

### 踩坑清单

| # | 问题 | 根因 | 修复 |
|---|------|------|------|
| 1 | 工具静默加载失败 | capabilities.json 中 `"workspace": false` 类型错误 | 不需要的权限字段直接不写 |
| 2 | LLM 看不到工具参数 | schema() 缺少 `required` 字段，compact_schema 压缩为空 | 添加 `"required": ["name"]` |
| 3 | LLM 不调用工具 | Qwen3 模型 function calling 能力不足（54 个工具） | 切换到 DeepSeek-V3 |
| 4 | systemd 重启失败 | PID 文件残留 + 数据库设置覆盖 config.toml | 清理进程 + 用 CLI 命令修改模型 |
| 5 | 切换模型后旧对话不生效 | 对话 session 缓存了旧模型信息 | 新开对话 session |

### 学习收获

通过这次 WASM Tool 实验，可以学到以下关键知识：

#### 1. IronClaw 的扩展架构

IronClaw 有两种主要的扩展机制：**WASM Tool**（代码级工具）和 **Skill**（提示词级扩展）。理解它们各自的定位和适用场景，是开发 IronClaw 插件的基础。

#### 2. WASM 沙箱安全模型

WASM Tool 运行在 `wasm32-wasip2` 沙箱中，无法直接访问宿主系统资源。所有外部能力（HTTP 请求、文件读取、密钥访问）都需要通过 `capabilities.json` 显式声明白名单。这是一种**能力导向（capability-based）**的安全模型——默认拒绝一切，按需授权。

#### 3. LLM Function Calling 的工程细节

- **Schema 压缩**：IronClaw 不会把完整的 JSON Schema 发给 LLM，而是通过 `compact_schema()` 只保留 `required` 字段和 `enum`/`const` 属性，减少 token 消耗。这意味着开发者**必须**在 schema 中声明 `required`，否则参数会被压缩掉。
- **Permissive 降级**：当 schema 被压缩为空时，工具会被标记为 permissive，LLM 需要额外调用 `tool_info` 才能获取参数定义，这大大降低了工具被选中的概率。
- **工具数量影响**：当可用工具数量较多（如 54 个）时，不同 LLM 模型的 function calling 能力差异显著。Qwen3 在大量工具场景下表现不佳，DeepSeek-V3 则能正确处理。

#### 4. 静默失败的排查思路

WASM 工具加载失败时**不会报错**（JSON 反序列化失败被静默吞掉），这是最难排查的问题类型。排查思路：
1. 先用 `ironclaw tool list` 确认工具是否被发现
2. 再通过 API `/api/extensions/tools` 确认工具是否被运行时加载
3. 用 `xxd` 检查文件的实际二进制内容（排除编码问题）
4. 对照源码中的类型定义检查 JSON 字段类型是否匹配

#### 5. 配置优先级体系

IronClaw 的配置有多个来源，优先级从高到低：
1. **数据库**（通过 CLI 命令 `ironclaw models set-*` 写入）
2. **环境变量**（`.env` 文件）
3. **配置文件**（`config.toml`）
4. **编译时默认值**

修改低优先级的配置不会覆盖高优先级的设置，这是"改了 config.toml 但不生效"的常见原因。

### WASM Tool vs Skill 对比

IronClaw 提供了两种截然不同的扩展机制，它们在架构层面互补：

| 维度 | WASM Tool（代码级工具） | Skill（提示词级扩展） |
|------|------------------------|----------------------|
| **本质** | 编译后的二进制代码（`.wasm`） | Markdown 文本文件（`SKILL.md`） |
| **运行位置** | WASM 沙箱（宿主进程外） | LLM 上下文（注入到系统提示词中） |
| **开发语言** | Rust（通过 wit-bindgen） | 自然语言（YAML frontmatter + Markdown） |
| **能力** | 执行计算、调用 API、处理数据 | 指导 LLM 行为、提供领域知识、定义工作流 |
| **调用方式** | LLM 通过 function calling 主动调用 | 根据用户消息关键词自动激活，注入到 LLM 上下文 |
| **激活机制** | LLM 根据工具 description + schema 决定是否调用 | 确定性评分（keyword/tag/regex 匹配），无 LLM 参与 |
| **安全模型** | 能力白名单（capabilities.json 声明 HTTP/文件/密钥权限） | 信任分级（Trusted 可用所有工具，Installed 只能用只读工具） |
| **部署位置** | `~/.ironclaw/tools/` | `~/.ironclaw/skills/` 或 `<workspace>/skills/` |
| **开发门槛** | 高（需要 Rust + WASM 工具链） | 低（只需写 Markdown） |
| **典型用途** | Web 搜索、GitHub 集成、数据库查询 | 写作助手、代码审查指南、领域专家角色 |

#### 联系

1. **互补关系**：Skill 告诉 LLM "怎么思考"，WASM Tool 给 LLM "能力去执行"。例如一个"代码审查"Skill 可以指导 LLM 的审查策略，同时依赖 `github` WASM Tool 来实际读取 PR 代码。
2. **共同的注册体系**：两者都通过 IronClaw 的 Tool Registry 统一管理，Agent 在每次对话中同时感知两者。
3. **Skill 可以影响 Tool 的可用性**：当 Installed（低信任）Skill 被激活时，IronClaw 会通过 **authority attenuation** 机制自动移除非只读工具，防止不受信任的 Skill 指令操纵 LLM 调用危险工具。

#### 区别的核心

- **WASM Tool = 给 Agent 新的"手"**：让 Agent 能做之前做不到的事（调 API、处理文件、执行计算）
- **Skill = 给 Agent 新的"脑"**：让 Agent 在特定领域更聪明（知道该怎么做、遵循什么规范、采用什么策略）

#### 选择建议

| 场景 | 推荐 |
|------|------|
| 需要调用外部 API（搜索、数据库、第三方服务） | WASM Tool |
| 需要执行计算或数据处理 | WASM Tool |
| 需要指导 LLM 的行为模式或工作流 | Skill |
| 需要注入领域知识或角色设定 | Skill |
| 需要两者结合（如"用特定策略调用特定 API"） | Skill + WASM Tool |

---

## 故障排除记录

1. **Rust 安装中断**：首次 `rustup` 安装过程中被中断，导致工具链损坏。解决方法：`rm -rf ~/.rustup ~/.cargo` 后重新安装。
2. **cargo/rust-std 组件缺失**：部分安装后 `cargo` 和 `rust-std` 组件缺失。解决方法：`rustup component remove <组件> && rustup component add <组件>`。
3. **无 perl-IPC-Run 问题**：与另一台服务器 (172.16.117.155) 不同，此服务器未安装 `postgresql17-devel`（运行时不需要），因此避免了 `perl(IPC::Run)` 依赖问题。
4. **ironclaw 命令找不到**：源码编译的二进制不在 PATH 中。解决方法：`ln -sf ~/ironclaw/target/release/ironclaw ~/.cargo/bin/ironclaw`。
5. **LLM 配置未生效**：Onboard 向导的 LLM 设置保存在数据库中但未正确加载，启动后回退到 NEAR AI 默认后端。解决方法：在 `~/.ironclaw/.env` 中手动设置 `LLM_BACKEND`、`LLM_BASE_URL`、`LLM_API_KEY`、`LLM_MODEL` 环境变量。
6. **LLM_BASE_URL 必须使用 HTTPS**：远程 API 端点使用 `http://` 会被拒绝。解决方法：将 URL 协议改为 `https://`。
7. **Sandbox 警告**：即使在 Onboard 中选择不启用 Docker sandbox，仍可能出现 sandbox 启用的警告。解决方法：在 `.env` 中显式设置 `SANDBOX_ENABLED="false"`。
8. **systemd 启动失败 — SELinux Permission Denied**：CentOS 9 的 SELinux Enforcing 模式下，systemd 服务进程无法读取 `user_home_t` 类型的文件。解决方法：使用 `semanage fcontext` 将二进制标记为 `bin_t`，将 `.env` 文件标记为 `etc_t`，然后 `restorecon` 应用更改。
9. **systemd StartLimitIntervalSec 位置错误**：在 CentOS 9 的 systemd 版本中，`StartLimitBurst` 和 `StartLimitIntervalSec` 应放在 `[Unit]` 段而非 `[Service]` 段，否则会被忽略。
