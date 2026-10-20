# /setup: vector memory and the token governor

`/setup` is the one place that reports and repairs the local services. Nothing
here downloads or starts software without your yes, and nothing here deletes
your records.

## Commands

| Command | Does |
| --- | --- |
| `/setup` | Reports every area, applies the local steps that need no approval, and tells you the exact command for the ones that do. |
| `/setup check` | Read-only. Changes no file, starts nothing, downloads nothing. `check --json` prints one JSON document (`steps`, `health`, `issues`). |
| `/setup status` | Same as `check`. |
| `/setup approve start-ollama` | Records your yes to start the local Ollama you have installed, then starts it. |
| `/setup approve download-model` | Records your yes to download the embedding model, then starts the download. |
| `/setup qdrant [enable\|disable\|own <id>\|release\|keep-running on\|off]` | The optional Qdrant copy of the vectors: explains it, looks (read-only) at Docker and Qdrant, and records your choice. Off by default. See `docs/vector-memory.md`. |
| `/setup approve start-qdrant` | Records your yes to starting the one Qdrant container you recorded as DaVinci's, then starts it. A later session restarts the same container if the projection is on. |
| `/setup repair` | Re-embeds records whose vector is missing or from another model, in batches. Derived data only: no record is changed or removed. |
| `/setup disable memory\|governor` | Turns the feature off for good (saved in `setup-profile.json`, previous file kept as `.bak`). |
| `/setup enable memory\|governor` | Turns it back on. If another layer still says off, it names which. |
| `/setup trust` | Records trust for this folder (the project trust store). |

## What "ready" means

Health comes from live probes, not from a flag: Ollama answers, the model is
installed, a test embedding has the configured size. States: `disabled`,
`unconfigured`, `checking`, `starting`, `ready`, `degraded`, `actionRequired`,
`failed`. Codes you may see: `PROVIDER_OFFLINE`, `MODEL_MISSING`,
`EMBEDDING_MISMATCH`, `CONFIG_CORRUPT`, `LOCAL_STORE_UNREADABLE`,
`CONSENT_REQUIRED`, `PROFILE_CORRUPT`, `ENV_CONFLICT`,
`GOVERNOR_STORE_UNWRITABLE`.

## Automatic start and recovery

If you approved starting Ollama, a later session brings that same executable
(same path and version) back by itself when it is down, off the turn path,
with retries after 1s, 2s and 4s and then every 120s. A failure produces one
notice; recovery produces one. An Ollama you started yourself is never
stopped or restarted by DaVinci. A feature you turned off gets no probe, no
start and no notice.

## Data safety

- `.davinci/vector-memory/records.jsonl` is authoritative. An unreadable file
  (for example not valid UTF-8) is reported as `LOCAL_STORE_UNREADABLE`, writes
  are blocked, and the bytes are left alone.
- A corrupt `vector-memory.json`, `token-governor.json` or `setup-profile.json`
  leaves the feature off and is never replaced by defaults.
- Environment variables cannot re-enable a feature you switched off; the
  conflict is shown as `ENV_CONFLICT`.

## Token governor

`/governor-status` and the `/setup` health list report `Enabled`, `Disabled` or
`Degraded` (turned on but the output store cannot be written), with the store
path, counters and the last error. When an original cannot be stored the model
keeps the whole output. Language-tool output stored before a restart is not
served again after it; run the tool again.

## Not covered by the automatic tests

The tests use loopback fakes and an injected launcher. A real restart matrix
with a real Ollama on Windows (stopped, killed, port taken, model removed,
machine reboot) has not been run.
