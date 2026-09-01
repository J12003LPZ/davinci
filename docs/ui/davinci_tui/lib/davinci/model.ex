defmodule Davinci.Model do
  @moduledoc """
  Application state.

  `screen` is the single instrument in hand — one panel at a time (design.md
  §1). `overlay` is the narrower class of surface that floats over a dimmed
  transcript (palette, session picker, model picker). `codex` is the only
  persistent sidebar, and it is opt-in at ≥120 columns.

  The transcript, plan and instrument contents below are fixtures standing in
  for real session state; swap them for your session store without touching the
  view modules.
  """

  alias Davinci.{Term, Theme}

  defstruct width: 100,
            height: 40,
            tick: 0,
            theme: nil,
            color_mode: :basic,
            animate: true,
            screen: :agent,
            overlay: nil,
            codex: false,
            running: true,
            composer: "",
            query: "git",
            palette_index: 0,
            session_index: 0,
            model_index: 0,
            recall_index: 0,
            catalog_index: 0,
            settings_index: 0,
            thinking_index: 3,
            login_index: 0,
            keys_offset: 0,
            resume_index: 0,
            tree_index: 4,
            security_index: 0,
            diff_index: 0,
            transcript: [],
            cwd: "C:\\dev\\oss\\davinci-rust",
            branch: "main",
            model_name: "sonnet",
            changes: {3, 42, 11},
            context: {47_000, 200_000}

  def new(context) do
    window = Map.get(context, :window, %{})
    color_mode = Term.enable_256_colors()

    %__MODULE__{
      width: Map.get(window, :width, 100),
      height: Map.get(window, :height, 40),
      color_mode: color_mode,
      animate: Term.animate?(),
      theme: Theme.new(color_mode, Term.no_color?()),
      transcript: transcript()
    }
  end

  # --- responsive breakpoints (design.md §7) --------------------------------

  def narrow?(model), do: model.width < 100
  def minimal?(model), do: model.width < 80
  def sidebar_allowed?(model), do: model.width >= 120
  def wide?(model), do: model.width >= 150
  def decoration?(model), do: model.width >= 100

  def codex_open?(model), do: model.codex and sidebar_allowed?(model)

  def mode(model) do
    cond do
      model.screen == :plan -> "plan"
      model.screen == :grafo -> "grafo"
      model.screen == :memoria -> "memoria"
      model.screen == :mensura -> "mensura"
      model.screen == :models -> "cogitator"
      model.screen == :thinking -> "cogitator"
      model.screen == :login -> "cogitator"
      model.screen == :settings -> "settings"
      model.screen == :keys -> "keys"
      model.screen in [:resume, :tree, :export, :vectors] -> "memoria"
      model.screen in [:compact, :governor] -> "mensura"
      model.screen == :graph_run -> "grafo"
      model.screen == :securitas -> "securitas"
      model.screen == :trust -> "fiducia"
      model.screen == :officina -> "officina"
      model.screen in [:recovery, :diff] -> "agent"
      model.overlay == :sessions -> "memoria"
      model.overlay == :cogitator -> "cogitator"
      true -> "agent"
    end
  end

  def blink?(%{animate: false}), do: true
  def blink?(model), do: rem(div(model.tick, 4), 2) == 0

  @doc "Every surface states its own context cost (design.md §9)."
  def context(model) do
    case model.screen do
      :plan -> {62_000, 200_000}
      :memoria -> {48_000, 200_000}
      :mensura -> {128_000, 200_000}
      _ -> if codex_open?(model), do: {78_000, 200_000}, else: model.context
    end
  end

  def context_fraction(model) do
    {used, cap} = context(model)
    used / cap
  end

  # --- composer -------------------------------------------------------------

  def type(model, string), do: %{model | composer: model.composer <> string}

  def backspace(%{composer: ""} = model), do: model

  def backspace(model) do
    %{model | composer: String.slice(model.composer, 0, String.length(model.composer) - 1)}
  end

  def submit(%{composer: ""} = model), do: model

  def submit(model) do
    turn = String.trim(model.composer)

    case command(turn) do
      {:ok, screen} -> %{model | composer: "", screen: screen, overlay: nil}
      :error -> append(model, turn)
    end
  end

  # The configuration surfaces are opened the way the product opens them —
  # by name, from the composer — rather than by inventing another ctrl key.
  @commands %{
    "/model" => :models,
    "/settings" => :settings,
    "/thinking" => :thinking,
    "/login" => :login,
    "/hotkeys" => :keys,
    "/resume" => :resume,
    "/tree" => :tree,
    "/compact" => :compact,
    "/export" => :export,
    "/graph" => :graph_run,
    "/memory-status" => :vectors,
    "/governor-status" => :governor,
    "/sec-report" => :securitas,
    "/trust" => :trust,
    "/reload" => :officina,
    "/diff" => :diff
  }

  def command(""), do: :error

  def command(line) do
    case String.split(line, ~r/\s+/, trim: true) do
      [name | _rest] -> Map.fetch(@commands, name)
      [] -> :error
    end
  end

  defp append(model, ""), do: model

  defp append(model, turn) do
    %{
      model
      | composer: "",
        running: true,
        screen: :agent,
        overlay: nil,
        transcript: model.transcript ++ [:gap, {:user, turn} | reply(turn)]
    }
  end

  # ctrl+c stops the run and shows what it stopped — what was kept, what was
  # billed, what is still on disk (6c). It never touches the app.
  def interrupt(model), do: %{model | running: false, screen: :recovery, overlay: nil}

  defp reply(_turn) do
    [
      :gap,
      {:agent, "davinci"},
      {:tool, :read, "instrumenta", "read crates\\davinci-session\\src\\store.rs", nil},
      {:studio,
       [
         {:done, "surveyed workspace", nil},
         {:active, "measuring the change", "crates\\davinci-session\\src\\store.rs"},
         {:queued, "verify workspace: fmt, clippy, test", nil}
       ]}
    ]
  end

  # --- fixtures -------------------------------------------------------------

  @doc "1b — transcript with tools, Studio and a Δ block."
  def transcript do
    [
      {:user, "explain how the agent runtime works"},
      :gap,
      {:agent, "davinci"},
      {:tool, :read, "instrumenta", "read crates\\davinci-agent\\src\\lib.rs", nil},
      {:tool, :search, "instrumenta", "search \"SessionManager\" · 8 matches", nil},
      {:tool, :done, "manus", "cargo check -p davinci-agent", "1.84s"},
      {:tool, :failed, "manus", "cargo test -p davinci-session", "0.42s"},
      {:detail, "error[E0308] mismatched types · store.rs:118"},
      :gap,
      {:studio,
       [
         {:done, "surveyed workspace", nil},
         {:done, "traced request pipeline", nil},
         {:active, "examining session persistence",
          "davinci-session\\src\\store.rs"},
         {:queued, "verify provider abstraction", nil}
       ]},
      :gap,
      {:prose,
       "A request enters davinci-agent as a Turn, is planned, then dispatched " <>
         "to the provider adapter. Session state is written after every tool " <>
         "call, so an interrupt never loses the transcript."},
      :gap,
      {:delta, "crates\\davinci-agent\\src\\runtime.rs", 31, 8,
       [
         {:add, "pub async fn execute_stream("},
         {:add, "    &self, req: Request, tx: Sender<Chunk>,"},
         {:add, ") -> Result<Usage> {"},
         {:del, "    self.execute(req).await"}
       ]}
    ]
  end

  @doc "1e — the Codex transcript, a windows-path bug."
  def codex_transcript do
    [
      {:user, "why does the session store fail on windows paths?"},
      :gap,
      {:agent, "davinci"},
      {:tool, :search, "instrumenta", "search \"PathBuf::from\" · 12 matches", nil},
      {:tool, :read, "instrumenta", "read crates\\davinci-session\\src\\store.rs", nil},
      {:tool, :failed, "manus", "cargo test -p davinci-session", "0.42s"},
      :gap,
      {:prose,
       "The store joins session ids with a literal / instead of Path::join, so " <>
         "a restored path becomes C:\\Users\\ines\\.davinci/sessions/… " <>
         "Canonicalising it fixes both platforms."},
      :gap,
      {:delta, "crates\\davinci-session\\src\\store.rs", 6, 3,
       [
         {:del, "let p = format!(\"{}/sessions/{}\", root, id);"},
         {:add, "let p = root.join(\"sessions\").join(id);"}
       ]}
    ]
  end

  @doc "1g / 1h — the narrow transcript."
  def narrow_transcript do
    [
      {:user, "run the tests"},
      :gap,
      {:agent, "davinci"},
      {:tool, :done, "manus", "cargo fmt", "0.31s"},
      {:tool, :done, "manus", "cargo clippy", "4.10s"},
      {:tool, :failed, "manus", "cargo test", "6.72s"},
      {:detail, "1 failed store::roundtrip_windows_paths"}
    ]
  end

  @doc "1c — Disegno plan."
  def plan do
    [
      {"I", :done, "Map provider abstraction", "davinci-ai\\src\\provider.rs"},
      {"II", :done, "Trace session lifecycle", nil},
      {"III", :active, "Implement streaming adapter", nil},
      {"IV", :queued, "Add parity fixtures against TS davinci", nil},
      {"V", :queued, "Verify workspace: fmt, clippy, test", nil}
    ]
  end

  @doc "1d — Instrumenta corpus: tools, sessions, files, modes."
  def corpus do
    [
      {"/git status", "working tree · 3 modified", "tool"},
      {"/git diff", "unstaged hunks", "tool"},
      {"/git commit", "stage all · write message", "tool"},
      {"memoria: fix-git-hooks", "session · 2 days ago", "session"},
      {".gitignore", "C:\\dev\\oss\\davinci-rust", "file"},
      {"crates\\davinci-git\\src\\lib.rs", "414 lines", "file"},
      {"mode: git-worktree", "isolate edits per turn", "mode"}
    ]
  end

  def corpus_total, do: 214

  def filtered_corpus(%{query: ""}), do: corpus()

  def filtered_corpus(%{query: query}) do
    needle = String.downcase(query)

    case Enum.filter(corpus(), fn {name, desc, kind} ->
           String.contains?(String.downcase(name <> desc <> kind), needle)
         end) do
      [] -> []
      hits -> hits
    end
  end

  @doc "1f — Memoria sessions."
  def sessions do
    [
      {"review-agent-runtime", "3m"},
      {"implement-rpc-mode", "18m"},
      {"provider-parity", "1h"},
      {"tui-redesign", "yesterday"},
      {"fix-git-hooks", "2d"}
    ]
  end

  @doc "1f — Cogitator models."
  def models do
    [
      {"anthropic / sonnet", "200k"},
      {"anthropic / opus", "200k"},
      {"openai / gpt", "128k"},
      {"google / gemini", "1m"},
      {"local / ollama", "32k"}
    ]
  end

  @doc "1e — git changes popover."
  def changes_list do
    [
      {"M", "davinci-session\\src\\store.rs", "+6"},
      {"M", "davinci-agent\\src\\runtime.rs", "+31"},
      {"A", "davinci-tui\\src\\theme.rs", "+54"}
    ]
  end

  @doc "1e — Codex file tree, flattened to (depth, glyph, name, status)."
  def tree do
    [
      {0, "▾", "crates", nil},
      {1, "▸", "davinci-agent", :delta},
      {1, "▸", "davinci-ai", nil},
      {1, "▸", "davinci-client", nil},
      {1, "▾", "davinci-session", :delta},
      {2, nil, "store.rs", :failed},
      {2, nil, "manager.rs", nil},
      {1, "▸", "davinci-tui", :delta},
      {1, "▸", "davinci-git", nil},
      {0, nil, "Cargo.toml", nil},
      {0, nil, "Cargo.lock", nil},
      {0, nil, "README.md", nil}
    ]
  end

  @doc "2a — Grafo impact list."
  def impact do
    [
      {:active, "davinci-session::store", "direct", "6 call sites", :border},
      {:read, "davinci-session::manager", "1 hop", "2 call sites", :border},
      {:read, "davinci-agent::runtime", "2 hops", "1 call site", :border},
      {:read, "davinci-tui::app", "3 hops", "render only", :border},
      {:attention, "davinci-cli::main", "3 hops", "no test coverage", :warning}
    ]
  end

  @doc "2b — Memoria recall hits."
  def recall do
    [
      {"0.91", "store::roundtrip: write after every tool call",
       "davinci-session\\src\\store.rs:118", 0.91, "turn 12 · promoted"},
      {"0.87", "manager::resume rebuilds the transcript",
       "davinci-session\\src\\manager.rs:44", 0.87, "turn 12 · promoted"},
      {"0.74", "notes: \"interrupts must not truncate memoria\"",
       "memoria\\decisions.md:9", 0.74, "session · tui-redesign"},
      {"0.61", "test roundtrip_windows_paths",
       "davinci-session\\tests\\store.rs:7", nil, nil},
      {"0.58", "changelog: atomic write via tempfile + rename", "CHANGELOG.md:212",
       nil, nil}
    ]
  end

  @doc "2c — Mensura budget by role."
  def budget do
    [
      {"system", "4.2k", 0.08, "pinned", :ok},
      {"codex map", "22.8k", 0.24, "cap 40k", :ok},
      {"transcript", "71.6k", 0.72, "! soft cap 60k", :breach},
      {"instrumenta", "21.4k", 0.20, "14 schemas", :ok},
      {"memoria", "8.4k", 0.12, "3 chunks", :ok},
      {"reserve", "10.0k", 0.12, "for the reply", :ok}
    ]
  end

  @doc """
  3a — the full model catalog. `credential` is what decides whether a row is
  usable; rows with none stay listed, dimmed, so the catalog reads the same
  every time.
  """
  def catalog do
    [
      %{name: "anthropic / claude-sonnet", window: "200k", thinking: "budget",
        price: "3.00 · 15.00", credential: :ready, note: "oauth", ring: true},
      %{name: "anthropic / claude-opus", window: "200k", thinking: "budget",
        price: "15.00 · 75.00", credential: :ready, note: "oauth", ring: true},
      %{name: "anthropic / claude-haiku", window: "200k", thinking: "budget",
        price: "0.80 · 4.00", credential: :ready, note: "oauth", ring: false},
      %{name: "openai / gpt", window: "128k", thinking: "effort",
        price: "2.50 · 10.00", credential: :ready, note: "api key", ring: true},
      %{name: "openai-codex / gpt-codex", window: "272k", thinking: "effort",
        price: "plan", credential: :ready, note: "oauth", ring: false},
      %{name: "google / gemini", window: "1m", thinking: "budget",
        price: "1.25 · 10.00", credential: :ready, note: "api key", ring: false},
      %{name: "groq / llama", window: "131k", thinking: "none",
        price: "0.59 · 0.79", credential: :ready, note: "api key", ring: false},
      %{name: "github-copilot / gpt", window: "128k", thinking: "effort",
        price: "seat", credential: :expired, note: "expired", ring: false},
      %{name: "xai / grok", window: "256k", thinking: "effort",
        price: "3.00 · 15.00", credential: :absent, note: "none", ring: false},
      %{name: "deepseek / chat", window: "64k", thinking: "budget",
        price: "0.28 · 0.42", credential: :absent, note: "none", ring: false},
      %{name: "zai / glm", window: "200k", thinking: "budget",
        price: "0.60 · 2.20", credential: :absent, note: "none", ring: false},
      %{name: "llama.cpp / qwen-coder", window: "32k", thinking: "none",
        price: "local", credential: :ready, note: "running", ring: false}
    ]
  end

  @doc "3b — settings, each with the ramp of values it accepts and its scope."
  def settings do
    [
      %{label: "Auto-compact threshold", value: "default", scope: :user,
        values: ~w(default 90% 75% 50% 25%),
        description:
          "When auto-compaction triggers: a context percentage or an absolute " <>
            "token count. default is 92% of the model window."},
      %{label: "Auto-compact", value: "on", scope: :user, values: ~w(on off),
        description: "Compact the context automatically before it overflows."},
      %{label: "Steering mode", value: "one-at-a-time", scope: :user,
        values: ["one-at-a-time", "all"],
        description:
          "Enter while streaming queues a steering message. one-at-a-time " <>
            "delivers one and waits for the reply."},
      %{label: "Follow-up mode", value: "one-at-a-time", scope: :user,
        values: ["one-at-a-time", "all"],
        description: "Queue follow-up messages until the agent stops."},
      %{label: "Transport", value: "websocket-cached", scope: :project,
        values: ["sse", "websocket", "websocket-cached", "auto"],
        description:
          "Preferred transport for providers that support more than one. Set by " <>
            "this project, overriding your user setting."},
      %{label: "HTTP idle timeout", value: "2 min", scope: :user,
        values: ["30 sec", "1 min", "2 min", "5 min", "disabled"],
        description:
          "Longest idle gap while waiting for headers or body chunks. Disable it " <>
            "for local models that pause longer than five minutes."},
      %{label: "Mermaid diagrams", value: "final", scope: :user,
        values: ~w(off final streaming),
        description: "Render mermaid code blocks as unicode diagrams."},
      %{label: "Hide thinking", value: "off", scope: :user, values: ~w(on off),
        description: "Hide thinking blocks in assistant replies."},
      %{label: "Cache miss notices", value: "on", scope: :user, values: ~w(on off),
        description:
          "Show a transcript notice for a significant prompt-cache miss and for " <>
            "what a compaction cost."},
      %{label: "Autocomplete max items", value: "7", scope: :user,
        values: ~w(3 5 7 10 15 20),
        description: "How many rows the composer's completion list may show."},
      %{label: "Skill commands", value: "on", scope: :user, values: ~w(on off),
        description: "Register every discovered skill as a /skill:name command."},
      %{label: "Quiet startup", value: "on", scope: :user, values: ~w(on off),
        description: "Skip the verbose banner when a session opens."}
    ]
  end

  @doc """
  3c — thinking levels. `fraction` is of the 64k ceiling, not of the window;
  `warn` marks a level that takes a third of the window before a turn starts.
  """
  def thinking_levels do
    [
      %{level: "off", budget: "0", fraction: 0.0, maps_to: "disabled → none", warn: false},
      %{level: "minimal", budget: "1.0k", fraction: 0.016, maps_to: "1024 → minimal", warn: false},
      %{level: "low", budget: "4.0k", fraction: 0.063, maps_to: "4096 → low", warn: false},
      %{level: "medium", budget: "8.0k", fraction: 0.125, maps_to: "8192 → medium", warn: false},
      %{level: "high", budget: "16.0k", fraction: 0.25, maps_to: "16384 → high", warn: false},
      %{level: "xhigh", budget: "32.0k", fraction: 0.5, maps_to: "32768 → high", warn: false},
      %{level: "max", budget: "64.0k", fraction: 1.0, maps_to: "! 32% of the window", warn: true}
    ]
  end

  @doc "3d — provider credentials and where each one came from."
  def providers do
    [
      %{name: "anthropic", method: "oauth", source: "device flow, in progress", state: :pending},
      %{name: "openai", method: "api key", source: "env OPENAI_API_KEY", state: :ready},
      %{name: "openai-codex", method: "oauth", source: "auth.json · refreshes in 22h", state: :ready},
      %{name: "google", method: "api key", source: "auth.json", state: :ready},
      %{name: "github-copilot", method: "oauth", source: "refresh rejected 401 · 2d ago", state: :expired},
      %{name: "groq", method: "api key", source: "env GROQ_API_KEY, unset", state: :absent},
      %{name: "xai · deepseek · zai", method: "api key", source: "never configured", state: :absent},
      %{name: "llama.cpp", method: "local", source: "router at 127.0.0.1:8080", state: :local}
    ]
  end

  @doc "3d — the device-code grant in flight."
  def device_code do
    %{code: "WQPT-FJ4M", url: "https://claude.ai/oauth/device", expires: "8m 41s", polls: 6}
  end

  @doc "3e — the keymap, grouped by the surface a key belongs to."
  def keymap do
    [
      {"INSTRUMENTS", "over the transcript",
       [
         {"ctrl+p", "instrumenta · palette"},
         {"ctrl+s", "memoria · sessions"},
         {"ctrl+r", "memoria · vector recall"},
         {"ctrl+o", "cogitator · model"},
         {"ctrl+l", "disegno · plan"},
         {"ctrl+g", "grafo · graph"},
         {"ctrl+u", "mensura · governor"},
         {"ctrl+e", "codex · workspace"},
         {"esc", "close whichever is open"}
       ]},
      {"RUN", "while the agent works",
       [
         {"ctrl+c", "interrupt the run · never the app"},
         {"ctrl+d", "quit"},
         {"ctrl+z", "suspend to the shell"},
         {"shift+tab", "cycle thinking level"},
         {"ctrl+t", "thinking on / off"}
       ]},
      {"COMPOSER", "",
       [
         {"enter", "send"},
         {"shift+enter", "newline · also ctrl+j"},
         {"alt+enter", "queue as follow-up"},
         {"alt+up", "take the last queued back"},
         {"tab", "complete command, file, skill"},
         {"ctrl+v", "paste image from clipboard"},
         {"ctrl+g", "open $EDITOR on the draft"},
         {"ctrl+x", "copy last agent message"}
       ]},
      {"SESSION LIST", "inside memoria",
       [
         {"ctrl+p", "show full paths"},
         {"ctrl+s", "sort recent / name"},
         {"ctrl+r", "rename"},
         {"ctrl+n", "named sessions only"},
         {"ctrl+d", "delete · confirms first"}
       ]},
      {"SESSION TREE", "",
       [
         {"ctrl+← ctrl+→", "fold / unfold a branch"},
         {"shift+l", "label this turn"},
         {"ctrl+d t u l a", "filter: default, no tools, user, labeled, all"}
       ]}
    ]
  end

  @doc "4a — the session list, with what resuming one would carry."
  def session_count, do: 34

  def resume_sessions do
    [
      %{name: "review-agent-runtime", branch: "main", turns: "42", tokens: "128k",
        model: "sonnet", touched: "3m", named: true, warning: nil,
        note: "forked from provider-parity at turn 12 · Δ7 files · 3 branches",
        last: "now fix the store.rs type error",
        path: "~\\.davinci\\agent\\sessions\\--dev--oss--davinci-rust\\01JB2K….jsonl"},
      %{name: "implement-rpc-mode", branch: "main", turns: "61", tokens: "184k",
        model: "sonnet", touched: "18m", named: true, warning: nil,
        note: "compacted twice · 2 forks", last: "add the rpc handshake test",
        path: "~\\.davinci\\agent\\sessions\\--dev--oss--davinci-rust\\01JAX7….jsonl"},
      %{name: "provider-parity", branch: "main", turns: "28", tokens: "96k",
        model: "opus", touched: "1h", named: true, warning: nil,
        note: "parent of review-agent-runtime", last: "compare the streaming shapes",
        path: "~\\.davinci\\agent\\sessions\\--dev--oss--davinci-rust\\01JAW1….jsonl"},
      %{name: "tui-redesign", branch: "davinci", turns: "117", tokens: "412k",
        model: "sonnet", touched: "yest.", named: true, warning: nil,
        note: "the longest session in this project", last: "draw 1h in NO_COLOR",
        path: "~\\.davinci\\agent\\sessions\\--dev--oss--davinci-rust\\01J9Q4….jsonl"},
      %{name: "01J8ZK…7QW4", branch: "main", turns: "4", tokens: "11k",
        model: "haiku", touched: "2d", named: false, warning: nil,
        note: "never named · four turns", last: "what does pi-parity do",
        path: "~\\.davinci\\agent\\sessions\\--dev--oss--davinci-rust\\01J8ZK….jsonl"},
      %{name: "fix-git-hooks", branch: "hooks", turns: "33", tokens: "88k",
        model: "gpt", touched: "2d", named: true,
        warning: "! branch hooks no longer exists · resuming replays against main",
        note: "! branch hooks no longer exists · resuming replays against main",
        last: "the pre-commit hook eats the exit code",
        path: "~\\.davinci\\agent\\sessions\\--dev--oss--davinci-rust\\01J8PD….jsonl"}
    ]
  end

  @doc """
  4b — the session tree. Rows with no `id` are spacers that carry only the
  trunk, so the verticals stay continuous without any row knowing its
  neighbours.
  """
  def session_tree do
    [
      %{trunk: "", state: :queued, id: "01", label: "explain how the agent runtime works",
        meta: "12:04", text_color: nil},
      %{trunk: "│", state: nil, id: nil, label: nil, meta: nil, text_color: nil},
      %{trunk: "├── ", state: :done, id: "02", label: "surveyed the workspace",
        meta: "12:05", text_color: nil},
      %{trunk: "│   │", state: nil, id: nil, label: nil, meta: nil, text_color: nil},
      %{trunk: "│   └── ", state: :failed, id: "03", label: "store as a trait · abandoned",
        meta: "12:09", text_color: nil},
      %{trunk: "│", state: nil, id: nil, label: nil, meta: nil, text_color: nil},
      %{trunk: "└── ", state: :done, id: "04", label: "traced the request pipeline",
        meta: "12:11", text_color: nil},
      %{trunk: "    │", state: nil, id: nil, label: nil, meta: nil, text_color: nil},
      %{trunk: "    ├── ", state: :active, id: "05", label: "fix the store.rs type error",
        meta: "12:18", text_color: nil},
      %{trunk: "    │", state: nil, id: nil, label: nil, meta: nil, text_color: nil},
      %{trunk: "    └── ", state: :queued, id: "06", label: "fork · streaming rewrite",
        meta: "12:22", text_color: nil}
    ]
  end

  @doc "4c — what a compaction would do, before it does it."
  def compaction do
    %{
      before_tokens: "184.2k",
      before_fraction: 0.92,
      before_note: "! 92% of 200k",
      after_tokens: "61.8k",
      after_fraction: 0.31,
      after_note: "31% of 200k",
      kept: [
        "the last 6 turns, whole",
        "every Δ and its hunks · 7 files",
        "the disegno plan, steps I–V",
        "your instruction: store.rs decisions",
        "AGENTS.md and CLAUDE.md · re-read, not summarised"
      ],
      folded: [
        "turns 1–18 · 96.4k",
        "31 tool results · kept as ids, retrievable",
        "9 superseded reads of the same file",
        "2 memoria injections, now stale"
      ],
      recovers: "122.4k",
      call_cost: "$0.19",
      cache_cost: "$0.23"
    }
  end

  @doc "4d — what an export carries out of the session."
  def export_ledger do
    %{
      included: [
        "42 turns of prose and thinking",
        "31 tool calls with their output",
        "every Δ hunk, syntax coloured",
        "4 pasted images, inlined as base64"
      ],
      excluded: [
        {:failed, "api keys and bearer tokens · redacted"},
        {:failed, "the contents of .env · 2 reads masked"},
        {:attention, "absolute paths · kept, they name your machine"},
        {:attention, "branch names and commit subjects · kept"}
      ],
      size: "2.9 MB",
      elapsed: "1.4s",
      gist: "https://gist.github.com/9f21c4…a70"
    }
  end

  @doc "5a — a task running as a graph of isolated workers."
  def graph_run do
    %{
      goal: "add prompt-cache parity to the openai adapter --complex",
      phases: [
        {"classify", :done},
        {"investigate", :done},
        {"plan", :done},
        {"implement", :active},
        {"verify", :queued},
        {"review", :queued},
        {"done", :queued}
      ],
      shape: [
        "t1 classifier ─┬─ t2 researcher    ─┐",
        "               ├─ t3 test-analyzer ─┼─ t5 planner",
        "               └─ t4 historian     ─┘      │",
        "                                           └─ t6 writer ◉ ─ t7 reviewer"
      ],
      tasks: [
        %{id: "t1 classifier", policy: "read-only", artifact: "feature · complex",
          usage: "2.1k↑ 0.4k↓ $0.01 4s", state: :done},
        %{id: "t2 researcher", policy: "read-only", artifact: "evidence · 14 call sites",
          usage: "31k↑ 3.2k↓ $0.14 48s", state: :done},
        %{id: "t3 test-analyzer", policy: "read-and-test", artifact: "baseline · 212 pass",
          usage: "18k↑ 2.0k↓ $0.09 1m52s", state: :done},
        %{id: "t4 historian", policy: "read-only", artifact: "evidence · 3 attempts",
          usage: "9.4k↑ 1.1k↓ $0.05 22s", state: :done},
        %{id: "t5 planner", policy: "read-only", artifact: "plan · 4 milestones",
          usage: "42k↑ 5.8k↓ $0.31 1m09s", state: :done},
        %{id: "t6 writer", policy: "write-no-git-mutation", artifact: "davinci-ai\\src\\openai.rs",
          usage: "64k↑ 9.7k↓ $0.71 2m14s", state: :active},
        %{id: "t7 reviewer", policy: "read-and-test", artifact: "pending · waits on t6",
          usage: "—", state: :queued}
      ],
      cost: "$1.31",
      cost_cap: "$8.00",
      cost_fraction: 0.16,
      workers: "6 of 12",
      parallel: "3",
      cycles: "0 of 2",
      replans: "0 of 1",
      artifacts: ".davinci\\graph\\g-7f2a\\"
    }
  end

  @doc "5b — the vector index itself."
  def vector_index do
    %{
      repo: "davinci-rust",
      repo_records: "6,914",
      total_records: "18,402",
      injection_cap: "1.5k tokens",
      floor: "0.70",
      kinds: [
        {"decision", "1,482", 0.48, "importance 0.9"},
        {"architecture", "906", 0.30, "importance 0.9"},
        {"discovery", "1,105", 0.36, "importance 0.7"},
        {"bug · fix", "842", 0.27, "importance 0.8"},
        {"constraint", "311", 0.10, "never evicted"},
        {"task result", "1,674", 0.54, "importance 0.6"},
        {"compaction", "64", 0.02, "one per compaction"},
        {"conversation", "530", 0.17, "first to go"}
      ],
      embeddings: "ollama",
      embed_host: "127.0.0.1:11434 · bge-small 384d",
      store: "qdrant",
      collection: "collection davinci-memoria · 3 shards",
      extraction: "haiku",
      config: "%USERPROFILE%\\.davinci\\vector-memory.json",
      health: [
        {:done, "reindexed on the last commit · 4m ago"},
        {:done, "embed 11ms p50 · 34ms p95"},
        {:attention, "7 records failed to embed · retried next reindex"},
        {:done, "no duplicate content hashes"}
      ]
    }
  end

  @doc "5c — what the governor did to this session's tool output."
  def governor do
    %{
      counters: [
        {"31", "of 96 results", "compressed", "head 40 · tail 40 · rest on disk", :primary},
        {"9", "of 61 reads", "deduplicated", "same file, same state hash", :secondary},
        {"4", "of 96 calls", "blocked", "anti-loop · no new state", :warning},
        {"96.2k", "of 200k", "tokens never sent", "about $0.29 at sonnet input", :success}
      ],
      stored: [
        %{id: "out-9f21c4", tool: "bash", call: "cargo test --workspace",
          size: "1,184 ln · 84 KB", stale: false},
        %{id: "out-4c07ab", tool: "grep", call: "\"SessionManager\" across crates",
          size: "612 ln · 31 KB", stale: false},
        %{id: "out-1e88d0", tool: "read", call: "davinci-ai\\src\\openai.rs",
          size: "2,041 ln · 96 KB", stale: false},
        %{id: "out-77b3e5", tool: "powershell", call: "git log --stat -n 40",
          size: "498 ln · 22 KB", stale: true}
      ],
      store_dir: "%USERPROFILE%\\.davinci\\outputs\\01JB2K\\ · dropped when the session ends"
    }
  end

  @doc "5d — a security scan mid-validation."
  def security_scan do
    %{
      validated: 31,
      candidates: 44,
      fraction: 0.7,
      files: "1,842",
      skipped: "96",
      bytes: "41.2 MB",
      severities: [
        {"critical", 1, :critical},
        {"high", 3, :high},
        {"medium", 6, :medium},
        {"low", 9, :low},
        {"informational", 14, :dismissed}
      ],
      dismissed: 11,
      findings: [
        %{message: "bearer token written to the transcript",
          location: "davinci-ai\\src\\auth.rs:214", severity: :critical, rule: "secret-in-log",
          evidence: "tracing::debug!(\"refresh {token}\")",
          path: "refresh_token() → subscriber → session jsonl → /export, /share"},
        %{message: "command built from an unquoted path",
          location: "davinci-agent\\src\\tools\\bash.rs:88", severity: :high, rule: "shell-injection",
          evidence: "format!(\"cd {} && {}\", dir, cmd)",
          path: "bash tool → cmd.exe → any path with a space or an ampersand"},
        %{message: "extension host inherits your environment",
          location: "davinci-cli\\src\\js_host.rs:141", severity: :high, rule: "env-leak",
          evidence: "Command::new(node).envs(env::vars())",
          path: "project extension → node subprocess → every API key you hold"},
        %{message: "session files written 0644 on unix",
          location: "davinci-session\\src\\store.rs:118", severity: :high, rule: "file-mode",
          evidence: "OpenOptions::new().create(true)",
          path: "session jsonl → any local account on a shared machine"},
        %{message: "http client accepts any tls version",
          location: "davinci-ai\\src\\http.rs:57", severity: :medium, rule: "weak-tls",
          evidence: "danger_accept_invalid_certs(false) only",
          path: "provider request → downgraded transport on a hostile network"},
        %{message: "hard-coded test key in a fixture",
          location: "tests\\fixtures\\auth.json:3", severity: :dismissed, rule: "secret-literal",
          evidence: "\"api_key\": \"sk-test-0000\"",
          path: "not a real credential · never read outside the test"}
      ],
      seal: "4b1f…c9e0",
      report: ".davinci\\security\\s-31c8\\report.json · 214 KB"
    }
  end

  @doc "6a — what a project would load, before it is trusted."
  def project_trust do
    %{
      files: [
        %{state: :attention, path: ".davinci\\extensions\\lint.js",
          detail: "runs as node, no sandbox", risk: :executes, risk_label: "executes code"},
        %{state: :attention, path: ".davinci\\extensions\\deploy.js",
          detail: "registers 3 tools, 1 pre-tool hook", risk: :executes, risk_label: "executes code"},
        %{state: :attention, path: ".davinci\\settings.json",
          detail: "3 keys, incl. transport and tool allowlist", risk: :limits,
          risk_label: "changes limits"},
        %{state: :read, path: ".davinci\\skills\\ (6)",
          detail: "instructions loaded on demand", risk: :prompt, risk_label: "prompt text"},
        %{state: :read, path: ".davinci\\prompts\\ (3)",
          detail: "slash commands that expand to prompts", risk: :prompt, risk_label: "prompt text"},
        %{state: :read, path: "AGENTS.md · CLAUDE.md",
          detail: "1,208 lines, prepended to every turn", risk: :prompt, risk_label: "prompt text"},
        %{state: :queued, path: ".davinci\\themes\\ (1)",
          detail: "colours only", risk: :harmless, risk_label: "harmless"}
      ],
      path: "C:\\dev\\clones\\vendor-cli",
      trusted: "14 projects",
      ignored: "2",
      store: "%USERPROFILE%\\.davinci\\trust.json"
    }
  end

  @doc "6b — what /reload loaded, and what it cost."
  def workshop do
    %{
      reload: [
        {:done, "keybindings · 39 bindings, 2 yours", "3ms", nil},
        {:done, "skills · 6 found, none loaded until named", "11ms", nil},
        {:done, "context files · AGENTS.md, CLAUDE.md · 4.1k", "6ms", nil},
        {:failed, "extensions · deploy.js failed to register", "318ms",
         "TypeError: hooks.preTool is not a function · deploy.js:41 · its 3 tools are missing"}
      ],
      native: [
        {:done, "vector-memory", "4 tools · 4 commands"},
        {:done, "token-governor", "2 tools · 2 commands"},
        {:done, "graph", "1 tool · 5 commands"},
        {:done, "security-scan", "7 tools · 3 commands"}
      ],
      javascript: [
        {:done, "plan-mode", "1 tool · registers --plan"},
        {:done, "lint.js · project", "1 post-tool hook"},
        {:failed, "deploy.js · project", "failed to register"}
      ],
      node: "node v24.19.0",
      schema: "21.4k · 11%",
      tools: [
        {"built-in tools", "8", 0.33, "read write edit grep find ls bash pwsh"},
        {"native tools", "14", 0.58, "memory, governor, graph, sec"},
        {"extension tools", "2", 0.08, "3 more if deploy.js is fixed"}
      ]
    }
  end

  @doc "6c — the turn that did not complete, and the interrupt after it."
  def failed_run do
    %{
      prompt: "rewrite the provider adapter to stream",
      tools: [
        {:read, "read davinci-ai\\src\\openai.rs", "2,041 lines"},
        {:done, "cargo check -p davinci-ai", "1.84s · manus"},
        {:failed, "stream · 429 rate limited after 1,204 tokens", "0.9s"}
      ],
      kept: "1,204 tokens",
      billed: "$0.04",
      aftermath: [
        {:done, "transcript written to the session file · nothing to recover on restart"},
        {:done, "the running cargo check was killed with its process group"},
        {:attention, "edit to openai.rs had not started — the file on disk is untouched"},
        {:skipped, "a second ctrl+c within a second clears the composer; ctrl+d quits"}
      ]
    }
  end

  @doc """
  6d — every file the turn changed. Each file carries its own hunk: a review
  screen that showed one file's diff under another file's name would be worse
  than showing none.
  """
  def review do
    %{
      files: [
        %{state: :delta, path: "crates\\davinci-ai\\src\\openai.rs", adds: 64, dels: 19,
          tests: "✓ 14 tests pass", test_state: :pass, hunk_note: "hunk 2 of 5",
          hunk: [
            {:context, "pub async fn complete(&self, req: Request) -> Result<Reply> {"},
            {:del, "    let body = self.post(req).await?;"},
            {:del, "    Ok(Reply::from(body))"},
            {:add, "    let mut stream = self.post_stream(req).await?;"},
            {:add, "    let mut reply = Reply::default();"},
            {:add, "    while let Some(chunk) = stream.next().await {"},
            {:add, "        reply.push(chunk?);"},
            {:add, "    }"},
            {:add, "    Ok(reply)"},
            {:context, "}"}
          ]},
        %{state: :delta, path: "crates\\davinci-ai\\src\\stream.rs", adds: 38, dels: 11,
          tests: "✓ 6 tests pass", test_state: :pass, hunk_note: "hunk 1 of 3",
          hunk: [
            {:context, "fn parse_event(line: &str) -> Option<Chunk> {"},
            {:del, "    serde_json::from_str(line).ok()"},
            {:add, "    let payload = line.strip_prefix(\"data: \")?;"},
            {:add, "    if payload == \"[DONE]\" { return None; }"},
            {:add, "    serde_json::from_str(payload).ok()"},
            {:context, "}"}
          ]},
        %{state: :delta, path: "crates\\davinci-agent\\src\\runtime.rs", adds: 21, dels: 6,
          tests: "! untested path", test_state: :untested, hunk_note: "hunk 1 of 2",
          hunk: [
            {:context, "match provider.complete(req).await {"},
            {:del, "    Ok(reply) => self.record(reply),"},
            {:add, "    Ok(reply) => {"},
            {:add, "        self.record(reply.clone());"},
            {:add, "        self.session.write(&reply)?;"},
            {:add, "    }"},
            {:context, "    Err(err) => self.fail(err),"}
          ]},
        %{state: :done, path: "crates\\davinci-ai\\tests\\stream.rs · new", adds: 17, dels: nil,
          tests: "✓ 4 tests pass", test_state: :pass, hunk_note: "the whole file",
          hunk: [
            {:add, "#[test]"},
            {:add, "fn done_sentinel_ends_the_stream() {"},
            {:add, "    let chunks = collect(\"data: [DONE]\\n\");"},
            {:add, "    assert!(chunks.is_empty());"},
            {:add, "}"}
          ]},
        %{state: :delta, path: "Cargo.toml", adds: 2, dels: 2,
          tests: "pinned", test_state: :none, hunk_note: "hunk 1 of 1",
          hunk: [
            {:del, "eventsource-stream = \"0.2\""},
            {:del, "futures = \"0.3\""},
            {:add, "eventsource-stream = \"=0.2.3\""},
            {:add, "futures = \"=0.3.31\""}
          ]},
        %{state: :failed, path: "crates\\davinci-ai\\src\\legacy.rs · deleted", adds: nil, dels: 88,
          tests: "! 2 references left", test_state: :untested, hunk_note: "88 lines removed",
          hunk: [
            {:del, "pub struct LegacyProvider {"},
            {:del, "    client: Client,"},
            {:del, "}"},
            {:context, "… 85 more removed lines"}
          ]},
        %{state: :delta, path: "CHANGELOG.md", adds: 3, dels: 1,
          tests: "one entry", test_state: :none, hunk_note: "hunk 1 of 1",
          hunk: [
            {:context, "## Unreleased"},
            {:del, "- nothing yet"},
            {:add, "### Added"},
            {:add, "- streaming for the openai adapter"},
            {:add, "- prompt-cache parity with the TS client"}
          ]}
      ],
      adds: 145,
      dels: 127,
      branch: "main",
      behind: "3 commits behind",
      warning: "legacy.rs is gone but 2 files still name it · the build will fail",
      tests: "212 of 212 tests pass on the changed crates · 41.2s"
    }
  end
end
