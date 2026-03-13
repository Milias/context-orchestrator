# Claude Code Subscription Usage via LiteLLM OAuth2

> **Date:** 2026-03-13 | Research into using a Claude Code subscription (Pro/Max plan) to power this application through LiteLLM's OAuth2 support or direct token usage.

---

> **BLOCKER — OAuth Tokens Prohibited in Third-Party Apps**
>
> As of approximately January 2026, Anthropic explicitly prohibits using OAuth tokens (`sk-ant-oat01-*`) obtained through Claude Pro/Max subscriptions in any application other than Claude Code and claude.ai. This is stated in their [legal and compliance documentation](https://code.claude.com/docs/en/legal-and-compliance) and is technically enforced server-side. **All three options described below are currently non-viable.** This document is preserved as research for context, and in case the policy changes in the future.

---

## 1. Executive Summary

This research investigated whether Claude Code subscription credits (Pro, Max 5x, Max 20x) could replace pay-per-token API keys for this application. The investigation found **three blocking issues**:

1. **ToS prohibition**: Anthropic's Consumer Terms of Service explicitly prohibit using OAuth tokens in third-party applications. This is enforced server-side — the API rejects `sk-ant-oat01-*` tokens from non-Claude-Code clients.
2. **Auth header conflict**: The app sends the same value as both `x-api-key` and `Authorization: Bearer` (`src/llm/anthropic.rs:85-86`). Anthropic's API checks `x-api-key` first — an OAuth token in that header causes an immediate `"invalid x-api-key"` error before the Bearer token is evaluated.
3. **Token lifecycle**: The access token (`sk-ant-oat01-*`) expires in ~8 hours, not ~1 year. The 1-year lifetime applies to the refresh token (`sk-ant-ort01-*`), which requires refresh logic the app does not implement.

**Conclusion**: The only supported authentication for third-party applications is API key authentication via the Anthropic Console (`sk-ant-api03-*`) with pay-per-token billing.

---

## 2. Current Architecture

### Auth Flow

The app resolves credentials through a priority chain in `src/config.rs:72-79`:

```
ANTHROPIC_AUTH_TOKEN → ANTHROPIC_API_KEY → error
```

`ANTHROPIC_AUTH_TOKEN` takes precedence. The resolved value is used as both an API key header and a Bearer token (`src/llm/anthropic.rs:85-86`):

```rust
.header("x-api-key", &self.api_key)
.header("authorization", format!("Bearer {}", self.api_key))
```

This dual-header approach works correctly with standard API keys (`sk-ant-api03-*`) — Anthropic authenticates via `x-api-key` and ignores the Bearer header. However, it **fails with OAuth tokens** because an OAuth token in `x-api-key` is rejected before the Bearer token is evaluated (see Section 8).

### Current LiteLLM Setup

The existing `run.sh` connects through a corporate LiteLLM proxy:

```bash
export ANTHROPIC_BASE_URL=https://litellm-v2.trading.imc.intra/
export ANTHROPIC_AUTH_TOKEN=sk-12Zcd0gUrgzXaepi3KVdbQ
```

This token is a LiteLLM master key, not an Anthropic credential. The proxy handles actual Anthropic authentication.

---

## 3. Requirements

| Requirement | Status |
|---|---|
| Use subscription credits | **BLOCKED** — prohibited by ToS |
| No code changes | **BLOCKED** — dual-header pattern incompatible with OAuth tokens |
| Token lifecycle | **BLOCKED** — access token expires in ~8hrs, app has no refresh logic |
| ToS compliance | **FAILED** — explicitly violates Consumer Terms of Service |
| Fallback | Existing retry logic correctly fails fast on 401 (no wasted retries) |

---

## 4. Option A: Direct OAuth Token (No Proxy)

> **Status: NON-VIABLE** — Blocked by ToS prohibition and auth header conflict.

The intended approach: use the Claude Code CLI to generate an OAuth token and pass it directly to Anthropic's API.

### Setup Steps (For Reference)

1. **Generate OAuth token**:
   ```bash
   claude setup-token
   ```
   Outputs a token with prefix `sk-ant-oat01-*`. Note: the `setup-token` subcommand is not listed in the official CLI reference (`claude --help`), but is widely referenced in community documentation and GitHub issues.

2. **Configure the app**:
   ```bash
   export ANTHROPIC_BASE_URL=https://api.anthropic.com
   export ANTHROPIC_AUTH_TOKEN=sk-ant-oat01-<your-token>
   ```

### Why It Fails

1. **ToS enforcement**: Anthropic's server-side checks reject `sk-ant-oat01-*` tokens from third-party applications. The API returns `"invalid x-api-key"` for the `x-api-key` header and `"OAuth authentication is currently not supported."` for the Bearer header when used outside Claude Code.

2. **Header precedence**: Anthropic's API evaluates `x-api-key` before `Authorization: Bearer`. Since the app sends the OAuth token as both, the API rejects the invalid `x-api-key` format immediately — the Bearer token is never checked.

3. **Token expiry**: The access token expires in ~8 hours. The ~1 year lifetime applies to the refresh token (`sk-ant-ort01-*`), which Claude Code CLI uses internally to obtain fresh access tokens. A custom app would need to implement OAuth token refresh logic.

### Token Details (Corrected)

| Property | Access Token | Refresh Token |
|---|---|---|
| Prefix | `sk-ant-oat01-` | `sk-ant-ort01-` |
| Lifetime | ~8 hours | ~1 year |
| Auto-refresh | No (requires refresh logic) | N/A |
| Used by | API requests | Obtaining new access tokens |

---

## 5. Option B: LiteLLM Proxy with OAuth

> **Status: NON-VIABLE** — Blocked by ToS prohibition and LiteLLM header forwarding limitations.

The intended approach: configure LiteLLM to forward the OAuth token to Anthropic.

### LiteLLM Configuration (For Reference)

```yaml
# litellm_config.yaml
model_list:
  - model_name: claude-sonnet-4-6
    litellm_params:
      model: anthropic/claude-sonnet-4-6-20250929
  - model_name: claude-opus-4-6
    litellm_params:
      model: anthropic/claude-opus-4-6

general_settings:
  forward_client_headers_to_llm_api: true
  forward_llm_provider_auth_headers: true  # Required for BYOK in LiteLLM v1.82+
  master_key: os.environ/LITELLM_MASTER_KEY
```

### Why It Fails (Beyond ToS)

1. **Authorization header not forwarded by default**: LiteLLM's documentation states: "The proxy's Authorization header (used for proxy authentication) is never forwarded to LLM providers." The `forward_client_headers_to_llm_api` setting alone is insufficient — `forward_llm_provider_auth_headers: true` is also required (added in LiteLLM v1.82+).

2. **Unfixed bugs**: GitHub issue BerriAI/litellm#14847 ("Anthropic v1/messages endpoint doesn't forward Client Headers to LLM API") was closed as NOT_PLANNED (auto-closed stale). This is the exact endpoint this application uses.

3. **Passthrough endpoint may help**: The passthrough endpoint (`/anthropic/v1/messages`) bypasses LiteLLM's request transformation and may handle headers differently. However, the app would need `ANTHROPIC_BASE_URL=http://localhost:4000/anthropic` to use it.

### Endpoint Patterns

LiteLLM exposes two endpoint patterns for Anthropic models:

1. **Standard** — `POST http://localhost:4000/v1/messages`
   LiteLLM translates the request. Requires models in `model_list`. Header forwarding has known bugs.

2. **Passthrough** — `POST http://localhost:4000/anthropic/v1/messages`
   LiteLLM proxies without transformation. No model list needed. More reliable for header forwarding.

---

## 6. Option C: LiteLLM with Per-User OAuth (Multi-User)

> **Status: NON-VIABLE** — Same blockers as Option B, compounded across users.

For scenarios where multiple users each bring their own Claude subscription. Each user generates their own `sk-ant-oat01-*` token. Same ToS and technical blockers apply.

---

## 7. Comparison Matrix

| Dimension | A: Direct OAuth | B: LiteLLM + OAuth | C: LiteLLM Multi-User |
|---|---|---|---|
| Setup complexity | Minimal | Moderate | Moderate + per-user |
| Code changes needed | Yes (remove `x-api-key` for OAuth) | Yes (same + LiteLLM config) | Yes (same) |
| ToS compliance | **Prohibited** | **Prohibited** | **Prohibited** |
| Token lifetime | ~8 hours (needs refresh) | ~8 hours (needs refresh) | ~8 hours (needs refresh) |
| Reliability | N/A | Low (header forwarding bugs) | Low |

**Recommendation**: None of these options are currently viable. Use API keys (`sk-ant-api03-*`) with pay-per-token billing.

---

## 8. Known Issues

### Anthropic OAuth Token Restriction (CRITICAL)

As of approximately January 2026, Anthropic deployed server-side checks that reject `sk-ant-oat01-*` tokens from third-party applications. Their [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance) states:

> OAuth authentication (used with Free, Pro, and Max plans) is intended exclusively for Claude Code and Claude.ai. Using OAuth tokens obtained through Claude Free, Pro, or Max accounts in any other product, tool, or service — including the Agent SDK — is not permitted and constitutes a violation of the Consumer Terms of Service.

This was reported in GitHub issue anthropics/claude-code#28091.

### Auth Header Precedence

Anthropic's API evaluates `x-api-key` before `Authorization: Bearer`. When both are present:
- If `x-api-key` contains a valid API key (`sk-ant-api03-*`): authenticates via that key, Bearer ignored
- If `x-api-key` contains an invalid value (including OAuth tokens): returns `"invalid x-api-key"` immediately — **Bearer token is never checked**

This means the app's current dual-header pattern (`src/llm/anthropic.rs:85-86`) would need modification for OAuth: the `x-api-key` header must be omitted or set to a different value when using Bearer authentication.

### LiteLLM Header Stripping

LiteLLM has documented issues with header forwarding for Anthropic:

- **GitHub Issue #13380**: Feature request for OAuth pass-through support for Anthropic (closed after PR #14821 merged)
- **GitHub Issue #19618**: OAuth tokens not forwarded — `clean_headers()` strips Authorization and `_get_forwardable_headers()` only forwards `x-*` and `anthropic-beta` headers
- **GitHub Issue #22398**: OAuth handler overwrites `anthropic-beta` header, dropping client-specified beta features
- **GitHub Issue #14847**: Anthropic `/v1/messages` endpoint doesn't forward client headers (closed NOT_PLANNED as stale)

### Token Expiry Handling

The access token (`sk-ant-oat01-*`) expires in ~8 hours. When expired:

- The app receives a `401 Unauthorized` response
- The retry logic (`src/llm/retry.rs`) correctly classifies 401 as `ApiError::Auth` (not retryable) and fails immediately without wasting retries
- The user must obtain a new token

The ~1 year lifetime frequently cited in community documentation refers to the refresh token (`sk-ant-ort01-*`), which Claude Code CLI uses internally to obtain fresh access tokens. A custom app would need OAuth refresh logic to leverage this.

### Subscription Usage Sharing

The OAuth token shares the subscription's usage quota across all surfaces:
- Claude web interface (claude.ai)
- Claude Code CLI
- Any other tool using the same token

Heavy usage in one surface reduces availability in others. Max plan rate limits are per-account, not per-application.

---

## 9. Recommended Setup

Given the blockers identified, the only supported path is **standard API key authentication**:

```bash
export ANTHROPIC_BASE_URL=https://api.anthropic.com
export ANTHROPIC_API_KEY=sk-ant-api03-<your-console-api-key>
export ANTHROPIC_MODEL=claude-sonnet-4-6

cargo run
```

Or via LiteLLM proxy (as currently configured in `run.sh`), where LiteLLM manages the API key.

### If Anthropic Lifts the OAuth Restriction

Should Anthropic change their policy to allow OAuth tokens in third-party apps, the app would need one code change: conditionally omit the `x-api-key` header when the credential is an OAuth token (detectable by the `sk-ant-oat01-` prefix). The `Authorization: Bearer` header alone would suffice. Additionally, token refresh logic would need to be implemented to handle the ~8 hour access token lifetime.

---

## 10. Integration with This App

### Current State

The app works correctly with:
1. **Direct API keys** (`sk-ant-api03-*`) via `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`
2. **LiteLLM proxy** with a master key (current `run.sh` setup)

### What Would Need to Change for OAuth (If Policy Changes)

1. **Detect token type**: Check if the configured token starts with `sk-ant-oat01-` to determine auth mode
2. **Conditional headers**: Send only `Authorization: Bearer` (not `x-api-key`) for OAuth tokens, since `x-api-key` rejects non-API-key formats
3. **Token refresh**: Implement OAuth token refresh using the refresh token (`sk-ant-ort01-*`) to handle ~8 hour access token expiry
4. **LiteLLM config**: Add `forward_llm_provider_auth_headers: true` alongside `forward_client_headers_to_llm_api: true` and test with a pinned LiteLLM version

---

## 11. Red/Green Team Audit

### Green Team (Verification)

**Token format**: `claude setup-token` produces tokens with `sk-ant-oat01-` prefix. Confirmed across multiple sources. However, the command is not listed in the official CLI reference — it appears in community documentation and GitHub issues.

**LiteLLM config syntax**: The YAML format is correct. However, `forward_client_headers_to_llm_api: true` alone does not forward the `Authorization` header — LiteLLM explicitly excludes it. The `forward_llm_provider_auth_headers: true` setting (v1.82+) is also required for BYOK scenarios.

**GitHub issues verified**:
- #13380 — EXISTS. Feature request for OAuth pass-through (not a bug report as originally described)
- #19618 — EXISTS. OAuth tokens not forwarded due to `clean_headers()` behavior
- #22398 — EXISTS. OAuth handler overwrites `anthropic-beta` header (not Authorization stripping as originally described)

**Pricing confirmed** (as of research date):
- Max 5x: $100/month ✓
- Max 20x: $200/month ✓
- API Sonnet: ~$3/$15 per million input/output tokens ✓
- API Opus: ~$5/$25 per million input/output tokens ✓

**Token lifetime corrected**: Access token (`sk-ant-oat01-*`) expires in ~8 hours. Refresh token (`sk-ant-ort01-*`) lasts ~1 year. Claude Code CLI handles refresh automatically; a custom app would not.

**Passthrough endpoint confirmed**: `/anthropic/v1/messages` is the correct LiteLLM passthrough path, confirmed by both LiteLLM and Claude Code documentation.

**Code references**: All 9 file:line references verified correct against source code.

### Red Team (Challenges)

**Challenge 1: ToS compliance** — **CRITICAL**
Anthropic explicitly prohibits using OAuth tokens in third-party applications. This is stated in their legal/compliance documentation and technically enforced server-side since approximately January 2026. The API rejects `sk-ant-oat01-*` tokens from non-Claude-Code clients.

**Challenge 2: Auth header precedence** — **HIGH**
The original document claimed Bearer takes precedence over `x-api-key`. This is backwards — `x-api-key` is checked first. An invalid `x-api-key` value (like an OAuth token) causes immediate rejection before the Bearer token is evaluated. The app's dual-header pattern is incompatible with OAuth tokens.

**Challenge 3: Retry behavior** — **LOW (document corrected)**
The original document incorrectly stated the retry logic would retry on 401. In fact, `src/llm/error.rs` correctly classifies 401/403 as `ApiError::Auth` (not retryable), and the retry loop in `src/llm/retry.rs` exits immediately for non-retryable errors. The code is correct; the original description was wrong.

**Challenge 4: LiteLLM header forwarding** — **HIGH**
GitHub issue BerriAI/litellm#14847 (Anthropic `/v1/messages` doesn't forward client headers) was closed NOT_PLANNED as stale — no fix implemented. The Authorization header is intentionally excluded from forwarding. The passthrough endpoint may be more reliable but is untested with this app.

**Challenge 5: Token stored in plain text** — **MEDIUM**
The `run.sh` approach stores tokens in a plain-text shell script. Tokens should be stored in `.env` (already in `.gitignore`) or a secrets manager.

---

## 12. Sources

- Anthropic Legal and Compliance: [code.claude.com/docs/en/legal-and-compliance](https://code.claude.com/docs/en/legal-and-compliance)
- Anthropic API Documentation: Messages API authentication
- Claude Code CLI: `claude setup-token` command (community-documented)
- LiteLLM Documentation: [forward_client_headers_to_llm_api](https://docs.litellm.ai/docs/proxy/forward_client_headers)
- LiteLLM Claude Code Max Tutorial: [docs.litellm.ai/docs/tutorials/claude_code_max_subscription](https://docs.litellm.ai/docs/tutorials/claude_code_max_subscription)
- LiteLLM GitHub Issues: #13380, #14847, #19618, #22398
- Anthropic GitHub: anthropics/claude-code#28091 (OAuth restriction report)
- Anthropic Pricing: [claude.ai/pricing](https://claude.ai/pricing)
- Source code: `src/config.rs`, `src/llm/anthropic.rs`, `src/llm/error.rs`, `src/llm/retry.rs`, `run.sh`
