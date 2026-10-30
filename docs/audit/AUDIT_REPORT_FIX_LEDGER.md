# Audit report fix ledger

Generated from the audit plan ledger, addendum and the commits on `J12003LPZ/audit-report-fixes`. Each row maps a report reference to its bug ID and disposition. Where a named regression exists it is `audit_wpNN_bNNN`; rows marked fixed without a named test still carry the package commit.

| ID | Refs | Title | Sev | Package | Disposition |
|---|---|---|---|---|---|
| B001 | T39, T58 | Hard links bypass read, search, and snapshot permission boundaries | H | WP01 | fixed; commit a2362c54; test audit_wp01_b001 |
| B002 | T3 | AppContainer mount capability names collide for distinct Unicode paths | H | WP01 | fixed; commit a2362c54; test audit_wp01_b002 |
| B003 | T4 | Host Git runs repository-local `core.fsmonitor` during prompt preparation | H | WP01 | fixed; commit a2362c54; test audit_wp01_b003 |
| B004 | T38 | Built-in `read` bypasses size bounds for images and notebooks | H | WP01 | fixed; commit a2362c54; test audit_wp01_b004 |
| B005 | T40 | Built-in `grep` reads every candidate file without a byte cap | H | WP01 | fixed; commit a2362c54; test audit_wp01_b005 |
| B006 | T41 | Checkpoint capture reads oversized files fully before rejecting them | H | WP01 | fixed; commit a2362c54; test audit_wp01_b006 |
| B007 | T47 | Saved workflow loading allocates the entire JSON file before validation | H | WP01 | fixed; commit a2362c54; test audit_wp01_b007 |
| B008 | T48 | `code_outline` has no file or symbol-count bound | H | WP01 | fixed; commit a2362c54; test audit_wp01_b008 |
| B009 | T49 | Textual semantic fallback reports UTF-8 byte offsets as document columns | M | WP01 | fixed; commit a2362c54; test audit_wp01_b009 |
| B010 | T50 | Fallback outline selects the whole declaration instead of its name | M | WP01 | fixed; commit a2362c54; test audit_wp01_b010 |
| B011 | T51 | Textual reference fallback matches symbol substrings | M | WP01 | fixed; commit a2362c54; test audit_wp01_b011 |
| B012 | T52 | Textual reference fallback reports at most one occurrence per line | M | WP01 | fixed; commit a2362c54; test audit_wp01_b012 |
| B013 | T53 | Semantic workspace search follows directory junctions outside the root | H | WP01 | fixed; commit a2362c54; test audit_wp01_b013 |
| B014 | T54 | `ls` materializes the whole directory before applying its result limit | H | WP01 | fixed; commit a2362c54; test audit_wp01_b014 |
| B015 | T55 | Native `grep` fallback collects every candidate path before scanning | H | WP01 | fixed; commit a2362c54; test audit_wp01_b015 |
| B016 | T57 | Textual semantic fallbacks silently label capped results complete | M | WP01 | fixed; commit a2362c54; test audit_wp01_b016 |
| B017 | T59 | Root repository instructions have no size bound and are mandatory prompt context | H | WP01 | fixed; commit a2362c54; test audit_wp01_b017 |
| B018 | T124 | Oversized semantic positions wrap to unrelated source locations | M | WP01 | fixed; commit a2362c54; test audit_wp01_b018 |
| B019 | T125 | Oversized grep context panics in debug and silently drops matches in release | H | WP01 | fixed; commit a2362c54; test audit_wp01_b019 |
| B020 | T127 | Native `find` and `grep` fallbacks can stack-overflow on a long glob in debug builds | H | WP01 | fixed; commit a2362c54; test audit_wp01_b020 |
| B021 | T1 | Numeric background-job IDs wrap to 32 bits | H | WP02 | fixed; commit ac3c3450; test audit_wp02_b021 |
| B022 | T2 | Huge `job_output.wait` panics the caller | H | WP02 | fixed; commit ac3c3450; test audit_wp02_b022 |
| B023 | #147 | Task tools reject schema-valid integral decimal numbers | M | WP02 | fixed; commit ac3c3450; test audit_wp02_b023 |
| B024 | #148 | Task journals and quarantined task tails can be group-readable on Unix | H | WP02 | fixed; commit ac3c3450; test audit_wp02_b024 |
| B025 | #149 | A successful read-only receipt can satisfy the required Implementation dimension | H | WP02 | fixed; commit ac3c3450; test audit_wp02_b025 |
| B026 | T75 | Context VM reuses a user goal after a history event changes role | M | WP02 | fixed; commit ac3c3450; test audit_wp02_b026 |
| B027 | T145 | Session task journal initializes and writes through hard links | H | WP02 | fixed; commit ac3c3450; test audit_wp02_b027 |
| B028 | T146 | task_create hides supported decision prerequisites from the model schema | M | WP02 | fixed; commit ac3c3450; test audit_wp02_b028 |
| B029 | #227 | `/import` accepts malformed message content and silently omits it from context | M | WP02 | fixed; commit ac3c3450; test audit_wp02_b029 |
| B030 | #229 | Context inspector labels user history as tool evidence | L | WP02 | fixed; commit ac3c3450; test audit_wp02_b030 |
| B031 | #241 | Incomplete workspace snapshots mislabel read-only shell calls as file changes | L | WP02 | fixed; commit ac3c3450; test audit_wp02_b031 |
| B032 | #289 | Graph rejects successful npm test wrapper around Python unittest | M | WP02 | fixed; commit ac3c3450; test audit_wp02_b032 |
| B033 | #304 | RPC automatic retries ignore server Retry-After delays | M | WP02 | fixed; commit ac3c3450; test audit_wp02_b033 |
| B034 | #344 | Bare workflow model overrides misreport `agent_status` provider/model | L | WP02 | fixed; commit ac3c3450; test audit_wp02_b034 |
| B035 | T6 | Provider token counters can overflow and panic | H | WP03 | fixed; commit cde08cb4; test audit_wp03_b035 |
| B036 | #150 | Public CBOR encoder emits map values its own decoder rejects | M | WP03 | fixed; commit cde08cb4; test audit_wp03_b036 |
| B037 | T84 | Telemetry report aggregation overflows on valid `u64` values | H | WP03 | fixed; commit cde08cb4; test audit_wp03_b037 |
| B038 | T31 | Documented `PI_SESSION_DIR` does not control normal session storage | M | WP04 | fixed; commit d0bf7974; test audit_wp04_b038 |
| B039 | T67 | Session statistics overflow on persisted token totals | H | WP04 | fixed; commit d0bf7974; test audit_wp04_b039 |
| B040 | T73 | Reopening a session materializes its entire JSONL file in memory | H | WP04 | fixed; commit d0bf7974; test audit_wp04_b040 |
| B041 | T83 | SQLite import wraps valid unsigned session fields into signed storage | M | WP04 | fixed; commit d0bf7974; test audit_wp04_b041 |
| B042 | T116 | V3 migration normalizes impossible calendar dates into wrong session timestamps | M | WP04 | fixed; commit d0bf7974; test audit_wp04_b042 |
| B043 | T144 | Runtime session log follows hard links and truncates/appends outside the session file | H | WP04 | fixed; commit d0bf7974; test audit_wp04_b043 |
| B044 | #215 | Malformed explicit session file is reported as missing | L | WP04 | fixed; commit d0bf7974; test audit_wp04_b044 |
| B045 | #301 | `/cost` rounds some midpoint totals down | L | WP04 | fixed; commit d0bf7974; test audit_wp04_b045 |
| B046 | T97, #206 | `--tools` allowlist is bypassed by later native/extension registration | H | WP05 | fixed; commit 0b70a758; test audit_wp05_b046 |
| B047 | T95 | `--no-tools` re-enables native and extension tools during startup | H | WP05 | fixed; commit 0b70a758; test audit_wp05_b047 |
| B048 | T96 | `--exclude-tools` does not exclude native/extension tools loaded later | M | WP05 | fixed; commit 0b70a758; test audit_wp05_b048 |
| B049 | T113 | `--fail-on-denied` ignores policy-denied actions in read-only mode | M | WP05 | fixed; commit 0b70a758; test audit_wp05_b049 |
| B050 | #214 | RPC and TUI shell commands bypass execution-sandbox restrictions | H | WP05 | fixed; commit 0b70a758; no named regression added |
| B051 | #225 | Default TUI ignores project settings under run-scoped `--approve` | L | WP05 | already satisfied by merged code |
| B052 | #280 | `--api-key` persists credential after `--no-session` run | M | WP05 | fixed; commit 0b70a758; no named regression added |
| B053 | #362 | `--approve` does not unlock existing project memory for print-mode `/memory-page` | M | WP05 | already satisfied by merged code |
| B054 | T119, #195 | Local package install bypasses project trust in a pristine folder and then blocks its own removal | H | WP06 | fixed; commit 69f45df8; test audit_wp06_b054 |
| B055 | T27 | Persistent extension output lines have no size cap | H | WP06 | fixed; commit 69f45df8; no named regression added |
| B056 | #138 | Package symbol lookup can follow a junction outside the workspace | H | WP06 | held (see pr-concerns.md) |
| B057 | #142 | Workspace snapshot junctions redirect checkpoint writes and retention deletes | H | WP06 | held (see pr-concerns.md) |
| B058 | T108 | Plugin hook approval remains valid after same-size changes to large or `node_modules` files | M | WP06 | fixed; commit 69f45df8; test audit_wp06_b058 |
| B059 | T65 | Package resource manifests can load skill files outside the package root | M | WP06 | fixed; commit 69f45df8; test audit_wp06_b059 |
| B060 | T66 | Manifest-declared direct-file extensions are collected but never loaded | M | - | held (see pr-concerns.md) |
| B061 | #232 | `/reload` hides configured missing or malformed JavaScript extensions | L | WP06 | held (see pr-concerns.md) |
| B062 | T17 | Malformed plugin marketplace settings look like no marketplaces | M | WP06 | fixed; commit 69f45df8; test audit_wp06_b062 |
| B063 | T18 | Malformed remote marketplace catalog is treated as absent | M | WP06 | fixed; commit 69f45df8; test audit_wp06_b063 |
| B064 | T118 | Adding a same-name marketplace silently replaces the existing registration | M | WP06 | fixed; commit 69f45df8; test audit_wp06_b064 |
| B065 | T120 | Marketplace parser rejects valid extensionless git:// URLs | M | WP06 | fixed; commit 69f45df8; test audit_wp06_b065 |
| B066 | #323 | First `/plugins` Discover fails to seed marketplaces in a fresh profile | M | - | held (see pr-concerns.md) |
| B067 | T186, T87 | Windows paste fallback submits short pasted text ending in a newline | M | - | held (see pr-concerns.md) |
| B068 | T88 | History loading accepts arbitrarily large prompt entries | M | WP07 | fixed; commit 386f0f5f; test audit_wp07_b068 |
| B069 | T126 | Markdown entity decoding bypasses assistant transcript terminal sanitization | H | WP07 | fixed; commit 386f0f5f; test audit_wp07_b069 |
| B070 | T133 | TUI transcript entries render untrusted text without terminal sanitization | H | WP07 | held (see pr-concerns.md) |
| B071 | T187 | Windows paste fallback treats a tab-only clipboard paste as autocomplete | H | WP07 | held (see pr-concerns.md) |
| B072 | #330 | `/login` API key remains in prompt history after logout | H | WP07 | held (see pr-concerns.md) |
| B073 | T5 | JSON HTTP response bodies have no practical size limit | H | WP08 | fixed; commit ed548ac6; no named regression added |
| B074 | T28 | Extreme device-login polling intervals panic during duration conversion | H | WP08 | fixed; commit ed548ac6; test audit_wp08_b074 |
| B075 | #72 | OAuth denial callbacks leave the CLI waiting until timeout | M | WP08 | fixed; commit ed548ac6; test audit_wp08_b075 |
| B076 | T68 | Provider SSE parser dispatches an incomplete final event at EOF | M | WP08 | fixed; commit ed548ac6; test audit_wp08_b076 |
| B077 | T69 | Provider SSE reader cannot parse valid CR-only line endings | M | WP08 | fixed; commit ed548ac6; test audit_wp08_b077 |
| B078 | T70 | Provider SSE parser drops a leading UTF-8 BOM event | M | WP08 | fixed; commit ed548ac6; test audit_wp08_b078 |
| B079 | #293 | Interrupted Codex turn poisons session continuation | M | - | held (see pr-concerns.md) |
| B080 | #305 | RPC automatic retry omits HTTP 408 and 409 | M | WP08 | fixed; commit ed548ac6; test audit_wp08_b080 |
| B081 | T11 | Repeating RPC `Hello` leaks the previous connection association | M | - | held (see pr-concerns.md) |
| B082 | T12 | Configured RPC handshake timeout is unused | M | - | held (see pr-concerns.md) |
| B083 | T33 | HTTP redirects leak custom API-key headers across origins | H | WP09 | fixed; commit 5645b919; no named regression added |
| B084 | T36 | Framed client event inbox has no bound | H | WP09 | held (see pr-concerns.md) |
| B085 | T60 | MCP HTTP transport rejects valid case-variant SSE media types | M | WP09 | fixed; commit 5645b919; test audit_wp09_b085 |
| B086 | T61 | MCP SSE parsing does not split standalone CR line endings | M | WP09 | fixed; commit 5645b919; test audit_wp09_b086 |
| B087 | T62 | MCP SSE parsers do not strip a leading UTF-8 BOM | M | WP09 | fixed; commit 5645b919; test audit_wp09_b087 |
| B088 | T63 | MCP HTTP accepts an SSE event truncated before its blank-line terminator | M | WP09 | fixed; commit 5645b919; test audit_wp09_b088 |
| B089 | #155 | RPC server snapshot cache keeps stale session metadata after prompts | M | - | held (see pr-concerns.md) |
| B090 | #153 | RPC server ignores cached remote model catalogs during discovery and prompt resolution | M | - | held (see pr-concerns.md) |
| B091 | #154 | RPC `steer` and `abort` cannot interrupt a running prompt | M | - | held (see pr-concerns.md) |
| B092 | T81 | `davinci client` reports connected without completing the handshake | M | - | held (see pr-concerns.md) |
| B093 | T82 | `davinci server` exits after its first client disconnects | M | - | held (see pr-concerns.md) |
| B094 | #275 | Escape cancellation waits for the MCP call timeout | M | - | held (see pr-concerns.md) |
| B095 | T26 | Benchmark validator discovers generated grader cache files | M | WP10 | fixed; commit ec941ec1; no named regression added |
| B096 | #157 | Unix installer reports whichever `davinci` is first on PATH, not the binary it installed | M | WP10 | fixed; commit ec941ec1; no named regression added |
| B097 | T123 | Clean-checkout model-data hydration fails when the required `kimi-coding` provider is absent upstream | M | - | held (see pr-concerns.md) |
| B098 | T134 | Windows installer deletes the working binary before replacement succeeds | H | WP10 | held (see pr-concerns.md) |
| B099 | T135 | Concurrent design-runtime setup runs collide on a one-second directory name | M | - | held (see pr-concerns.md) |
| B100 | T15, T42, T90, #196 | Auth --min-expiry overflows during duration parsing | H | WP11 | fixed; commit 5efca73d; test audit_wp11_b100 |
| B101 | T137, T110 | Text `--print` writes model terminal controls directly to stdout | H | WP11 | fixed; commit 5efca73d; no named regression added |
| B102 | T34 | Graph export leaks credential defaults under generic parameter names | H | WP11 | fixed; commit 5efca73d; test audit_wp11_b102 |
| B103 | T35 | Opening exported sessions can fetch attacker-chosen Markdown images | H | WP11 | fixed; commit 5efca73d; no named regression added |
| B104 | T109 | Marketplace and model metadata are printed with terminal controls intact | H | WP11 | held (see pr-concerns.md) |
| B105 | T115 | RPC stdin accepts unbounded command lines and queues input without backpressure | H | WP11 | held (see pr-concerns.md) |
| B106 | #203 | HTML export embeds hidden custom-message content | M | WP11 | fixed; commit 5efca73d; test audit_wp11_b106 |
| B107 | T13 | LSP unregister batch partially mutates state on invalid input | M | WP12 | fixed; commit d09895a3; no named regression added |
| B108 | T56 | Output-schema `enum` and `const` reject mathematically equal JSON numbers | M | WP12 | fixed; commit d09895a3; test audit_wp12_b108 |
| B109 | #132 | Security source search enforces its 256-character query limit as bytes | M | WP12 | fixed; commit d09895a3; no named regression added |
| B110 | T64 | Output-schema `minItems` and `maxItems` reject valid integral decimals | M | WP12 | fixed; commit d09895a3; test audit_wp12_b110 |
| B111 | T128 | Worst-case `code_query` text fallback stalls for seconds on a small query and 4 MB of text | M | - | held (see pr-concerns.md) |
| B112 | T129 | Quoted empty `code_query` literal matches every text line | M | - | held (see pr-concerns.md) |
| B113 | T130 | `code_query` applies its 2,048-character query limit as a UTF-8 byte limit | M | WP12 | fixed; commit d09895a3; no named regression added |
| B114 | T131 | Managed browser path, text, and selector limits count UTF-8 bytes instead of characters | M | WP12 | fixed; commit d09895a3; no named regression added |
| B115 | T161 | `verification_plan` rejects schema-valid non-ASCII paths using UTF-8 byte length | M | WP12 | fixed; commit d09895a3; no named regression added |
| B116 | T162 | Language-intelligence text limits count UTF-8 bytes instead of schema characters | M | WP12 | fixed; commit d09895a3; no named regression added |
| B117 | T168 | `verification_plan` accepts user requirements longer than its schema limit | M | WP12 | fixed; commit d09895a3; no named regression added |
| B118 | T171 | Test-impact planning rejects schema-valid Unicode paths | M | WP12 | fixed; commit d09895a3; no named regression added |
| B119 | T175 | Git intelligence accepts zero for positive result limits | M | WP12 | fixed; commit d09895a3; no named regression added |
| B120 | T176 | Test-impact schema permits more combined inputs than runtime | M | WP12 | fixed; commit d09895a3; no named regression added |
| B121 | T177 | `git_related_commits` ignores `symbol` when `query` is also supplied | M | WP12 | fixed; commit d09895a3; no named regression added |
| B122 | T178 | `git_related_commits.symbol` searches messages instead of historical code | M | WP12 | fixed; commit d09895a3; no named regression added |
| B123 | T160 | Workspace snapshot path limits count UTF-8 bytes instead of schema characters | M | WP13 | fixed; commit f9c86647; test audit_wp13_b123 |
| B124 | T170 | Workspace snapshots persist labels far beyond their advertised limit | M | WP13 | fixed; commit f9c86647; test audit_wp13_b124 |
| B125 | T179 | Change impact accepts Windows absolute paths as workspace-relative | M | WP13 | fixed; commit f9c86647; test audit_wp13_b125 |
| B126 | T180 | Change-impact analysis fails to normalize equivalent relative paths and scopes | M | WP13 | fixed; commit f9c86647; test audit_wp13_b126 |
| B127 | T181 | Empty change-impact symbols suppress working-tree discovery | M | WP13 | fixed; commit f9c86647; test audit_wp13_b127 |
| B128 | T182 | Change-impact working-tree discovery omits staged files before the first Git commit | M | WP13 | fixed; commit f9c86647; test audit_wp13_b128 |
| B129 | T183 | Workspace diff does not enforce its 128-path request limit | M | WP13 | fixed; commit f9c86647; test audit_wp13_b129 |
| B130 | T184 | Workspace checkpoint rejects schema-valid path lists above the default 64-file cap | M | WP13 | fixed; commit f9c86647; test audit_wp13_b130 |
| B131 | #139, #352 | `package_symbol` reads declaration files larger than its stated 512 KiB limit | H | WP14 | fixed; commit 10d9a47b; no named regression added |
| B132 | #140 | Package export inspection drops nested `import` and `require` conditions | M | - | held (see pr-concerns.md) |
| B133 | #141 | Package type lookup misses `@types` installed in the selected workspace | M | - | held (see pr-concerns.md) |
| B134 | #351 | `package_symbol` reports a longer prefix match as the requested symbol | L | WP14 | fixed; commit 10d9a47b; no named regression added |
| B135 | #357 | `package_symbol` ignores TypeScript `typesVersions` redirects | L | WP14 | held (see pr-concerns.md) |
| B136 | #358 | `package_why` misses transitive npm peer dependency reasons | L | WP14 | held (see pr-concerns.md) |
| B137 | #359 | `package_dependents` returns inaccurate source references | L | WP14 | held (see pr-concerns.md) |
| B138 | #360 | `package_exports` loses array and null export targets | L | WP14 | held (see pr-concerns.md) |
| B139 | T172 | Graph worker cancellation exceeds its 500 ms responsiveness bound on Windows | M | - | held (see pr-concerns.md) |
| B140 | #291 | Graph run sheet pause, stop, and retry controls do not reach controller | M | - | held (see pr-concerns.md) |
| B141 | #298 | Failed graph checkpoint leaves persisted run reported as running | L | WP15 | held (see pr-concerns.md) |
| B142 | #326 | Graph dry-run reports verification passed without running checks | M | - | held (see pr-concerns.md) |
| B143 | #327 | Saving a generated Graph run creates an unrunnable definition | M | - | held (see pr-concerns.md) |
| B144 | #328 | Graph subcommand results are hidden behind the previous run view | M | - | held (see pr-concerns.md) |
| B145 | #331 | Graph completion details corrupt Windows path to `.davinci` | M | - | held (see pr-concerns.md) |
| B146 | #356 | Graph resume cannot recover an interrupted active writer | M | - | held (see pr-concerns.md) |
| B147 | T91 | Learning candidate ID actions ignore candidate status | M | - | held (see pr-concerns.md) |
| B148 | T92 | Learning approval hides a failed durable-memory write | H | WP16 | held (see pr-concerns.md) |
| B149 | T93 | A later learning approval restores memories erased by `/memory-clear` | H | WP16 | held (see pr-concerns.md) |
| B150 | T143 | Zero-byte token-governor retrieval budget makes compressed output unrecoverable | M | - | held (see pr-concerns.md) |
| B151 | #201 | `/learning-reject` treats documented reason as candidate ID | M | - | held (see pr-concerns.md) |
| B152 | #274 | Learning pending confidence shows expanded binary float | L | WP16 | held (see pr-concerns.md) |
| B153 | #200, #292 | Hidden custom messages appear in user transcripts and resumed sessions | M | - | held (see pr-concerns.md) |
| B154 | T16 | `/resume` reports unreadable session storage as empty | M | - | held (see pr-concerns.md) |
| B155 | T32 | Resume hint omits a custom Davinci session directory | M | - | held (see pr-concerns.md) |
| B156 | T45 | Session-tree row builder discards parent/branch topology | M | - | held (see pr-concerns.md) |
| B157 | T46 | `/tree` never marks a completed current turn as current | L | WP17 | held (see pr-concerns.md) |
| B158 | T173 | `/new` creates persistent session files in `--no-session` ephemeral mode | M | - | held (see pr-concerns.md) |
| B159 | #216 | Startup `/resume` can exit when a listed session has a malformed entry | L | WP17 | held (see pr-concerns.md) |
| B160 | #217 | Startup `--resume` rename does not persist | L | WP17 | held (see pr-concerns.md) |
| B161 | #218 | Startup `--resume` delete confirmation does not delete | M | - | held (see pr-concerns.md) |
| B162 | #231 | Context inspector pin/exclude controls are local to open sheet | M | - | held (see pr-concerns.md) |
| B163 | #324 | `/import` rejects quoted paths | L | WP17 | held (see pr-concerns.md) |
| B164 | T25, T104, #211 | CLI value options accept missing values or consume the next flag | M | - | held (see pr-concerns.md) |
| B165 | T22, #105 | Package subcommands accept unexpected arguments | M | - | held (see pr-concerns.md) |
| B166 | T23, T98, T99 | `update` silently ignores malformed selectors and options | M | - | held (see pr-concerns.md) |
| B167 | T24, T100, T101 | Model refresh conflates provider filtering and package targets | M | - | held (see pr-concerns.md) |
| B168 | T107, #199 | `davinci config` prints the selector panel and exits instead of opening an interactive selector | M | - | held (see pr-concerns.md) |
| B169 | T21 | `davinci config --local` reads the wrong settings scope | M | - | held (see pr-concerns.md) |
| B170 | T102 | A leading `--offline` prevents shell subcommand dispatch | M | - | held (see pr-concerns.md) |
| B171 | T103 | `--offline` inside prompt text bypasses the documented `--` option boundary | H | WP18 | held (see pr-concerns.md) |
| B172 | T114 | `--mode rpc` silently ignores positional prompt messages | M | - | held (see pr-concerns.md) |
| B173 | #197 | `update --extension <name>` rejects documented package name | M | - | held (see pr-concerns.md) |
| B174 | #223 | `DAVINCI_OFFLINE=1` does not prevent startup tool downloads | M | - | held (see pr-concerns.md) |
| B175 | #238 | Package install rejects valid `git://` URLs | M | WP18 | fixed; commit 6b066069; test audit_wp18_b175 |
| B176 | #345 | Versionless npm remove deletes files but leaves pinned registration | M | - | held (see pr-concerns.md) |
| B177 | #346 | Registered local package cannot be removed after its directory disappears | M | - | held (see pr-concerns.md) |
| B178 | #234 | RPC setters report success for invalid or missing values | M | - | held (see pr-concerns.md) |
| B179 | #235 | Invalid RPC queue modes silently reset to `all` | M | - | held (see pr-concerns.md) |
| B180 | #273 | Offline security report argument errors are masked by provider admission | M | - | held (see pr-concerns.md) |
| B181 | #342 | RPC `abort_retry` is deferred until active prompt ends | M | - | held (see pr-concerns.md) |
| B182 | #343 | Offline RPC prompts require a provider credential | M | - | held (see pr-concerns.md) |
| B183 | #353 | RPC accepts blank prompts and queue messages | L | WP18 | fixed; commit 6b066069; no named regression added |
| B184 | #361 | JSON print output contains a raw native-command line | M | - | held (see pr-concerns.md) |
| B185 | T117, #279 | Auth commands misinterpret unqualified model IDs as provider names | M | - | held (see pr-concerns.md) |
| B186 | T136, T111 | `/list-models` misaligns columns for Unicode model IDs | L | WP19 | held (see pr-concerns.md) |
| B187 | T194, #363 | Model catalog refresh treats an expected provider 404 as a fatal update error | M | - | held (see pr-concerns.md) |
| B188 | #151 | Scoped-model chooser drops cached remote models when one provider refresh fails | M | - | held (see pr-concerns.md) |
| B189 | #152 | Hidden built-in Codex models still appear in the available model list | M | - | held (see pr-concerns.md) |
| B190 | T89 | `auth check --credentials` omits header-only credentials | M | - | held (see pr-concerns.md) |
| B191 | #219 | `/doctor` labels existing settings file absent | L | WP19 | held (see pr-concerns.md) |
| B192 | #300 | `auth check` reports expired, unrefreshable OAuth credential as ready | M | - | held (see pr-concerns.md) |
| B193 | #314 | `/doctor` misses the legacy auth fallback | L | WP19 | held (see pr-concerns.md) |
| B194 | #321 | Configured API-key models are missing from `/model` picker | M | - | held (see pr-concerns.md) |
| B195 | #339 | `--list-models` does not perform documented fuzzy search | L | WP19 | fixed; commit bf750e20; test audit_wp19_b195 |
| B196 | #340 | Model cycling fails to enter configured scope from outside it | L | WP19 | held (see pr-concerns.md) |
| B197 | #341 | Thinking-level cycling skips first available level after `off` | L | WP19 | fixed; commit bf750e20; test audit_wp19_b197 |
| B198 | T7 | `/diff` can report a clean tree when Git status fails | M | - | held (see pr-concerns.md) |
| B199 | T8 | `/diff` omits untracked-file content | M | - | held (see pr-concerns.md) |
| B200 | T9 | `/diff` silently truncates after 24 paths | M | - | held (see pr-concerns.md) |
| B201 | T10 | `/diff` cannot show staged changes in an unborn Git repository | M | - | held (see pr-concerns.md) |
| B202 | T29 | Git porcelain parser corrupts quoted and arrow-containing paths | M | - | held (see pr-concerns.md) |
| B203 | T30 | `/diff` drops the working-tree half of Git status | M | - | held (see pr-concerns.md) |
| B204 | T77 | The current TUI editor shortcut ignores the configured `externalEditor` | M | - | held (see pr-concerns.md) |
| B205 | T78 | Clipboard paste forwards images rejected by the resize budget | H | WP21 | held (see pr-concerns.md) |
| B206 | T79 | TIFF clipboard images are mislabeled or dropped | H | WP21 | held (see pr-concerns.md) |
| B207 | T80 | Timed-out clipboard helpers keep running and accumulate | H | WP21 | held (see pr-concerns.md) |
| B208 | T121 | Windows external editor cannot launch executables whose path contains spaces | M | - | held (see pr-concerns.md) |
| B209 | T122 | Windows terminal image fallback displays home paths with backslashes | L | WP21 | held (see pr-concerns.md) |
| B210 | #237 | `/copy` reports success and empties clipboard for image-only replies | M | - | held (see pr-concerns.md) |
| B211 | #297 | Auto-resized `@file` images ignore EXIF orientation | M | - | held (see pr-concerns.md) |
| B212 | T37 | ANSI-aware wrapping can split an emoji grapheme across rows | L | WP22 | held (see pr-concerns.md) |
| B213 | T43 | TUI Tab path completion ignores the caret and breaks on spaces | M | - | held (see pr-concerns.md) |
| B214 | T76 | ANSI-aware truncation can cut emoji graphemes before its ellipsis | M | - | held (see pr-concerns.md) |
| B215 | T159 | TUI text selection maps terminal cells onto code points instead of graphemes | M | - | held (see pr-concerns.md) |
| B216 | T165 | Mermaid node borders are too short for double-width labels | M | - | held (see pr-concerns.md) |
| B217 | T169 | Composer command highlighting stops early for wide Unicode slash-command names | M | - | held (see pr-concerns.md) |
| B218 | T185 | Transcript typewriter animation splits Unicode grapheme clusters | M | - | held (see pr-concerns.md) |
| B219 | T188 | Terminal path-link detection splits file paths at spaces | M | - | held (see pr-concerns.md) |
| B220 | T189 | Terminal link detection rejects valid uppercase HTTP(S) schemes | M | - | held (see pr-concerns.md) |
| B221 | T190 | Word navigation splits combining marks outside the basic combining diacritic block | M | - | held (see pr-concerns.md) |
| B222 | T191 | Inline-code highlighting shifts after soft-wrapped user text | M | - | held (see pr-concerns.md) |
| B223 | T192 | Editor word navigation treats unspaced Thai and Lao text as one word | M | - | held (see pr-concerns.md) |
| B224 | T193 | User message rendering drops unmatched and escaped backticks | L | WP22 | held (see pr-concerns.md) |
| B225 | T163 | Extensions skill path truncation discards the path suffix for wide Unicode names | M | - | held (see pr-concerns.md) |
| B226 | T164 | Extensions rows truncate Unicode status text earlier than their cell budget | M | - | held (see pr-concerns.md) |
| B227 | T166 | Permission panel clips long file paths without exposing the target suffix | M | - | held (see pr-concerns.md) |
| B228 | #230 | Narrow fullscreen Ctrl-D confirmation is obscured | L | WP23 | held (see pr-concerns.md) |
| B229 | #240 | `/tasks` advertises unsupported selection and view controls | M | - | held (see pr-concerns.md) |
| B230 | #320 | Narrow `/permissions` header hides section labels | L | WP23 | held (see pr-concerns.md) |
| B231 | #322 | Plugin and MCP Discover hide install/view hints on narrow terminals | L | WP23 | held (see pr-concerns.md) |
| B232 | #325 | `/agents` hides profile list at 40Ã—12 | L | WP23 | held (see pr-concerns.md) |
| B233 | #329 | Narrow `/learning-pending` truncates target candidate IDs | L | WP23 | held (see pr-concerns.md) |
| B234 | #332 | `/memory-search` result omits matching text excerpt | L | WP23 | held (see pr-concerns.md) |
| B235 | #336 | `/mcp` reports a dead server as connected | M | - | held (see pr-concerns.md) |
| B236 | #285, #303 | `/config` cannot change project-scoped settings | M | - | held (see pr-concerns.md) |
| B237 | #302, #295 | TUI `/setup check --json` returns truncated JSON | M | - | held (see pr-concerns.md) |
| B238 | T14 | `/cost` displays unknown usage as zero | M | - | held (see pr-concerns.md) |
| B239 | T19 | Bare `/name` clears the session name | M | - | held (see pr-concerns.md) |
| B240 | #204 | `/name` gives incomplete confirmation when clearing name | L | WP24 | held (see pr-concerns.md) |
| B241 | #207 | Denied shell call is marked with success glyph | L | WP24 | held (see pr-concerns.md) |
| B242 | #208 | Auto Mode approval for workspace restore is denied internally | M | - | held (see pr-concerns.md) |
| B243 | #226 | Design enablement state is ignored by release TUI | M | - | held (see pr-concerns.md) |
| B244 | #236 | `/jobs` renders status messages as shell execution | L | WP24 | held (see pr-concerns.md) |
| B245 | #270 | Malformed saved workflow silently falls through to prompt execution | M | - | held (see pr-concerns.md) |
| B246 | #287 | Write approval mislabels directory target as file overwrite | L | WP24 | held (see pr-concerns.md) |
| B247 | #288 | TUI hyperlink truncates custom share viewer URL | L | WP24 | held (see pr-concerns.md) |
| B248 | #316 | `/sessions` compatibility alias is shadowed by `ecc:sessions` | M | - | held (see pr-concerns.md) |
| B249 | #333 | TUI labels local `/design` usage errors as model failures | L | WP24 | held (see pr-concerns.md) |
| B250 | #334 | `/sandbox-status` joins enforcement and permission lines | L | WP24 | held (see pr-concerns.md) |
| B251 | #347 | Prompt-template filenames with spaces register but do not dispatch | M | - | held (see pr-concerns.md) |
| B252 | #348 | `/hotkeys` displays a raw terminal escape sequence in shortcut labels | L | WP24 | held (see pr-concerns.md) |
| B253 | #349 | `/design-sync` treats commented-out text as design facts | M | - | held (see pr-concerns.md) |
| B254 | #350 | `/graph runs` search drops uppercase letters | L | WP24 | held (see pr-concerns.md) |
| B255 | T112, #213, #337 | CLI theme and TUI-mode flags are ignored by the default DaVinci shell | L | WP25 | held (see pr-concerns.md) |
| B256 | T20 | Interactive `/help` omits host-handled built-in commands | L | WP25 | held (see pr-concerns.md) |
| B257 | T158 | `--prompt-profile` is accepted and documented but missing from `davinci --help` | M | - | held (see pr-concerns.md) |
| B258 | T174 | `/graph` autocomplete omits saved-graph commands | L | WP25 | held (see pr-concerns.md) |
| B259 | #202 | `/autocompact` argument hint misstates valid inputs | L | WP25 | held (see pr-concerns.md) |
| B260 | #296 | UI design docs advertise retired `/settings` command | L | WP25 | held (see pr-concerns.md) |
| B261 | #307 | Top-level help advertises IPC commands whose help is unavailable on Windows | L | WP25 | held (see pr-concerns.md) |
| B262 | #319 | RPC advertises JSON-RPC but uses a proprietary envelope | M | - | held (see pr-concerns.md) |
| B263 | T44, #44, #315 | `/tree` cannot extract text from normal session messages | M | - | held (see pr-concerns.md) |
| B264 | T74, #259 | `--no-context-files` help falsely claims Davinci loads `CLAUDE.md` | L | â€” | held (see pr-concerns.md) |
| B265 | T71 | OAuth callback host and port overrides diverge from provider redirect URIs | M | - | held (see pr-concerns.md) |
| B266 | #205 | One installed skill is counted as 75 and cannot be removed as a unit | M | - | held (see pr-concerns.md) |
| B267 | #209 | `/import` accepts broken session ancestry | M | â€” | fixed (pre-existing audit commit) |
| B268 | #212 | `/config` search shortcut inserts its own slash | L | â€” | skipped (source-based reason in addendum) |
| B269 | #220 | Settings-changing commands fail to acquire profile lock | M | â€” | skipped (source-based reason in addendum) |
| B270 | #221 | `/cost` merges accounting footer into preceding TUI line | L | â€” | held (see pr-concerns.md) |
| B271 | #222 | Exported HTML omits foreground theme colors | M | - | held (see pr-concerns.md) |
| B272 | #228 | Near-full imported context stalls TUI command handling | M | â€” | skipped (source-based reason in addendum) |
| B273 | #233 | `/tasks` omits task-count metadata | L | â€” | fixed (pre-existing audit commit) |
| B274 | #239 | Resumed sessions drop `turn_started` after an interrupted turn | M | - | held (see pr-concerns.md) |
| B275 | #242 | `/config` narrow settings layout wraps columns and truncates controls | L | â€” | held (see pr-concerns.md) |
| B276 | #243 | `/context` inspector clips scroll controls at 50Ã—16 | L | â€” | held (see pr-concerns.md) |
| B277 | #244 | `/workflow-stop` reports completed runs as stopped | L | â€” | held (see pr-concerns.md) |
| B278 | #245 | Selected value-taking equals forms are silently ignored | H | â€” | held (see pr-concerns.md) |
| B279 | #246 | Explicit `--extension` file is not loaded | M | - | held (see pr-concerns.md) |
| B280 | #247 | Transcript scroll hint advertises End but requires Ctrl+End | L | â€” | held (see pr-concerns.md) |
| B281 | #248 | Saved-workflow trust error recommends nonexistent slash command | L | â€” | held (see pr-concerns.md) |
| B282 | #249 | Project-saved workflow remains blocked after trust is recorded | M | - | held (see pr-concerns.md) |
| B283 | #250 | `/jobs kill` leaves its PowerShell process running | H | â€” | fixed (pre-existing audit commit) |
| B284 | #251 | Stopped background agent is shown as completed | M | - | held (see pr-concerns.md) |
| B285 | #252 | Marketplace subcommand `--help` is parsed as an operand | L | â€” | held (see pr-concerns.md) |
| B286 | #253 | `--no-themes` does not suppress theme discovery | L | â€” | held (see pr-concerns.md) |
| B287 | #254 | Default TUI does not render verbose startup resources | L | â€” | held (see pr-concerns.md) |
| B288 | #255 | `@file` errors duplicate the error prefix | L | â€” | held (see pr-concerns.md) |
| B289 | #256 | Positional `@directory` is silently ignored | M | - | held (see pr-concerns.md) |
| B290 | #257 | Doubled `@` silently selects a different file | M | - | held (see pr-concerns.md) |
| B291 | #258 | Package root files are registered as prompts and skills | M | - | held (see pr-concerns.md) |
| B292 | #260 | `--models` help names the wrong shortcut | L | â€” | held (see pr-concerns.md) |
| B293 | #271 | `/sec-status` empty state looks like an empty security report | L | â€” | held (see pr-concerns.md) |
| B294 | #272 | Completed security scan status and report views disagree | M | - | held (see pr-concerns.md) |
| B295 | #278 | Authenticated Design generation is rejected before missing-runtime setup gate | M | - | held (see pr-concerns.md) |
| B296 | #282 | Authenticated GPT-6 Luna TUI delay is intermittent | M | - | held (see pr-concerns.md) |
| B297 | #286 | Successful model-facing `agent_stop` is reported as an error | L | â€” | fixed (pre-existing audit commit) |
| B298 | #290 | Interrupted security scan loses source-read tool on resume | M | â€” | fixed (pre-existing audit commit) |
| B299 | #294 | Project trust docs advertise `/trust` as a command | L | â€” | held (see pr-concerns.md) |
| B300 | #299 | Authenticated GPT-6 Luna/medium security scans fail before reviewing files | M | - | held (see pr-concerns.md) |
| B301 | #313 | Skills Discover shows an inflated â€œbelowâ€ count | L | â€” | held (see pr-concerns.md) |
| B302 | #354 | Recovered tool call can stall when its ID is reissued | M | â€” | skipped (source-based reason in addendum) |
| B303 | #355 | Current worktree release build fails at compaction caller | M | - | held (see pr-concerns.md) |
| B304 | #156 | Windows transactional writes fail when NTFS 8.3 names are disabled | H | â€” | skipped (source-based reason in addendum) |
