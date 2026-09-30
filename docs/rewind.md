# Rewind normal conversations

`/rewind` opens recent user prompts. Choose the prompt whose work you want to
undo, review the file preview, and restore **code**, **conversation**, or **both**.
The checkpoint is immediately before that prompt; selecting an older prompt
includes later prompts on the current branch. Each real user prompt has one
checkpoint, including interrupted turns. Hook feedback and harness continuations
do not create extra checkpoints.

Press Escape twice with an empty composer to open rewind. To keep the session
tree shortcut, set `doubleEscapeAction` to `tree` in settings. `fork` and `none`
also remain available.

File restoration uses the existing file-tool preimages, effect reports and
transactional rewind engine. `write`, `edit` and `apply_patch` edits are recorded.
A new file is removed when its contents still match the recorded version; a
deleted file is recreated from its preimage. Empty directories created for new
files can remain after their files are removed. Existing path, symlink and
unfinished-transaction protections still apply.

**Changes made through shell commands are not tracked and cannot be restored by
rewind.** This includes `bash`, `powershell`, `exec_command`, shell mode (`!`), and
external processes. Rewind is not a git reset or a snapshot of the whole working
tree. Stop background jobs and workers before rewinding.

Later overlapping user edits are conflicts and are never overwritten.
Nonoverlapping later edits can be retained by the existing three-way inverse.
Untracked edits between two recorded mutations of the same file are treated
conservatively as a conflict; restore those prompts separately. Conversation-only
restore is available even when code has conflicts.

Conversation restore moves to a durable branch before the selected prompt. The
abandoned messages remain in the session tree. Code-only restore keeps the
conversation, and restored effects are marked so another rewind does not replay
them. Checkpoint metadata and bounded effect reports survive reopening the
session. Reports contain exact file bytes and are created with user-only access
on Unix, beside the session as `<session>.rewind-effects.jsonl`; keep them with the
session. Existing 16 MiB per-blob, 256 MiB task-storage and 128 MiB effect-report
limits apply. A missing or invalid report fails closed.
Checkpoints are bound to the original workspace, so changing directories cannot
restore similarly named files in another project. Forks inherit completed
checkpoints and copy their validated reports into the fork's own report.
Snapshots from a process crash or a historical tree position within an
unfinished prompt omit that prompt's checkpoint, whose final effect boundary is
unavailable. Graceful aborts settle their checkpoint before returning.

## RPC

All commands use the existing JSONL RPC transport. No model call is needed.

1. `{"type":"get_rewind_checkpoints"}` returns `checkpoints` (newest first) and
   the shell-change limitation.
2. `{"type":"rewind_preview","checkpointId":"<id>"}` returns `preview` and
   the limitation. Read `preview.preview_digest`, files and conflicts.
3. `{"type":"rewind_apply","checkpointId":"<id>","previewDigest":"<digest>","mode":"both"}`
   restores the chosen domains. `mode` is `code`, `conversation`, or `both`.

Apply requires a preview minted by the same live host. The authority is consumed
on success, is never loaded from a client-supplied preview, and is invalid after
session changes, new prompts, transcript changes, recorded effects, or file
changes. A stale preview requires another preview. Streaming, compaction, live
workers and background jobs refuse rewind. File conflicts block code restoration;
conversation-only mode remains available.

The offline `prompt_rewind_*`, `rpc_rewind_*`, modal and double-Escape fixtures
exercise actual file-tool mutations, interruption and session reopening, domain
selection, manual edits, stale/forged authority, and shell-change disclosure.
