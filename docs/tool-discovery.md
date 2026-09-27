# Deferred tool discovery and patch inputs

`ToolSurface::Lean` remains opt-in. On cache-sensitive routes, a fresh Lean
root exposes the authorized core: `read`, `grep`, `find`, `ls`, `exec_command`,
`write_stdin`, `apply_patch`, `batch`, `tool_search`, `update_plan`,
`propose_plan`, `ask_user_question`, `agent`, `job_output`, and `job_kill`.
Windows also exposes `bash`. Alternate shells, legacy editing interfaces, and
specialist tools remain discoverable. Existing visibility is retained when
switching a running session from Full to Lean; permission revocation still
removes the affected schemas.

## Discovery

Use `tool_search` with one of these inputs:

```json
{"mode":"exact","query":"git_branch_diff"}
{"mode":"family","query":"git"}
{"mode":"search","query":"history","limit":5}
```

Exact mode matches a registered name; family mode matches a registered family
ID. Family discovery exposes every authorized member that has a schema.
Search and exact discovery expose the returned page. Listings default to five
names and cap at 20. The response includes total and activation counts,
family IDs, schema availability, and a cursor when more names remain.
Pass that cursor with the same mode and query to continue the listing.

The native adapter registers `browser`, `git`, `lsp`, `package`, `build`,
`sec`, `graph`, `tests`, `impact`, `verification`, `repo`, `workspace`,
`memory`, and `skills`. Built-in managed-process tools use `process`.
Known extension tool names share that registration table. Other manifest
tools use `extension:<manifest-name>`.

MCP tools receive metadata when the handshake route is registered. Known tool
names and exact server IDs (such as `git`, `github`, `playwright`, and `lsp`)
map to their corresponding families. Other connected tools use
`mcp:<server-name>`. Arbitrary tool/server name prefixes do not confer family
membership, read-only trust, or permission.

A host without a runtime registry can still discover built-in and connected
MCP schemas. Native and JavaScript extension schemas require their owning
runtime registry; discovery does not claim those adapters are available when
they are absent.

## Relevance before a request

Lean roots add relevant registered families from the actual user request.
Examples include Git history, browser screenshots, language-server references,
package exports, background processes, build targets, security audits, and
graph workflows. A generic parser edit does not activate those families.
Explicit registered tool names and namespaced family IDs also select their
family. Nested workers retain their existing exposure policy.

Activation is additive across requests and user turns. It filters the current
tool selection, tool-wide denial rules, and schema availability. Scoped path
and argument permissions still run at dispatch. An unknown family has no
effect. Optional decision advice uses the same `Agent::activate_tool_families`
boundary and cannot grant authority or remove the core.

## Patch and edit inputs

Update hunks require `@@` and at least one content line. An empty unprefixed
line inside a hunk means empty context, just like a line containing one space.
`*** End of File` constrains that hunk's consumed context to the actual file
tail; an earlier repeated match is not eligible. Failure to match the tail
aborts the transaction before writes. LF and CRLF files retain their line
ending style and existing final-newline behavior.

The model-facing `edit` schema requires `path` and a nonempty `edits` array,
with `oldText` and `newText` in each item. Legacy runtime input compatibility
is retained.

The advertised patch grammar has a separate developer check in
`crates/davinci-ai/tests/apply_patch_grammar.py`, using the test-only
`lark==1.2.2` dependency. It exercises actual grammar parsing, including blank
context, empty additions, CRLF, EOF markers, and malformed hunks. This local
check does not establish live provider acceptance or promotion evidence.
