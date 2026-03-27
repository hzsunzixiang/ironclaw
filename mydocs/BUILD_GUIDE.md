# Ironclaw Linux 部署指南

> **目标环境**: Linux RHEL/CentOS 9 — aarch64 (ARM64)
>
> ```
> Linux IronClaw 5.14.0-447.el9.aarch64 #1 SMP PREEMPT_DYNAMIC
> ```

## 一、项目概览

Ironclaw 是一个用 Rust 编写的安全优先个人 AI 助手框架。
- **版本**: 0.22.0
- **Rust 最低版本**: 1.92
- **License**: MIT OR Apache-2.0
- **部署平台**: Linux aarch64 (RHEL/CentOS 9)

---

## 二、构建前提条件

### 2.1 必需依赖

| 依赖 | 版本要求 | 用途 |
|------|---------|------|
| **Rust** | 1.92+ | 主编译工具链 |
| **PostgreSQL** | 15+ | 数据持久化（默认数据库） |
| **pgvector** 扩展 | - | PostgreSQL 向量搜索支持 |
| **wasm32-wasip2** target | - | 编译 WASM 通道（Telegram 等） |
| **wasm-tools** | - | WASM 组件模型转换和裁剪 |

### 2.2 系统编译依赖（RHEL/CentOS 9）

```bash
# 安装基础编译工具和依赖库
sudo dnf groupinstall -y "Development Tools"
sudo dnf install -y \
    gcc gcc-c++ make \
    pkg-config \
    openssl-devel \
    perl-core \
    cmake \
    git \
    curl
```

> **说明**: `openssl-devel` 是编译 Rust 网络相关 crate 的必需依赖；`cmake` 在启用 `bedrock` feature 时需要。

### 2.3 可选依赖

| 依赖 | 用途 | 启用方式 |
|------|------|---------|
| **CMake** | AWS Bedrock feature 编译需要 | `--features bedrock` |
| **Docker** | 沙箱容器执行 | 运行时可选 |
| **NEAR AI 账号** | 默认 LLM 提供商认证 | `ironclaw onboard` |

---

## 三、构建方式总览

在 Linux 服务器上推荐以下 **3 种构建方式**：

```
┌──────────────────────────────────────────────────┐
│           Ironclaw Linux 构建方式                  │
├──────────────┬──────────────┬────────────────────┤
│ 1. 源码编译   │ 2. Docker    │ 3. 预编译二进制     │
│ cargo build  │ docker build │ Release 下载        │
│ (开发/学习)   │ (部署/隔离)   │ (快速使用)          │
└──────────────┴──────────────┴────────────────────┘
```

---

## 四、方式一：源码编译（推荐用于开发学习）

### 4.1 安装 Rust 工具链

```bash
# Step 1: 安装 Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# 安装过程中选择默认选项 (1)

# Step 2: 加载环境变量（当前 shell 立即生效）
. "$HOME/.cargo/env"

# Step 3: 验证安装
rustc --version
# 预期输出: rustc 1.94.1 (e408947bf 2026-03-25) 或更新版本

# Step 4: 将 cargo 环境变量写入 shell 配置（永久生效）
echo '. "$HOME/.cargo/env"' >> ~/.bashrc

# Step 5: 添加 WASM 编译目标（build.rs 会自动编译 Telegram channel）
rustup target add wasm32-wasip2

# Step 6: 安装 wasm-tools（WASM 组件模型转换）
cargo install wasm-tools
```

### 4.2 安装 PostgreSQL + pgvector

#### 方式 A：直接安装（RHEL/CentOS 9）

```bash
# Step 1: 安装 PostgreSQL 官方仓库
sudo dnf install -y https://download.postgresql.org/pub/repos/yum/reporpms/EL-9-aarch64/pgdg-redhat-repo-latest.noarch.rpm

# Step 2: 禁用系统自带的 PostgreSQL 模块（避免冲突）
sudo dnf -qy module disable postgresql

# Step 3: 安装 PostgreSQL 17
sudo dnf install -y postgresql17-server postgresql17-devel

# Step 4: 初始化数据库
sudo /usr/pgsql-17/bin/postgresql-17-setup initdb

# Step 5: 启动并设置开机自启
sudo systemctl start postgresql-17
sudo systemctl enable postgresql-17

# Step 6: 安装 pgvector 扩展
# 方法一：通过 yum 仓库安装（如果可用）
sudo dnf install -y pgvector_17

# 方法二：从源码编译安装 pgvector
# cd /tmp
# git clone --branch v0.8.0 https://github.com/pgvector/pgvector.git
# cd pgvector
# export PG_CONFIG=/usr/pgsql-17/bin/pg_config
# make
# sudo make install
```

#### 方式 B：使用 Docker（更简单）

```bash
# 启动 PostgreSQL + pgvector 容器（推荐用于快速部署）
# 需要先将项目 clone 到服务器，然后使用 docker-compose
docker compose up -d

# 数据库连接信息:
# host: localhost:5432
# db:   ironclaw
# user: ironclaw
# pass: ironclaw (仅开发用)
```

### 4.3 创建数据库

```bash
# 如果使用直接安装的 PostgreSQL：
# 先切换到 postgres 用户
sudo -u postgres createuser ironclaw --createdb
sudo -u postgres createdb ironclaw -O ironclaw

# 启用 pgvector 扩展
sudo -u postgres psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"

# 如果使用 Docker 方式，数据库已自动创建，跳过此步骤
```

### 4.4 获取源码并编译

```bash
# Step 1: Clone 仓库
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# Step 2: Debug 编译（快，适合开发）
cargo build

# Step 3: Release 编译（慢，适合部署）
cargo build --release

# 产物位置
# Debug:   target/debug/ironclaw
# Release: target/release/ironclaw
```

> **提示**: 首次编译需要 10-20 分钟（依赖约 400+ crate，包括 wasmtime、cranelift 等大型依赖）。后续增量编译会快很多。

### 4.5 build.rs 自动构建流程

编译时 `build.rs` 会自动执行以下操作：

```
build.rs 执行流程:
│
├── 1. 收集 registry/ 目录下的 JSON 清单
│   ├── registry/tools/*.json      (11个工具清单)
│   ├── registry/channels/*.json   (5个通道清单)
│   ├── registry/mcp-servers/*.json (7个MCP服务清单)
│   └── registry/_bundles.json     (捆绑包定义)
│   → 输出: $OUT_DIR/embedded_catalog.json
│
└── 2. 编译 Telegram WASM 通道
    ├── cargo build --release --target wasm32-wasip2
    │   (编译 channels-src/telegram/)
    ├── wasm-tools component new (转为组件模型)
    └── wasm-tools strip (去除调试信息)
    → 输出: channels-src/telegram/telegram.wasm
```

> **注意**: 如果 `wasm32-wasip2` target 或 `wasm-tools` 未安装，build.rs 会打印警告但不会阻止主程序编译。

### 4.6 完整构建脚本（含所有通道）

```bash
# 构建所有捆绑通道 + 主程序
./scripts/build-all.sh

# 等价于:
# 1. ./channels-src/telegram/build.sh  (如果目录存在)
# 2. cargo build --release
```

---

## 五、方式二：Docker 构建（推荐用于生产部署）

### 5.1 安装 Docker（RHEL/CentOS 9）

```bash
# 安装 Docker CE
sudo dnf config-manager --add-repo https://download.docker.com/linux/centos/docker-ce.repo
sudo dnf install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin

# 启动 Docker 并设置开机自启
sudo systemctl start docker
sudo systemctl enable docker

# 将当前用户加入 docker 组（免 sudo）
sudo usermod -aG docker $USER
# 需要重新登录 shell 生效
```

### 5.2 主程序镜像

```bash
# 构建主程序镜像（多阶段构建，注意平台为 arm64）
docker build --platform linux/arm64 -t ironclaw:latest .

# 运行
docker run --env-file .env -p 3000:3000 ironclaw:latest
```

**Dockerfile 多阶段构建流程:**
```
Stage 1 (builder): rust:1.92-slim-bookworm
  ├── 安装: pkg-config, libssl-dev, cmake, gcc, g++
  ├── rustup target add wasm32-wasip2
  ├── cargo install wasm-tools
  └── cargo build --release --bin ironclaw

Stage 2 (runtime): debian:bookworm-slim
  ├── 安装: ca-certificates, libssl3
  ├── 复制: ironclaw 二进制 + migrations/
  ├── 非 root 用户 (uid=1000)
  └── EXPOSE 3000
```

### 5.3 Worker 镜像（沙箱执行）

```bash
docker build -f Dockerfile.worker -t ironclaw-worker .
```

Worker 镜像包含丰富的开发工具：
- Rust 1.92, Node.js, Python3, Git, GitHub CLI
- Claude Code CLI (`@anthropic-ai/claude-code`)
- 非 root 用户 `sandbox` (uid=1000)

### 5.4 Sandbox 镜像（WASM 工具构建）

```bash
docker build -f docker/sandbox.Dockerfile -t ironclaw-sandbox .
```

轻量级镜像，仅包含 WASM 编译工具链。

### 5.5 docker-compose（本地开发数据库）

```bash
# 启动 PostgreSQL + pgvector（仅数据库，不含主程序）
docker compose up -d

# 数据库连接信息:
# host: localhost:5432
# db:   ironclaw
# user: ironclaw
# pass: ironclaw (仅开发用)
```

`docker-compose.yml` 使用 `pgvector/pgvector:pg16` 镜像，自动包含 pgvector 扩展。

---

## 六、方式三：预编译二进制

### 6.1 Shell 脚本安装

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/nearai/ironclaw/releases/latest/download/ironclaw-installer.sh | sh
```

### 6.2 当前服务器对应的 Target Triple

| 平台 | Target Triple |
|------|--------------|
| **Linux (ARM64)** ← 当前服务器 | `aarch64-unknown-linux-gnu` |
| Linux (ARM64 musl) | `aarch64-unknown-linux-musl` |
| Linux (x86_64) | `x86_64-unknown-linux-gnu` |
| Linux (x86_64 musl) | `x86_64-unknown-linux-musl` |

> 当前服务器架构为 `aarch64`，对应 target 为 `aarch64-unknown-linux-gnu`。

---

## 七、Feature Flags

Cargo.toml 中定义了以下 feature flags：

| Feature | 默认启用 | 说明 |
|---------|---------|------|
| `postgres` | ✅ | PostgreSQL 数据库支持（deadpool-postgres, pgvector, refinery 迁移） |
| `libsql` | ✅ | libSQL/Turso 嵌入式数据库支持 |
| `html-to-markdown` | ✅ | HTML 转 Markdown 功能 |
| `bedrock` | ❌ | AWS Bedrock LLM 支持（需要 CMake） |
| `integration` | ❌ | 重型集成测试（CI 专用） |
| `import` | ❌ | OpenClaw 数据导入（json5 + libsql） |

```bash
# 默认编译（postgres + libsql + html-to-markdown）
cargo build

# 启用 AWS Bedrock
cargo build --features bedrock

# 仅 PostgreSQL（不含 libsql）
cargo build --no-default-features --features postgres,html-to-markdown
```

---

## 八、首次运行配置

### 8.1 Onboard 向导

```bash
ironclaw onboard
```

向导会引导完成：
1. **数据库连接** — 配置 `DATABASE_URL`
2. **NEAR AI 认证** — 浏览器 OAuth（GitHub/Google）
3. **密钥加密** — 使用 Linux Secret Service（如 gnome-keyring）
4. **通道安装** — 从 `channels-src/` 安装已编译的 WASM 通道

配置持久化到数据库；引导变量写入 `~/.ironclaw/.env`。

> **Linux 无头服务器注意**: 如果服务器没有图形界面，密钥加密可能需要额外配置 `gnome-keyring` 或使用环境变量方式。

### 8.2 环境变量配置

核心环境变量（参考 `.env.example`）：

```env
# 数据库（根据实际安装方式调整）
DATABASE_URL=postgres://ironclaw@localhost/ironclaw

# LLM 提供商（默认 nearai）
# LLM_BACKEND=nearai|anthropic|openai|ollama|github_copilot|gemini_oauth|openai_compatible|minimax|bedrock

# Agent 设置
AGENT_NAME=ironclaw
AGENT_MAX_PARALLEL_JOBS=5
AGENT_USE_PLANNING=true

# 日志
RUST_LOG=ironclaw=debug
```

### 8.3 LLM 提供商选择

| 提供商 | 配置方式 | 适合场景 |
|--------|---------|---------|
| **NEAR AI** (默认) | OAuth 浏览器登录 | 开箱即用 |
| **Ollama** | 本地运行，无需 API Key | 离线/隐私优先 |
| **Anthropic** | `ANTHROPIC_API_KEY` | Claude 模型 |
| **OpenAI** | `OPENAI_API_KEY` | GPT 模型 |
| **GitHub Copilot** | IDE OAuth token | 已有 Copilot 订阅 |
| **OpenRouter** | `openai_compatible` + API Key | 300+ 模型 |
| **Gemini** | OAuth 浏览器登录 | Google 模型 |

详见 `docs/LLM_PROVIDERS.md`。

---

## 九、启动运行

```bash
# 交互式 REPL
cargo run

# 或使用编译好的二进制
./target/release/ironclaw

# 带调试日志
RUST_LOG=ironclaw=debug cargo run

# 后台运行（生产部署）
nohup ./target/release/ironclaw > ironclaw.log 2>&1 &

# 或使用 systemd 管理（推荐生产环境）
# 参见下方 systemd 服务配置
```

### 9.1 systemd 服务配置（可选，推荐生产环境）

```bash
# 创建 systemd 服务文件
sudo tee /etc/systemd/system/ironclaw.service << 'EOF'
[Unit]
Description=Ironclaw AI Assistant
After=network.target postgresql-17.service

[Service]
Type=simple
User=ironclaw
WorkingDirectory=/home/ironclaw
ExecStart=/home/ironclaw/ironclaw/target/release/ironclaw
EnvironmentFile=/home/ironclaw/.ironclaw/.env
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

# 启用并启动服务
sudo systemctl daemon-reload
sudo systemctl enable ironclaw
sudo systemctl start ironclaw

# 查看日志
sudo journalctl -u ironclaw -f
```

---

## 十、开发命令

```bash
# 代码格式化
cargo fmt

# Lint 检查
cargo clippy --all --benches --tests --examples --all-features

# 运行测试（需要先创建测试数据库）
sudo -u postgres createdb ironclaw_test
cargo test

# 运行特定测试
cargo test test_name
```

---

## 十一、项目目录结构（构建相关）

```
ironclaw/
├── Cargo.toml              # 主工程配置 (workspace)
├── Cargo.lock              # 依赖锁定文件
├── build.rs                # 编译时构建脚本（WASM通道 + registry catalog）
├── .env.example            # 环境变量模板
├── Dockerfile              # 主程序 Docker 镜像
├── Dockerfile.worker       # Worker 沙箱 Docker 镜像
├── Dockerfile.test         # 测试 Docker 镜像
├── docker-compose.yml      # 本地开发 PostgreSQL
├── docker/
│   └── sandbox.Dockerfile  # WASM 构建沙箱镜像
├── scripts/
│   └── build-all.sh        # 完整构建脚本（通道 + 主程序）
├── src/                    # 主程序源码
├── crates/
│   ├── ironclaw_common/    # 共享类型库
│   └── ironclaw_safety/    # 安全/消毒库
├── channels-src/           # WASM 通道源码
│   ├── telegram/           # Telegram 通道（build.rs 自动编译）
│   ├── slack/
│   ├── discord/
│   ├── whatsapp/
│   └── feishu/
├── tools-src/              # WASM 工具源码
│   ├── github/
│   ├── gmail/
│   └── ...
├── registry/               # 内置注册表清单（编译时嵌入）
│   ├── tools/*.json
│   ├── channels/*.json
│   ├── mcp-servers/*.json
│   └── _bundles.json
├── migrations/             # PostgreSQL 数据库迁移（V1~V13）
├── wit/                    # WASM Interface Types 定义
│   └── channel.wit
└── providers.json          # LLM 提供商配置
```

---

## 十二、Linux 部署完整流程速查

以下是在全新 RHEL/CentOS 9 aarch64 服务器上从零部署的完整步骤：

```bash
# ===== 1. 系统依赖 =====
sudo dnf groupinstall -y "Development Tools"
sudo dnf install -y gcc gcc-c++ make pkg-config openssl-devel perl-core cmake git curl

# ===== 2. 安装 Rust =====
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
. "$HOME/.cargo/env"
echo '. "$HOME/.cargo/env"' >> ~/.bashrc
rustup target add wasm32-wasip2
cargo install wasm-tools

# ===== 3. 安装 PostgreSQL 17 + pgvector =====
sudo dnf install -y https://download.postgresql.org/pub/repos/yum/reporpms/EL-9-aarch64/pgdg-redhat-repo-latest.noarch.rpm
sudo dnf -qy module disable postgresql
sudo dnf install -y postgresql17-server postgresql17-devel
sudo /usr/pgsql-17/bin/postgresql-17-setup initdb
sudo systemctl start postgresql-17
sudo systemctl enable postgresql-17
# pgvector: sudo dnf install -y pgvector_17 (或从源码编译)

# ===== 4. 创建数据库 =====
sudo -u postgres createuser $USER --createdb
sudo -u postgres createdb ironclaw -O $USER
sudo -u postgres psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"

# ===== 5. 获取源码并编译 =====
git clone https://github.com/nearai/ironclaw.git
cd ironclaw
cargo build --release

# ===== 6. 首次配置 =====
./target/release/ironclaw onboard

# ===== 7. 启动 =====
RUST_LOG=ironclaw=debug ./target/release/ironclaw
```

---

## 十三、常见问题

### Q: 首次编译很慢？
A: 正常。Ironclaw 依赖约 400+ crate（包括 wasmtime、cranelift 等大型依赖），首次编译需要 10-20 分钟。后续增量编译会快很多。

### Q: build.rs 报 Telegram WASM 构建失败？
A: 确保已安装 `wasm32-wasip2` target 和 `wasm-tools`：
```bash
rustup target add wasm32-wasip2
cargo install wasm-tools
```
即使失败，主程序仍可编译，只是 Telegram 通道不可用。

### Q: 编译时报 openssl 相关错误？
A: 确保已安装 `openssl-devel`：
```bash
sudo dnf install -y openssl-devel pkg-config
```

### Q: 编译时报 linker 'cc' not found？
A: 确保已安装编译工具链：
```bash
sudo dnf groupinstall -y "Development Tools"
sudo dnf install -y gcc gcc-c++
```

### Q: 不想安装 PostgreSQL？
A: 可以用 `docker compose up -d` 启动容器化的 PostgreSQL，或使用 libsql feature 作为嵌入式数据库替代。

### Q: Release 编译优化选项？
A: `profile.release` 配置了 `strip = true`（去除调试符号）。`profile.dist`（用于发布）额外启用了 `lto = "fat"` 和 `codegen-units = 1` 以获得最大优化。

### Q: 防火墙需要开放哪些端口？
A: Ironclaw 默认监听 3000 端口：
```bash
sudo firewall-cmd --permanent --add-port=3000/tcp
sudo firewall-cmd --reload
```
