# Browser / Playwright verification

DaVinci can verify frontend behavior against managed local dev servers using real
browser automation. The native browser tools work in ordinary interactive sessions
and authorized Graph roles. Browser verification is strictly optional: if
unconfigured or unavailable, sessions continue normally with graceful fallback to
source analysis and targeted tests.

## Host configuration and prerequisites

Browser automation requires host-configured, trusted dependencies. DaVinci never
downloads or installs Playwright, Node, or Chromium at runtime, and never executes
untrusted dependency installation scripts or resolves project `node_modules`.

- **Global configuration**: Host configuration specifies `enabled: true`, the
  absolute path to trusted `node`, the absolute path to an external trusted
  Playwright `package`, and the pinned Playwright `version`.
- **Project settings override**: Project settings may only disable the capability
  (`browserVerification.enabled: false`). Project configuration cannot enable a
  disabled host or override host binary, package, or version pins.
- **Fail-open fallback**: When browser verification is disabled, unconfigured, or
  the backend crashes/exits, tool calls return structured fallback errors, allowing
  the agent to fall back to test-based or source-based verification.

## Model-facing tools and opaque handles

When enabled and permitted, ten native tools are exposed to the agent:

| Tool | Purpose |
| --- | --- |
| `browser_open` | Connects to a managed dev server and opens a new isolated browser context. |
| `browser_snapshot` | Returns bounded DOM HTML of the current page. |
| `browser_click` | Performs a user click using role/name, label, or test-id selectors. |
| `browser_type` | Enters text into an input element identified by role or label. |
| `browser_select` | Selects an option in a dropdown selector. |
| `browser_console` | Retrieves collected console error messages. |
| `browser_network` | Retrieves failed network requests (e.g. HTTP 4xx/5xx). |
| `browser_accessibility` | Returns an ARIA accessibility tree snapshot. |
| `browser_screenshot` | Retains a PNG screenshot as an immutable content-addressed artifact. |
| `browser_close` | Closes the browser context and releases associated resources. |

### Tool boundaries and security invariants

- **Opaque handles only**: The model interacts solely through opaque identifiers
  (`browser_id`). It cannot provide file paths, executable paths, URLs outside the
  managed server's origin, CDP commands, or arbitrary JavaScript execution strings.
- **Selector confinement**: Actions accept deterministic, accessible selectors
  (`role`, `name`, `label`, `data-testid`). Arbitrary XPath, eval strings, or
  unrestricted script injection are rejected.
- **Network confinement**: Every context is bound to a dedicated origin proxy.
  Traffic is restricted to the declared managed server port on `127.0.0.1` or `::1`.
  Outbound requests to foreign hosts, foreign ports, unauthorized redirects,
  unauthorized HTTPS tunnels, and foreign WebSocket upgrades fail closed with zero
  outbound hits.
- **Bounded outputs**: DOM snapshots, accessibility trees, console logs, network
  events, and tool responses are strictly size-capped. Truncation signals
  (`omitted: true`) are tracked explicitly.

## Transaction source binding

`browser_open` optionally accepts an opaque `transaction_id`.

- **Host-resolved binding**: The model supplies only the transaction ID. The host
  resolves the transaction record through the trusted coordinator, capturing its
  sequence, canonical workspace identity, affected file paths, and SHA-256 source
  digest.
- **Current read authority rechecks**: `ProcessManager::check_current_source_read`
  verifies live session ownership, canonical workspace, active task contract, and
  permission policy for `read` before binding and before collecting evidence.
- **Invalidation on change**: If the transaction source, sequence, workspace, or
  affected file content changes between browser operations or before verification,
  source binding fails rather than silently rebinding to newer source.

## Browser verification receipts vs. transaction state

Host verification is performed through the host-only non-model RPC route
`verify_browser` (taking `browser_id`, `dom_contains`, and optional
`accessibility_contains`).

- **Receipt composition**: Returns an `InteractionReceipt` with `backend_kind:
  real_browser`, asserting:
  1. DOM contains the expected assertion string.
  2. Accessibility tree contains the expected text (if specified).
  3. No console errors occurred during the session.
  4. No network request failures occurred.
  5. No evidence was truncated (`incomplete_coverage` is empty).
  6. Screenshot artifact was successfully retained.
  7. Transaction source remained unchanged throughout verification.
- **Evidence only — not automatic verification**: A passing browser verification
  receipt is evidence of runtime behavior. It **does not** silently transition an
  edit transaction to `Verified` state. Transactions remain in `Applied` state until
  the complete verification protocol finishes.

## Retained screenshot artifacts

`browser_screenshot` captures PNG screenshots into DaVinci's existing bounded
artifact storage tracker.

- **No binary output in conversations**: Screenshots never produce base64 strings
  or raw bytes in tool call responses or model context.
- **Content-addressed storage**: Artifacts are stored under SHA-256 content hashes
  with immutable UUID labels. Duplicate writes and cross-context access are denied.
- **Frontend RPC retrieval**: Frontend clients retrieve screenshot bytes in bounded
  chunks via the `get_browser_artifact` RPC command, validated against the
  original context process lease, canonical workspace, and current permissions.

## Graph integration

- **Shared parent engine**: In Graph mode, workers share the parent session's
  browser engine and artifact budget while isolating browser contexts, cookies, and
  temporary state.
- **Authenticated coordinator transport**: Worker browser tools are forwarded over
  the parent task coordinator channel with bounded timeouts and cancellation signals.
- **Conditional registration**: Browser tools are placed into the coordinator
  transport only when an actual browser host is configured (`deps.browser.is_some()`).
  If no browser is configured, coordinator transport registers only process and task
  tools, preventing dispatch errors.
- **Host-derived transaction owner**: Graph worker transactions receive their owner
  identity (`agent_id`, `parent_agent_id`, `session_id`, `task_id`, `graph_node`)
  strictly from trusted scheduler state, never from model-supplied parameters.
