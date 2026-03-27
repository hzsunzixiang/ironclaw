# IronClaw Browser Use 方案指南

IronClaw **目前没有内置 Browser Use 工具**（即 Agent 自主控制浏览器浏览网页、点击、截图等能力），但提供了以下替代方案。

---

## 现有的 Web 相关能力

IronClaw 内置了以下与"上网"相关的工具：

| 工具 | 能力 | 局限 |
|------|------|------|
| **web-search** | Brave 搜索 API，返回搜索结果（标题、URL、摘要） | 只返回搜索结果，不能浏览页面 |
| **llm-context** | Brave LLM Context API，返回网页预提取内容（文本、表格、代码） | 依赖 Brave 的预处理，不是真正的浏览器渲染 |
| **http** (内置) | HTTP 请求工具，支持 GET/POST 等，可选 HTML→Markdown 转换 | 只能获取静态 HTML，无法处理 JS 渲染的页面 |

这些工具可以满足**信息检索**需求，但无法实现真正的 **Browser Use**（即 Agent 像人一样操作浏览器）。

---

## 方案 1：通过 MCP 接入 Playwright/Puppeteer Browser（推荐 ⭐）

IronClaw 原生支持 MCP Server，可以接入社区的 Browser Use MCP Server。

### 安装 Playwright MCP

```bash
# 安装 Node.js（如果没有）
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo bash -
sudo apt-get install -y nodejs

# 安装 Playwright MCP Server
npm install -g @anthropic/mcp-playwright

# 安装 Chromium 及系统依赖
npx playwright install --with-deps chromium
```

### 注册到 IronClaw

```bash
ironclaw mcp add browser-use npx @anthropic/mcp-playwright \
  --transport stdio
```

### 或者使用 Puppeteer MCP

```bash
npm install -g @anthropic/mcp-puppeteer

ironclaw mcp add puppeteer npx @anthropic/mcp-puppeteer \
  --transport stdio
```

### 验证

```bash
ironclaw mcp test browser-use
ironclaw mcp list
```

### 使用示例

在对话中直接让 Agent 浏览网页：

```
请打开 https://news.ycombinator.com 并告诉我今天的热门文章
```

Agent 将能使用 navigate、click、screenshot、type 等浏览器操作工具。

---

## 方案 2：使用 Browserbase 等云端浏览器服务

如果服务器是无头环境（没有 GUI），或不想在服务器上安装浏览器，可以使用云端浏览器服务。

### 配置 Browserbase MCP

```bash
ironclaw mcp add browserbase https://mcp.browserbase.com \
  --transport http \
  --header "Authorization: Bearer YOUR_BROWSERBASE_API_KEY"
```

### 优势

- 无需在服务器上安装浏览器和依赖
- 云端渲染，不消耗本地资源
- 支持并发多个浏览器会话

### 劣势

- 需要注册 Browserbase 账号并付费
- 依赖外部网络连接

---

## 方案 3：直接用 HTTP 工具 + HTML→Markdown（轻量替代）

如果只需要 Agent 能"读网页"而不需要交互操作（点击、填表等），IronClaw 的内置 `http` 工具已经支持 HTML 转 Markdown。

### 使用方式

在对话中直接请求：

```
请帮我获取 https://example.com 的内容
```

Agent 会使用 `http` 工具 GET 页面，自动将 HTML 转为 Markdown 返回。

### 适用场景

- 读取静态网页内容
- 抓取 API 返回的 JSON 数据
- 下载文件

### 不适用场景

- 需要 JavaScript 渲染的 SPA 页面
- 需要登录后才能访问的页面
- 需要点击、滚动等交互操作

---

## 注意事项

1. **无头服务器**：CentOS 服务器通常没有 GUI，需确保 Playwright 以 `headless: true` 模式运行（MCP Server 通常默认如此）
2. **系统依赖**：Playwright Chromium 需要安装系统级依赖库（`npx playwright install --with-deps chromium` 会自动处理）
3. **资源消耗**：浏览器进程比较吃内存，建议服务器至少有 **2GB 可用内存**
4. **安全性**：Browser Use 意味着 Agent 可以访问任意网页，注意配合 IronClaw 的 approval 机制控制权限
5. **网络环境**：如果服务器在国内，部分国外网站可能无法直接访问

## 方案对比

| 维度 | 方案 1 (Playwright MCP) | 方案 2 (云端浏览器) | 方案 3 (HTTP 工具) |
|------|------------------------|--------------------|--------------------|
| 安装复杂度 | 中等（需装 Node + Chromium） | 低（只需 API Key） | 无需安装 |
| 资源消耗 | 高（本地跑浏览器） | 低（云端运行） | 极低 |
| JS 渲染支持 | ✅ | ✅ | ❌ |
| 交互操作 | ✅ 点击/填表/截图 | ✅ 点击/填表/截图 | ❌ 只能 GET/POST |
| 费用 | 免费 | 付费 | 免费 |
| 推荐场景 | 通用 Browser Use | 无头服务器/高并发 | 只需读取网页内容 |
