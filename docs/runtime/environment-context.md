# Runtime environment context

The optional Task 5 environment snapshot is enabled with
`PI_ENVIRONMENT_CONTEXT=1`. It is off by default. The earlier combined
environment/guidance experiment was not promoted after the results recorded in
[E1](../perf/request-efficiency/E1.md). Restoring the implementation behind this
flag supplies an independently testable experiment; it does not establish a
latency or request-count improvement.

When enabled for a built-in prompt, the first request of a real user turn gets
the current working directory, operating system, UTC date, tool-to-shell mapping,
top-level names, and a Python executable discovery hint. The shell mapping uses
the tool runtime selectors, including `PI_SHELL` for `bash`/`shell` and Unix
`exec_command`, and `pwsh`/`powershell` for Windows `exec_command`. Discovery does
not execute a shell, Python, project code, `date`, or `ls`. A discovered Python
path is not proof that Python or a test suite executes successfully.

The snapshot is reused through model/tool continuations. A new real user turn
refreshes it, including the date. Changing the agent cwd, advertised shell
interfaces, or shell-selection environment also refreshes it at a request
boundary while retaining the current turn's date. Filesystem changes alone do
not trigger repeated listing or reappend the snapshot during that turn.

The directory listing never recurses. It scans at most 4,096 entries, sorts
names lexically, and keeps at most 50 names. If the scan limit is exceeded, the
partial list is omitted. Missing/unreadable directories and timed-out scans
have explicit status values. Non-UTF-8 names use lossy conversion with an
explicit flag. Symlink entries are names only; their targets are not scanned.

The rendered environment block is limited to 4,096 UTF-8 bytes. Long fields and
name lists are truncated with status flags. JSON string escaping and escaping
of `<`, `>`, and `&` prevent filenames from closing the surrounding prompt
sections. The complete runtime-state budget is 1,500 estimated tokens.

Each agent admits one probe worker, shared by its clones, and waits at most
75 ms for it. An operating-system filesystem operation cannot always be
cancelled. If one remains stalled, later captures report `probe_busy` until it
returns rather than creating more workers. The snapshot is used as a bounded
fallback and late results do not rewrite a prepared turn.

Environment data stays in the dynamic runtime section. Cache-sensitive routes
append it to conversation context and retain byte-identical stable provider
instructions. Other routes retain their existing system-prompt placement.
Custom replacement prompts receive no built-in environment or visual policy.
Within this experiment, visual-backend guidance is conditional on visual task
relevance or a visual verification requirement. The default retains the
existing guidance.

The duplicate runtime wrapper correction is enabled independently of this
experiment: `runtime_state_text` owns the one `<runtime_state>` block and turn
context includes the rendered section directly.

## Verification notices

Completion evidence is delivered as a `verification_notice` agent event with
`status`, `generation`, and `text`. It is display metadata, not model-authored
chat. Interactive hosts render a notice; print mode keeps the model answer on
stdout and writes notices to stderr. JSON mode exposes the distinct event.
The same status for the same mutation generation is emitted once. A changed
status or a later mutation may emit a new notice.

Notices do not enter the assistant transcript, provider input, or persisted
chat. Consequently final-answer and `/copy` selection continue to use the
model's text. When reopening a session written by an older build, messages
marked `davinciVerificationStatus` are omitted from active conversation
history. The original session journal is preserved.
