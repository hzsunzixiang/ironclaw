# mini-agent-loop Debug Toolkit

本目录包含 mini-agent-loop 项目的 **LLDB 自动化调试工具集**。

通过预设的 14 个断点 + Python 回调，可以在不进入 LLDB 交互界面的情况下，
自动追踪 Agent Loop 的完整执行流程，并以彩色格式输出关键变量信息。

---

## 📁 文件说明

| 文件 | 说明 |
|------|------|
| `debug_run.sh` | 一键启动脚本（推荐入口） |
| `lldbinit` | LLDB 初始化脚本，定义 14 个断点 |
| `debug_breakpoints.py` | Python 断点回调，负责格式化输出 |
| `LLDB_DEBUG_GUIDE.md` | 详细的 LLDB 调试指南 |
| `logs/` | 自动生成的调试日志目录 |

---

## 🚀 快速开始

### 前置条件

```bash
# 确保已编译 debug 版本
cd /path/to/mini-agent-loop
cargo build
```

### 最简单的用法：一条命令

```bash
# 从项目根目录运行
./debug/debug_run.sh "What is 42 + 58?"
```

这会：
1. 自动创建输入文件（包含你的问题 + `quit`）
2. 后台启动 LLDB batch 模式
3. **前台实时流式输出调试日志**
4. 结束后日志保存在 `debug/logs/` 目录

---

## 📖 所有运行模式

### 1. 自动模式（默认，推荐）

```bash
./debug/debug_run.sh "What is 42 + 58?"
```

后台运行 + 前台实时看输出，结束后自动退出。

### 2. 纯后台模式

```bash
./debug/debug_run.sh --bg "Calculate 100 * 3.14"
```

完全后台运行，不占终端。随时查看日志：

```bash
tail -f debug/logs/debug_*.log
```

### 3. 文件输入模式

```bash
# 先创建输入文件（支持多轮对话）
echo -e "What is 1+1?\nWhat is 2+2?\nquit" > input.txt

# 运行
./debug/debug_run.sh --input input.txt
```

### 4. 管道输入模式

```bash
echo -e "Hello\nquit" | ./debug/debug_run.sh --stdin
```

### 5. 交互式模式（传统 LLDB）

```bash
./debug/debug_run.sh --interactive
```

进入 LLDB 交互界面，手动输入 `run` 启动。

### 6. 交互式 + 日志

```bash
./debug/debug_run.sh --log
```

### 7. 查看帮助

```bash
./debug/debug_run.sh --help
```

---

## 🔍 断点列表

共 14 个断点，覆盖 Agent Loop 的完整流程：

| # | 位置 | 说明 | 监控变量 |
|---|------|------|----------|
| BP1 | `main.rs:164` | 用户输入捕获 | `input` |
| BP2 | `main.rs:189` | 构建对话上下文 | `current_model` |
| BP3 | `main.rs:192` | 进入 Agent Loop | `messages` |
| BP4 | `agent.rs:68` | Agent Loop 入口 | `config`, `messages` |
| BP5 | `agent.rs:75` | LLM 调用前 | `iteration`, `messages`, `tool_defs` |
| BP6 | `agent.rs:77` | LLM 响应返回 | `response` |
| BP7 | `agent.rs:80` | 文本响应分支 | `text` |
| BP8 | `agent.rs:85` | 工具调用分支 | `tool_calls`, `content` |
| BP9 | `execute_tool_with_safety` | 工具执行 | `tool_name`, `params` |
| BP10 | `process_tool_result` | 工具结果 | `tool_name`, `tool_call_id`, `result` |
| BP11 | `calculator.rs:48` | 计算器工具 | `params` |
| BP12 | `openai.rs:275` | OpenAI 请求 | `self`, `messages`, `tools` |
| BP13 | `openai.rs:306` | OpenAI 响应解析 | `status` |
| BP14 | `main.rs:193` | 最终响应 | `text` |

---

## 📊 输出示例

运行后你会看到类似这样的彩色输出：

```
============================================================
[BP1: USER INPUT  (main.rs:164)]
  input = "What is 42 + 58?"
============================================================

============================================================
[BP5: BEFORE LLM CALL  (agent.rs:75)]
  iteration = 1
  *messages = { buf = ..., len = 2 }
  tool_defs = { buf = ..., len = 1 }
============================================================

============================================================
[BP8: LLM TOOL CALLS  (agent.rs:85)]
  tool_calls = [...]
  content = <not in scope>
============================================================

============================================================
[BP11: CALCULATOR TOOL  (calculator.rs:48)]
  params = {"expression": "42 + 58"}
============================================================

============================================================
[BP7: LLM TEXT RESPONSE  (agent.rs:80)]
  text = "42 + 58 = 100"
============================================================
```

---

## 🛠 手动使用 LLDB（不通过脚本）

如果你想直接使用 LLDB，从**项目根目录**运行：

```bash
# 加载 lldbinit 并启动
rust-lldb -s debug/lldbinit target/debug/mini-agent-loop

# 在 LLDB 中输入 run 启动
(lldb) run
```

或者在已有的 LLDB 会话中加载：

```bash
(lldb) command source debug/lldbinit
```

---

## 📝 日志管理

日志自动保存在 `debug/logs/` 目录，文件名格式：`debug_YYYYMMDD_HHMMSS.log`

```bash
# 查看最新日志
ls -lt debug/logs/ | head -5

# 清理旧日志
rm debug/logs/debug_*.log
```

---

## ⚠️ 注意事项

1. **必须使用 debug 构建**：`cargo build`（不是 `cargo build --release`）
2. **断点行号**：如果修改了源码，断点行号可能需要更新（编辑 `lldbinit`）
3. **LLDB 版本**：优先使用 `rust-lldb`（Rust 工具链自带），回退到系统 `lldb`
4. **Rust 变量可见性**：LLDB 对 Rust 变量的展示有限，复杂类型可能只显示内部结构
5. **环境变量**：确保 `OPENAI_API_KEY` 等环境变量已设置（程序运行需要）
