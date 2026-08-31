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

    %{
      model
      | composer: "",
        running: true,
        screen: :agent,
        overlay: nil,
        transcript: model.transcript ++ [:gap, {:user, turn} | reply(turn)]
    }
  end

  def interrupt(model), do: %{model | running: false}

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
end
