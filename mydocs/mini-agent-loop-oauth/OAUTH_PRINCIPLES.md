# OAuth 2.0 原理说明 — 从 RFC 到代码实现

> 本文档结合 10 份 OAuth 相关 RFC 和 mini-agent-loop-oauth 工程代码，
> 系统阐述 OAuth 2.0 Authorization Code + PKCE 流程的原理、安全设计和实现映射。

---

## 目录

1. [概述：为什么需要 OAuth](#1-概述为什么需要-oauth)
2. [核心 RFC 全景图](#2-核心-rfc-全景图)
3. [Authorization Code Grant 流程详解](#3-authorization-code-grant-流程详解)
4. [PKCE 原理与实现](#4-pkce-原理与实现)
5. [本地回调服务器](#5-本地回调服务器)
6. [Token 生命周期管理](#6-token-生命周期管理)
7. [安全设计：从 RFC 到代码的防御体系](#7-安全设计从-rfc-到代码的防御体系)
8. [代码架构与 RFC 映射表](#8-代码架构与-rfc-映射表)
9. [与 IronClaw 的对照](#9-与-ironclaw-的对照)
10. [附录：RFC 速查表](#10-附录rfc-速查表)

---

## 1. 概述：为什么需要 OAuth

OAuth 2.0 解决的核心问题是：**如何让第三方应用在不获取用户密码的情况下，安全地访问用户在其他服务上的资源。**

```
传统方式（不安全）：
  用户 → 把 GitHub 密码告诉 Agent → Agent 用密码登录 GitHub

OAuth 方式（安全）：
  用户 → 在 GitHub 页面上点"授权" → Agent 获得一个有限权限的 token
```

RFC 6749 (The OAuth 2.0 Authorization Framework) 的 Abstract 这样定义：

> *"The OAuth 2.0 authorization framework enables a third-party application to obtain
> limited access to an HTTP service, either on behalf of a resource owner by
> orchestrating an approval interaction between the resource owner and the HTTP
> service, or by allowing the third-party application to obtain access on its own behalf."*

在本工程中，Agent 需要调用外部工具（如 GitHub API、Google API），这些工具需要用户授权。
OAuth 2.0 提供了标准化的授权流程，让 Agent 能安全地获取和管理访问令牌。

---

## 2. 核心 RFC 全景图

本工程涉及的 10 份 RFC 构成了一个完整的 OAuth 2.0 安全体系：

```
┌─────────────────────────────────────────────────────────────────┐
│                    OAuth 2.0 RFC 体系                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────┐   核心框架                                        │
│  │ RFC 6749 │ ← OAuth 2.0 Authorization Framework              │
│  └────┬─────┘   定义四种授权模式、角色、端点                      │
│       │                                                         │
│  ┌────┴─────┐   令牌使用                                        │
│  │ RFC 6750 │ ← Bearer Token Usage                              │
│  └──────────┘   定义 Authorization: Bearer <token> 头格式        │
│                                                                 │
│  ┌──────────┐   安全增强                                        │
│  │ RFC 7636 │ ← PKCE (Proof Key for Code Exchange)              │
│  └──────────┘   防止授权码拦截攻击（本工程核心）                   │
│                                                                 │
│  ┌──────────┐   令牌管理                                        │
│  │ RFC 7009 │ ← Token Revocation                                │
│  │ RFC 7662 │ ← Token Introspection                             │
│  └──────────┘                                                   │
│                                                                 │
│  ┌──────────┐   服务发现                                        │
│  │ RFC 7591 │ ← Dynamic Client Registration                     │
│  │ RFC 8414 │ ← Authorization Server Metadata                   │
│  └──────────┘                                                   │
│                                                                 │
│  ┌──────────┐   资源指示                                        │
│  │ RFC 8707 │ ← Resource Indicators                             │
│  └──────────┘                                                   │
│                                                                 │
│  ┌──────────┐   安全规范                                        │
│  │ RFC 6819 │ ← Threat Model & Security Considerations          │
│  │ RFC 9700 │ ← Security Best Current Practice (BCP)            │
│  └──────────┘   ← 本工程安全设计的主要依据                       │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 各 RFC 在本工程中的角色

| RFC | 标题 | 本工程使用 | 对应代码 |
|-----|------|-----------|---------|
| **6749** | OAuth 2.0 核心框架 | Authorization Code Grant 全流程 | `oauth/mod.rs` |
| **6750** | Bearer Token 使用 | Token 格式和传输 | `oauth/token.rs` |
| **7636** | PKCE | S256 挑战/验证器 | `oauth/pkce.rs` |
| **7009** | Token 撤销 | `/revoke` 命令设计参考 | `main.rs` |
| **7591** | 动态客户端注册 | Provider 配置模式参考 | `oauth/mod.rs` |
| **7662** | Token 内省 | Token 有效性检查参考 | `oauth/token.rs` |
| **8414** | 授权服务器元数据 | 端点发现模式参考 | `oauth/mod.rs` |
| **8707** | 资源指示器 | IronClaw MCP OAuth 使用 | (参考) |
| **6819** | 威胁模型 | 安全设计依据 | 全局 |
| **9700** | 安全最佳实践 | PKCE 强制、state 必须 | `oauth/pkce.rs` |

---

## 3. Authorization Code Grant 流程详解

### 3.1 RFC 6749 §4.1 定义的流程

RFC 6749 定义了四种授权模式，本工程使用最安全的 **Authorization Code Grant**：

```
     +----------+
     | Resource |
     |   Owner  |        RFC 6749 Figure 3: Authorization Code Flow
     +----------+
          ^
          |
         (B) 用户在浏览器中授权
     +----|-----+          Client Identifier      +---------------+
     |         -+----(A)-- & Redirection URI ---->|               |
     |  User-   |                                 | Authorization |
     |  Agent  -+----(B)-- User authenticates --->|     Server    |
     |          |                                 |               |
     |         -+----(C)-- Authorization Code ---<|               |
     +-|----|---+                                 +---------------+
       |    |                                         ^      v
      (A)  (C)                                        |      |
       |    |                                         |      |
       ^    v                                         |      |
     +---------+                                      |      |
     |         |>---(D)-- Authorization Code ---------'      |
     |  Client |          & Redirection URI                  |
     |         |                                             |
     |         |<---(E)----- Access Token -------------------'
     +---------+       (w/ Optional Refresh Token)
```

### 3.2 本工程的具体实现

将 RFC 的抽象流程映射到代码中的 `run_oauth_flow()` 函数：

```
                        本工程实现流程
                        
  ┌─────────────┐                              ┌──────────────────┐
  │ Mini Agent  │                              │  Authorization   │
  │   (Client)  │                              │  Server (GitHub/ │
  │             │                              │  Google/...)     │
  └──────┬──────┘                              └────────┬─────────┘
         │                                              │
    ①  build_auth_url()                                 │
    │   生成 PKCE challenge + state                     │
    │   构建授权 URL                                     │
         │                                              │
    ②  bind_callback_listener()                         │
    │   在 127.0.0.1:9876 启动监听                       │
         │                                              │
    ③  open_browser(url)                                │
    │   打开浏览器 ─────────────────────────────────────→│
    │                                                   │
    │              用户在浏览器中登录并授权                  │
    │                                                   │
    ④  wait_for_callback()                              │
    │   ←── GET /callback?code=xxx&state=yyy ───────────│
    │   验证 state 参数                                  │
    │   提取 authorization code                          │
         │                                              │
    ⑤  exchange_code()                                  │
    │   POST /token ────────────────────────────────────→│
    │   grant_type=authorization_code                   │
    │   + code + code_verifier                          │
    │   ←── { access_token, refresh_token } ────────────│
         │                                              │
    ⑥  store.store()                                    │
    │   保存 token 到 ~/.mini-agent/tokens.json          │
         │                                              │
    ✅  完成                                             │
```

### 3.3 代码对照

**Step ①: 构建授权 URL** — `oauth/mod.rs` → `build_auth_url()`

RFC 6749 §4.1.1 要求的参数：

```
Authorization Request 必须包含:
  response_type=code     (REQUIRED)  ← 指定使用 Authorization Code 模式
  client_id              (REQUIRED)  ← 标识客户端身份
  redirect_uri           (OPTIONAL)  ← 回调地址
  scope                  (OPTIONAL)  ← 请求的权限范围
  state                  (RECOMMENDED) ← CSRF 防护
```

代码实现：

```rust
// oauth/mod.rs — build_auth_url()
let mut url = format!(
    "{}?response_type=code&client_id={}&redirect_uri={}&state={}",
    provider.authorization_url,
    urlencoding::encode(&provider.client_id),   // RFC 6749 §4.1.1: client_id REQUIRED
    urlencoding::encode(&redirect_uri),          // RFC 6749 §4.1.1: redirect_uri
    urlencoding::encode(&state),                 // RFC 6749 §10.12: state for CSRF
);

// RFC 6749 §3.3: scope
if !provider.scopes.is_empty() {
    url.push_str(&format!("&scope={}", urlencoding::encode(&provider.scopes.join(" "))));
}

// RFC 7636 §4.3: PKCE code_challenge
if let Some(ref pkce) = pkce {
    url.push_str(&format!(
        "&code_challenge={}&code_challenge_method=S256",
        pkce.challenge
    ));
}
```

**Step ⑤: 交换令牌** — `oauth/mod.rs` → `exchange_code()`

RFC 6749 §4.1.3 要求的参数：

```
Token Request 必须包含:
  grant_type=authorization_code  (REQUIRED)
  code                           (REQUIRED)  ← 从回调中获取的授权码
  redirect_uri                   (REQUIRED)  ← 必须与授权请求中的一致
  client_id                      (REQUIRED)  ← 公开客户端在 body 中发送
```

代码实现：

```rust
// oauth/mod.rs — exchange_code()
let mut params = vec![
    ("grant_type", "authorization_code".to_string()),  // RFC 6749 §4.1.3
    ("code", code.to_string()),                         // 授权码
    ("redirect_uri", redirect_uri),                     // 必须匹配
];

if let Some(verifier) = code_verifier {
    params.push(("code_verifier", verifier.to_string())); // RFC 7636 §4.5
}

// RFC 6749 §2.3.1: 客户端认证
if let Some(ref secret) = provider.client_secret {
    request = request.basic_auth(&provider.client_id, Some(secret)); // 机密客户端
} else {
    params.push(("client_id", provider.client_id.clone()));          // 公开客户端
}
```

---

## 4. PKCE 原理与实现

### 4.1 为什么需要 PKCE

PKCE (Proof Key for Code Exchange, 发音 "pixy") 由 RFC 7636 定义，解决的核心问题是：

> **授权码拦截攻击 (Authorization Code Interception Attack)**

在原始的 OAuth 2.0 流程中，授权码通过浏览器重定向传递。对于本地应用（Native App），
攻击者可能通过以下方式拦截授权码：

```
正常流程:
  Authorization Server → 302 Redirect → http://127.0.0.1:9876/callback?code=abc123
                                         ↓
                                    我们的 Agent 收到 code

攻击场景 (无 PKCE):
  Authorization Server → 302 Redirect → http://127.0.0.1:9876/callback?code=abc123
                                         ↓
                                    恶意应用也监听了同一端口，抢先收到 code
                                    恶意应用用 code 换取 access_token ← 攻击成功！
```

RFC 9700 §2.1.1 明确要求：

> *"Public clients MUST use PKCE [RFC7636] to this end"*
> *"For confidential clients, the use of PKCE [RFC7636] is RECOMMENDED"*
> *"Authorization servers MUST support PKCE [RFC7636]."*

### 4.2 PKCE 的密码学原理

PKCE 的核心思想是：**用一个只有合法客户端知道的秘密，将授权请求和令牌交换请求绑定在一起。**

```
RFC 7636 Figure 2: Abstract Protocol Flow

                                              +-------------------+
                                              |   Authz Server    |
    +--------+                                | +---------------+ |
    |        |--(A)- Authorization Request ---->|               | |
    |        |       + t(code_verifier), t_m  | | Authorization | |
    |        |                                | |    Endpoint   | |
    |        |<-(B)---- Authorization Code -----|               | |
    |        |                                | +---------------+ |
    | Client |                                |                   |
    |        |                                | +---------------+ |
    |        |--(C)-- Access Token Request ---->|               | |
    |        |          + code_verifier       | |    Token      | |
    |        |                                | |   Endpoint    | |
    |        |<-(D)------ Access Token ---------|               | |
    +--------+                                | +---------------+ |
                                              +-------------------+
```

数学过程：

```
1. 客户端生成随机数:
   code_verifier = BASE64URL(random_bytes(32))
   → 43 个字符的高熵随机字符串

2. 客户端计算哈希:
   code_challenge = BASE64URL(SHA-256(code_verifier))
   → 43 个字符的哈希值

3. 授权请求发送 code_challenge (哈希值，不可逆)
4. 令牌交换发送 code_verifier (原始值)
5. 服务器验证: SHA-256(code_verifier) == code_challenge ?
```

**为什么安全？**

- 攻击者即使拦截了授权码和 `code_challenge`，也无法反推出 `code_verifier`
  （SHA-256 是单向函数）
- 没有 `code_verifier`，攻击者无法完成令牌交换
- `code_verifier` 通过 TLS 直接发送到令牌端点，不经过浏览器

### 4.3 代码实现

`oauth/pkce.rs` — `PkceChallenge::generate()`

```rust
pub fn generate() -> Self {
    // RFC 7636 §4.1: 生成 32 字节随机数作为 code_verifier
    let mut verifier_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut verifier_bytes);  // 使用操作系统 CSPRNG
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes); // → 43 chars

    // RFC 7636 §4.2: code_challenge = BASE64URL(SHA256(code_verifier))
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize()); // → 43 chars

    Self { verifier, challenge }
}
```

关键设计决策：

| 决策 | RFC 依据 | 代码实现 |
|------|---------|---------|
| 使用 S256 而非 plain | RFC 9700: *"clients SHOULD use PKCE code challenge methods that do not expose the PKCE verifier"* | `code_challenge_method=S256` |
| 32 字节随机数 | RFC 7636 §4.1: verifier 43-128 chars | `[0u8; 32]` → 43 base64url chars |
| 使用 OS CSPRNG | RFC 7636 §7.1: *"high entropy"* | `rand::rngs::OsRng` |
| Base64url-no-pad 编码 | RFC 7636 §Appendix A | `URL_SAFE_NO_PAD.encode()` |

### 4.4 PKCE 防御效果图

```
有 PKCE 的情况下:

  ① Agent 生成: verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
                challenge = SHA256(verifier) = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"

  ② 授权请求: ...&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM
                                  ↑ 攻击者可以看到这个（但无法反推 verifier）

  ③ 授权码回调: ?code=abc123
                 ↑ 假设攻击者拦截了这个

  ④ 攻击者尝试令牌交换:
     POST /token
       code=abc123
       code_verifier=???  ← 攻击者不知道 verifier！
                             SHA-256 不可逆，无法从 challenge 推导
     → 服务器拒绝: code_verifier 验证失败 ❌

  ⑤ 合法 Agent 令牌交换:
     POST /token
       code=abc123
       code_verifier=dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk
     → 服务器验证: SHA256(verifier) == challenge ✅
     → 返回 access_token
```

---

## 5. 本地回调服务器

### 5.1 为什么需要本地服务器

OAuth 2.0 Authorization Code 流程需要一个 **redirect_uri** 来接收授权码。
对于本地应用（如我们的 CLI Agent），RFC 8252 (OAuth 2.0 for Native Apps) §7.3 规定：

> *"Native apps SHOULD use the loopback interface to receive the authorization
> response."*

这意味着我们需要在本地启动一个临时 HTTP 服务器来接收浏览器的重定向。

### 5.2 回调流程

```
  ┌──────────┐     ┌──────────────┐     ┌──────────────────┐
  │  Agent   │     │   Browser    │     │  Auth Server     │
  │ (Client) │     │ (User-Agent) │     │ (GitHub/Google)  │
  └────┬─────┘     └──────┬───────┘     └────────┬─────────┘
       │                  │                       │
  ① bind 127.0.0.1:9876  │                       │
       │                  │                       │
  ② open(auth_url) ──────→│                       │
       │                  │──── GET /authorize ──→│
       │                  │                       │
       │                  │←── Login Page ────────│
       │                  │                       │
       │                  │──── POST credentials →│
       │                  │                       │
       │                  │←── 302 Redirect ──────│
       │                  │    Location: http://127.0.0.1:9876
       │                  │              /callback?code=xxx&state=yyy
       │                  │                       │
       │←── GET /callback?code=xxx&state=yyy ─────│
       │                  │                       │
  ③ 验证 state            │                       │
  ④ 提取 code             │                       │
       │                  │                       │
       │──── 200 OK ─────→│                       │
       │    (HTML 成功页)  │                       │
       │                  │                       │
  ⑤ exchange_code() ─────────────────────────────→│
       │                  │                       │
       │←── access_token ────────────────────────│
```

### 5.3 代码实现

`oauth/callback.rs` — `bind_callback_listener()`

```rust
pub async fn bind_callback_listener() -> Result<TcpListener, CallbackError> {
    // RFC 8252 §7.3: 绑定到 loopback 接口
    let ipv4_addr = format!("127.0.0.1:{}", CALLBACK_PORT); // 端口 9876
    match TcpListener::bind(&ipv4_addr).await {
        Ok(listener) => Ok(listener),
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            Err(CallbackError::PortInUse(e.to_string())) // 快速失败
        }
        Err(_) => {
            // IPv4 不可用，回退到 IPv6 loopback
            TcpListener::bind(format!("[::1]:{}", CALLBACK_PORT)).await
                .map_err(|e| /* ... */)
        }
    }
}
```

`oauth/callback.rs` — `wait_for_callback()`

```rust
pub async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,    // CSRF 验证
    display_name: &str,      // 用于 HTML 页面
) -> Result<String, CallbackError> {
    // 5 分钟超时
    tokio::time::timeout(Duration::from_secs(300), async move {
        loop {
            let (mut socket, _) = listener.accept().await?;
            // 解析 HTTP 请求行: "GET /callback?code=xxx&state=yyy HTTP/1.1"
            // 1. 检查 error= 参数 (RFC 6749 §4.1.2.1)
            // 2. 验证 state 参数 (RFC 6749 §10.12)
            // 3. 提取 code 参数
            // 4. 返回 HTML 成功/失败页面
        }
    }).await
}
```

### 5.4 安全设计

| 安全措施 | RFC 依据 | 实现 |
|----------|---------|------|
| **Loopback-only** | RFC 8252 §7.3 | 仅绑定 `127.0.0.1`，拒绝 `0.0.0.0` |
| **固定端口** | IronClaw 约定 | 端口 9876，避免端口扫描 |
| **5 分钟超时** | 防止悬挂监听器 | `tokio::time::timeout(300s)` |
| **快速失败** | 防止并发冲突 | `AddrInUse` 立即返回错误 |
| **HTML 转义** | XSS 防护 | `html_escape()` 转义 `& < > " '` |
| **错误优先检查** | RFC 6749 §4.1.2.1 | 先检查 `error=` 再提取 `code` |
| **IPv6 回退** | 兼容性 | IPv4 失败时尝试 `[::1]` |

---

## 6. Token 生命周期管理

### 6.1 Token 类型

RFC 6749 定义了两种令牌：

```
┌─────────────────────────────────────────────────────────────┐
│                    Token 生命周期                             │
│                                                             │
│  Access Token (RFC 6749 §1.4)                               │
│  ├── 短期有效（通常 1 小时）                                   │
│  ├── 用于访问受保护资源                                       │
│  ├── 格式: Bearer token (RFC 6750)                           │
│  └── 使用: Authorization: Bearer <access_token>              │
│                                                             │
│  Refresh Token (RFC 6749 §1.5)                              │
│  ├── 长期有效（数天到数月）                                    │
│  ├── 仅用于获取新的 access token                              │
│  ├── 不发送给资源服务器                                       │
│  └── 刷新时可能被轮换 (RFC 9700 §4.13.2)                     │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 6.2 Token 刷新流程

RFC 6749 §6 定义了刷新流程：

```
  ┌────────┐                                  ┌──────────────────┐
  │ Client │                                  │  Auth Server     │
  └───┬────┘                                  └────────┬─────────┘
      │                                                │
      │  access_token 过期                               │
      │                                                │
      │  POST /token                                   │
      │    grant_type=refresh_token                     │
      │    refresh_token=<old_refresh_token>            │
      │    client_id=<id>                              │
      │ ──────────────────────────────────────────────→│
      │                                                │
      │  {                                             │
      │    "access_token": "<new_access_token>",       │
      │    "refresh_token": "<new_refresh_token>",  ← 轮换！
      │    "expires_in": 3600                          │
      │  }                                             │
      │ ←──────────────────────────────────────────────│
      │                                                │
      │  丢弃旧 refresh_token                           │
      │  保存新 token 对                                │
```

### 6.3 代码实现

`oauth/token.rs` — `TokenSet`

```rust
pub struct TokenSet {
    pub access_token: String,                    // RFC 6749 §5.1
    pub refresh_token: Option<String>,           // RFC 6749 §5.1
    pub expires_at: Option<DateTime<Utc>>,       // 从 expires_in 计算
    pub scopes: Vec<String>,                     // RFC 6749 §3.3
    pub provider: String,
}

impl TokenSet {
    /// RFC 9700: 提前 60 秒判断过期，主动刷新
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => Utc::now() + Duration::seconds(60) >= exp,
            None => false,
        }
    }
}
```

`oauth/token.rs` — `refresh_access_token()`

```rust
pub async fn refresh_access_token(
    token_url: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<TokenSet, String> {
    let mut params = vec![
        ("grant_type", "refresh_token".to_string()),     // RFC 6749 §6
        ("refresh_token", refresh_token.to_string()),
    ];
    // ... 客户端认证 (Basic auth 或 body) ...
    // ... 发送请求，解析响应 ...

    Ok(TokenSet::from_response(
        access_token,
        // RFC 9700 §4.13.2: 使用新的 refresh_token（轮换），否则保留旧的
        new_refresh.or_else(|| Some(refresh_token.to_string())),
        expires_in,
        scope,
        String::new(),
    ))
}
```

### 6.4 Token 存储安全

`oauth/token.rs` — `TokenStore::save()`

```rust
pub fn save(&self) -> Result<(), String> {
    // 存储到 ~/.mini-agent/tokens.json
    std::fs::write(&path, content)?;

    // Unix: 设置 0600 权限（仅所有者可读写）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, Permissions::from_mode(0o600));
    }
    Ok(())
}
```

| 存储方面 | 本工程 | IronClaw |
|---------|--------|----------|
| 存储位置 | `~/.mini-agent/tokens.json` | 加密 SecretsStore (系统钥匙串) |
| 文件权限 | `0600` (owner-only) | 系统钥匙串管理 |
| 加密 | 无（明文 JSON） | AES-256 加密 |
| Refresh Token | 与 access_token 一起存储 | 单独存储为 `{name}_refresh_token` |

---

## 7. 安全设计：从 RFC 到代码的防御体系

### 7.1 威胁模型 (RFC 6819 + RFC 9700)

RFC 6819 和 RFC 9700 定义了 OAuth 2.0 面临的主要威胁：

```
┌─────────────────────────────────────────────────────────────────┐
│                    威胁与防御对照                                 │
├──────────────────────┬──────────────────┬───────────────────────┤
│ 威胁                  │ RFC 依据          │ 本工程防御             │
├──────────────────────┼──────────────────┼───────────────────────┤
│ 授权码拦截            │ RFC 7636         │ PKCE S256             │
│ CSRF 攻击            │ RFC 6749 §10.12  │ state 参数验证         │
│ 授权码注入            │ RFC 9700 §4.5    │ PKCE 绑定             │
│ Token 泄露           │ RFC 6750 §5.3    │ 文件权限 0600          │
│ 重放攻击             │ RFC 6749 §10.4   │ 一次性授权码            │
│ 钓鱼攻击             │ RFC 6749 §10.11  │ Loopback redirect     │
│ XSS                 │ OWASP            │ HTML 转义              │
│ 端口抢占             │ RFC 8252 §7.3    │ 固定端口 + 快速失败     │
│ 悬挂监听器           │ 最佳实践          │ 5 分钟超时             │
│ PKCE 降级攻击        │ RFC 9700 §4.8    │ 始终使用 S256          │
│ Refresh Token 盗用   │ RFC 9700 §4.13   │ Token 轮换             │
└──────────────────────┴──────────────────┴───────────────────────┘
```

### 7.2 CSRF 防护详解

**攻击场景：**

```
攻击者构造一个恶意链接:
  https://github.com/login/oauth/authorize?
    client_id=AGENT_ID&
    redirect_uri=http://127.0.0.1:9876/callback&
    state=ATTACKER_STATE

如果用户点击了这个链接，授权码会发送到 Agent，
但 Agent 会将 token 关联到攻击者的会话。
```

**防御 (RFC 6749 §10.12)：**

```rust
// oauth/pkce.rs — generate_state()
pub fn generate_state() -> String {
    let mut state_bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut state_bytes);  // 256 位熵
    URL_SAFE_NO_PAD.encode(state_bytes)               // 不可预测
}

// oauth/callback.rs — wait_for_callback()
// 回调时严格验证:
let actual_state = params.get("state").cloned().unwrap_or_default();
if actual_state != expected_state {
    return Err(CallbackError::StateMismatch { expected, actual });
}
```

### 7.3 客户端认证 (RFC 6749 §2.3)

RFC 6749 定义了两类客户端：

```
┌─────────────────────────────────────────────────────────────┐
│  Confidential Client (机密客户端)                            │
│  ├── 有 client_secret                                       │
│  ├── 认证方式: HTTP Basic Auth                               │
│  │   Authorization: Basic base64(client_id:client_secret)   │
│  └── 例: 服务端 Web 应用                                     │
│                                                             │
│  Public Client (公开客户端)                                   │
│  ├── 无 client_secret                                       │
│  ├── 认证方式: client_id 在请求体中                           │
│  ├── 必须使用 PKCE (RFC 9700)                                │
│  └── 例: 本地 CLI 应用（如本工程）                             │
└─────────────────────────────────────────────────────────────┘
```

代码中的双路径处理：

```rust
// oauth/mod.rs — exchange_code()
if let Some(ref secret) = provider.client_secret {
    // 机密客户端: HTTP Basic Auth (RFC 6749 §2.3.1)
    request = request.basic_auth(&provider.client_id, Some(secret));
} else {
    // 公开客户端: client_id 在 body 中 (RFC 6749 §2.3.1)
    params.push(("client_id", provider.client_id.clone()));
}
```

---

## 8. 代码架构与 RFC 映射表

### 8.1 模块结构

```
src/oauth/
├── mod.rs          ← 流程编排 + Provider 配置
│   ├── OAuthProvider        RFC 6749 §2 (Client Registration)
│   ├── build_auth_url()     RFC 6749 §4.1.1 + RFC 7636 §4.3
│   ├── exchange_code()      RFC 6749 §4.1.3 + RFC 7636 §4.5
│   ├── run_oauth_flow()     完整流程编排
│   ├── get_valid_token()    RFC 9700 §4.13 (自动刷新)
│   ├── github_provider()    内置 GitHub 配置
│   └── google_provider()    内置 Google 配置
│
├── pkce.rs         ← PKCE 密码学
│   ├── PkceChallenge        RFC 7636 §4.1-4.2
│   │   └── generate()       生成 verifier + challenge
│   └── generate_state()     RFC 6749 §10.12 (CSRF)
│
├── callback.rs     ← 本地回调服务器
│   ├── CALLBACK_PORT        9876 (固定端口)
│   ├── callback_url()       RFC 8252 §7.3 (Loopback)
│   ├── bind_callback_listener()  绑定 + IPv6 回退
│   ├── wait_for_callback()  接收授权码 + state 验证
│   ├── html_escape()        XSS 防护
│   └── landing_html()       回调结果页面
│
└── token.rs        ← Token 生命周期
    ├── TokenSet             RFC 6749 §5.1
    │   ├── from_response()  从 token 响应构建
    │   └── is_expired()     提前 60s 判断过期
    ├── TokenStore           文件存储 (~/.mini-agent/tokens.json)
    │   ├── load() / save()  持久化 (0600 权限)
    │   ├── store() / get()  CRUD
    │   └── remove()         撤销
    └── refresh_access_token()  RFC 6749 §6 + RFC 9700 §4.13.2
```

### 8.2 完整 RFC 条款映射

| 代码位置 | 函数/结构体 | RFC 条款 | 说明 |
|---------|------------|---------|------|
| `pkce.rs` | `PkceChallenge::generate()` | RFC 7636 §4.1 | code_verifier 生成 |
| `pkce.rs` | `PkceChallenge::generate()` | RFC 7636 §4.2 | code_challenge = SHA256(verifier) |
| `pkce.rs` | `generate_state()` | RFC 6749 §10.12 | CSRF state 参数 |
| `pkce.rs` | `generate_state()` | RFC 9700 §4.7.1 | state 必须不可预测 |
| `mod.rs` | `build_auth_url()` | RFC 6749 §4.1.1 | 授权请求参数 |
| `mod.rs` | `build_auth_url()` | RFC 7636 §4.3 | PKCE challenge 参数 |
| `mod.rs` | `build_auth_url()` | RFC 6749 §3.3 | scope 参数 |
| `mod.rs` | `exchange_code()` | RFC 6749 §4.1.3 | 令牌交换请求 |
| `mod.rs` | `exchange_code()` | RFC 7636 §4.5 | code_verifier 发送 |
| `mod.rs` | `exchange_code()` | RFC 6749 §2.3.1 | 客户端认证 (Basic/body) |
| `mod.rs` | `get_valid_token()` | RFC 9700 §4.13 | 自动刷新策略 |
| `callback.rs` | `callback_url()` | RFC 8252 §7.3 | Loopback redirect URI |
| `callback.rs` | `bind_callback_listener()` | RFC 8252 §7.3 | Loopback 绑定 |
| `callback.rs` | `wait_for_callback()` | RFC 6749 §4.1.2 | 授权响应处理 |
| `callback.rs` | `wait_for_callback()` | RFC 6749 §4.1.2.1 | 错误响应处理 |
| `callback.rs` | state 验证 | RFC 6749 §10.12 | CSRF 防护 |
| `callback.rs` | `html_escape()` | OWASP | XSS 防护 |
| `token.rs` | `TokenSet` | RFC 6749 §5.1 | Token 响应结构 |
| `token.rs` | `TokenSet.access_token` | RFC 6750 | Bearer Token |
| `token.rs` | `TokenSet.is_expired()` | RFC 9700 §4.13 | 提前刷新 |
| `token.rs` | `refresh_access_token()` | RFC 6749 §6 | Refresh Grant |
| `token.rs` | refresh token 轮换 | RFC 9700 §4.13.2 | Token Rotation |
| `token.rs` | `TokenStore::save()` 0600 | 最佳实践 | 文件权限保护 |

---

## 9. 与 IronClaw 的对照

本工程是 IronClaw OAuth 实现的精简提取。以下是对照关系：

```
┌─────────────────────────────────────────────────────────────────┐
│                  IronClaw OAuth 架构                             │
│                                                                 │
│  src/cli/oauth_defaults.rs     ← 共享 OAuth 基础设施             │
│  ├── OAuthCredentials          ← 内置凭据 (Google)               │
│  ├── build_oauth_url()         ← 构建授权 URL + PKCE             │
│  ├── exchange_oauth_code()     ← 令牌交换                        │
│  ├── store_oauth_tokens()      ← 存储到 SecretsStore             │
│  ├── validate_oauth_token()    ← Token 验证                      │
│  └── PendingOAuthFlow          ← 并发流程管理                     │
│                                                                 │
│  src/llm/oauth_helpers.rs      ← 回调服务器                      │
│  ├── bind_callback_listener()  ← 绑定 loopback                  │
│  ├── wait_for_callback()       ← 等待回调                        │
│  └── landing_html()            ← HTML 页面                       │
│                                                                 │
│  src/tools/mcp/auth.rs         ← MCP OAuth 2.1                  │
│  ├── ProtectedResourceMetadata ← RFC 8707 资源发现               │
│  ├── AuthorizationServerMetadata ← RFC 8414 服务器元数据          │
│  ├── ClientRegistrationRequest ← RFC 7591 动态注册               │
│  └── full_oauth_flow()         ← 完整 MCP OAuth 流程             │
│                                                                 │
│  src/extensions/manager.rs     ← WASM 工具 OAuth 编排            │
│  └── activate_extension()      ← 触发 OAuth 流程                 │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘

                              ↓ 精简提取

┌─────────────────────────────────────────────────────────────────┐
│                  Mini Agent OAuth 架构                           │
│                                                                 │
│  src/oauth/mod.rs              ← 流程编排 (合并 oauth_defaults)  │
│  src/oauth/pkce.rs             ← PKCE (提取自 build_oauth_url)  │
│  src/oauth/callback.rs         ← 回调 (提取自 oauth_helpers)     │
│  src/oauth/token.rs            ← Token (提取自 store_oauth_*)   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 简化了什么

| 特性 | IronClaw | Mini Agent | 原因 |
|------|----------|-----------|------|
| Token 存储 | 加密 SecretsStore | JSON 文件 (0600) | 简化，无系统钥匙串依赖 |
| OAuth 代理 | 支持 hosted proxy | 仅本地 loopback | 简化，无远程部署需求 |
| 动态注册 | RFC 7591 DCR | 手动配置 | 简化，减少复杂度 |
| 资源指示器 | RFC 8707 resource | 不支持 | MCP 专用特性 |
| 并发管理 | PendingOAuthRegistry | 单流程 | 简化，CLI 场景足够 |
| Token 验证 | validation endpoint | 仅过期检查 | 简化 |
| 内置凭据 | Google 编译时内置 | 运行时传入 | 简化，无硬编码密钥 |

### 保留了什么（核心安全特性）

- ✅ PKCE S256 (RFC 7636)
- ✅ CSRF state 验证 (RFC 6749 §10.12)
- ✅ Loopback-only 绑定 (RFC 8252 §7.3)
- ✅ 5 分钟超时
- ✅ HTML 转义 (XSS 防护)
- ✅ 错误优先检查
- ✅ Token 刷新 + 轮换 (RFC 9700 §4.13)
- ✅ 双路径客户端认证 (Basic/body)

---

## 10. 附录：RFC 速查表

### RFC 6749 — OAuth 2.0 Authorization Framework

| 章节 | 内容 | 本工程使用 |
|------|------|-----------|
| §1.1 | 角色定义 (Resource Owner, Client, Auth Server, Resource Server) | 概念基础 |
| §2.1 | 客户端类型 (Confidential vs Public) | 双路径认证 |
| §2.3.1 | 客户端认证 (HTTP Basic) | `exchange_code()` |
| §3.1 | Authorization Endpoint | `authorization_url` |
| §3.2 | Token Endpoint | `token_url` |
| §3.3 | Scope | `scopes` 字段 |
| §4.1 | Authorization Code Grant | 核心流程 |
| §4.1.1 | Authorization Request | `build_auth_url()` |
| §4.1.2 | Authorization Response | `wait_for_callback()` |
| §4.1.3 | Access Token Request | `exchange_code()` |
| §5.1 | Successful Response | `TokenSet` |
| §6 | Refreshing an Access Token | `refresh_access_token()` |
| §10.12 | CSRF Protection | `generate_state()` |

### RFC 7636 — PKCE

| 章节 | 内容 | 本工程使用 |
|------|------|-----------|
| §4.1 | Client Creates a Code Verifier | `PkceChallenge::generate()` |
| §4.2 | Client Creates the Code Challenge | SHA-256 + base64url |
| §4.3 | Client Sends Code Challenge with Auth Request | `build_auth_url()` |
| §4.5 | Client Sends Code Verifier with Token Request | `exchange_code()` |
| Appendix A | S256 code_challenge_method | `code_challenge_method=S256` |

### RFC 9700 — Security Best Current Practice

| 章节 | 内容 | 本工程使用 |
|------|------|-----------|
| §2.1.1 | Public clients MUST use PKCE | `use_pkce: true` |
| §2.1.1 | Confidential clients RECOMMENDED PKCE | 默认启用 |
| §2.1.1 | S256 is the only safe method | 仅支持 S256 |
| §4.5 | Authorization Code Injection | PKCE 防御 |
| §4.7.1 | CSRF via state | `generate_state()` |
| §4.8 | PKCE Downgrade Attack | 始终发送 challenge |
| §4.13 | Refresh tokens | `get_valid_token()` |
| §4.13.2 | Refresh token rotation | 保存新 refresh_token |

### RFC 8252 — OAuth 2.0 for Native Apps

| 章节 | 内容 | 本工程使用 |
|------|------|-----------|
| §7.3 | Loopback Interface Redirection | `127.0.0.1:9876` |
| §8.1 | Authorization Code Grant recommended | 使用此模式 |
| §8.2 | Implicit Grant NOT recommended | 不使用 |

### RFC 6750 — Bearer Token Usage

| 章节 | 内容 | 本工程使用 |
|------|------|-----------|
| §2.1 | Authorization Request Header Field | `Authorization: Bearer <token>` |
| §5.3 | Token Storage | 文件权限 0600 |

---

> **文档版本**: 1.0
> **对应代码**: `mini-agent-loop-oauth` (commit: feat(oauth))
> **参考 RFC**: 6749, 6750, 6819, 7009, 7591, 7636, 7662, 8252, 8414, 8707, 9700
