# ChatGPT Codex backend route (the `openai-codex` OAuth login)

Status: implemented offline. **No live request has been made through this route from DaVinci.** `/login openai-codex`
is the only OAuth login; it uses the Codex CLI OAuth client and the route below. API-key use goes through `openai`.
A Sign in with ChatGPT login stored by an earlier build (no route marker) still resolves on the documented
`https://api.openai.com/v1` route until `/login openai-codex` replaces it.

## Why it exists

A 6-call route probe (2026-10-17, same prompt, about 835 output tokens, no tools, same model and effort) measured the
documented route at 35 to 45 tokens per second and Codex CLI's route at 122 to 136. A 48-run comparison on the 8 public
tasks put DaVinci at 1.81x Codex's median wall time with 98% of that time spent waiting on the provider. The
wall-time gap is the route, not DaVinci's own code. Evidence: `C:\bench\pubcmp\route-probe.jsonl`,
`C:\bench\pubcmp\report.txt`.

## How to use it

```
/login openai-codex     sign in with the Codex CLI OAuth client; stores the login under openai-codex
/login openai <key>     API key provider, separate from the subscription login
```

The route follows the credential. A login made with `/login openai-codex` carries `OPENAI_CODEX_ROUTE=chatgpt-backend`
in its stored `env`; `resolve_provider_auth` reports its source as `OAuth (Codex CLI)`; the stream layer
(`codex_backend::route_model`) then moves the model's base URL to `https://chatgpt.com/backend-api/codex`, so requests
go to `https://chatgpt.com/backend-api/codex/responses` (HTTP) or the `wss://` form of it, the endpoint Codex CLI uses. Nothing else
changes: no catalog entry, no setting, no other provider.

## Risks you accept by using it

- The backend is the Codex CLI product's private route. The login presents the **Codex CLI OAuth client**
  (`app_EMoamEEZ73f0CkXaXp7hrann`, redirect `localhost:1455`, `codex_cli_simplified_flow`). OpenAI can change or
  refuse that at any time, and it may not be permitted for third-party clients. The risk is to the ChatGPT account.
- Commit `58b0a742` removed this route on purpose ("disable API-key/private-backend fallback"). This branch brings
  back the login, not the API-key fallback.
- `AGENTS.md`: public Responses capabilities do not establish backend support. The recorded backend probe
  (`codex-backend-probe.md`, 2026-09-26) shows rejected request shapes (prompt cache options, prewarm, late tools,
  remote compaction); those stay disabled for this route.

## Guards

- The route follows the credential. A login stored before the Codex CLI login became the only one keeps the documented route until it is replaced.
- `stream::require_codex_subscription_route`: documented route plus verified Sign in with ChatGPT OAuth, or a backend
  login routed to the backend. A backend URL with any other credential is refused. Authorization header overrides are
  refused on both routes. No API key is accepted for `openai-codex`.
- A backend login needs an access token with the `chatgpt_account_id` claim the backend requires.
- Subscription-policy admission still requires the documented route and refuses a backend login.

## What is not covered

- The Fast service tier is not changed or enabled.
- Model catalog refresh (`https://api.openai.com/v1/models`) and the plan-account email check read a Sign in with ChatGPT token and skip a Codex CLI login, so the built-in catalog is used. Whether the Codex CLI token is accepted by the catalog endpoint is unverified.
- No live verification. Before relying on it, run one scoped probe through the backend login and record the outcome
  here: first request accepted, cached tokens, tokens per second against the documented route.

## Tests

`crates/davinci-ai/src/codex_backend.rs` (5 tests): the route moves only for a backend login; the account-id
requirement; resolved source; the route gate on both routes; the login provider's authorize, exchange and refresh
requests while `openai-codex` keeps none.
