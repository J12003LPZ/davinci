defmodule Davinci.Views.GraphRun do
  @moduledoc """
  5a — `/graph`. A task run as a graph of isolated workers, which is a different
  instrument from 2a: 2a studies the code's dependencies, this watches a run.

  Every worker is a child process with its own tool allowlist and its own shell
  policy, and the screen says which policy each one got — that is the whole
  safety argument, so it is on the row rather than in a manual. The phase rail
  and the budgets are meters with their caps (design.md §9).
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @id 16
  @policy 21

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    run = Model.graph_run()

    echo =
      line([
        t(Theme.glyph(:user) <> " ", th.primary),
        t("/graph " <> run.goal, th.muted)
      ])

    rail =
      line(
        run.phases
        |> Enum.flat_map(fn {name, state} ->
          [
            t(Theme.glyph(state) <> " ", Theme.state_color(th, state), nil, th.emphasis),
            t(name, if(state == :active, do: th.text, else: th.muted)),
            t("  ", th.border)
          ]
        end)
      )

    graph =
      box(
        width: w,
        theme: th,
        title: [t("GRAFO", th.primary), t(" · ", th.border), t("WORKER GRAPH", th.muted)],
        right: [
          t("#{length(run.tasks)} tasks", th.muted),
          t(" · ", th.border),
          t("0 blocked", th.success)
        ],
        body: Enum.map(run.shape, fn row -> [t(row, th.border)] end)
      )

    header =
      line([
        sp(2),
        t(String.pad_trailing("WORKER", @id + 1), th.border),
        t(String.pad_trailing("SHELL POLICY", @policy + 1), th.border),
        t("ARTIFACT", th.border)
      ])

    # Below 88 the usage column goes: what a worker is doing outranks what it
    # has spent, and the run total is in the budgets below either way.
    usage? = model.width >= 88
    tasks = Enum.map(run.tasks, &task(&1, model, usage?, th))

    budgets = [
      line([
        t("cost ", th.muted),
        t(run.cost, th.text),
        t(" of ", th.muted),
        t(run.cost_cap, th.text),
        t("  ", th.border)
      ] ++ meter(run.cost_fraction, 16, th, th.success)),
      line([
        t("workers ", th.muted),
        t(run.workers, th.text),
        t(" · at most ", th.muted),
        t(run.parallel, th.text),
        t(" at a time", th.muted),
        t(" · revision cycles ", th.muted),
        t(run.cycles, th.text),
        t(" · replans ", th.muted),
        t(run.replans, th.text)
      ]),
      line([
        t("no run deadline · per-role timeouts unlimited", th.border),
        t(" · ", th.border),
        t(run.artifacts, th.border)
      ])
    ]

    footer = [
      line([
        t("enter open artifact", th.border),
        t(" · ", th.border),
        t("v tail a worker", th.border),
        t(" · ", th.border),
        t("r resume a stopped run", th.border)
      ]),
      line([
        t("a abort", th.border),
        t(" · ", th.border),
        t("esc close", th.border)
      ])
    ]

    [echo, blank(), rail, blank()] ++
      graph ++ [blank(), header] ++ tasks ++ [blank()] ++ budgets ++ [blank()] ++ footer
  end

  defp task(task, model, usage?, th) do
    glyph =
      if task.state == :active,
        do: Theme.spinner(model.tick, model.animate),
        else: Theme.glyph(task.state)

    color = Theme.state_color(th, task.state)
    text_color = if task.state == :active, do: th.text, else: th.muted
    detail = if task.state == :queued, do: th.border, else: th.muted

    line(
      [
        t(glyph <> " ", color, nil, th.emphasis),
        t(String.pad_trailing(task.id, @id + 1), text_color),
        t(String.pad_trailing(task.policy, @policy + 1), detail),
        t(String.pad_trailing(clip(task.artifact, 28), 29), detail)
      ] ++ if(usage?, do: [t(task.usage, th.border)], else: [])
    )
  end
end
