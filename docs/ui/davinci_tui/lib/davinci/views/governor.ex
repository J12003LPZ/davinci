defmodule Davinci.Views.Governor do
  @moduledoc """
  5c — `governor-status`. What the governor did to your tool output, which is a
  different question from 2c's "where is the budget going".

  Counters carry their denominator, and the screen shows one compressed result
  in full so the elision marker and the retrieval id are visible rather than
  described: nothing is deleted, the tail is on disk, and the model can ask for
  any range of it back.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @id 12
  @tool 12

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    status = Model.governor()

    counters =
      Enum.map(status.counters, fn {value, cap, label, note, color} ->
        line([
          t(String.pad_leading(value, 6), color_for(color, th)),
          t(" " <> String.pad_trailing(cap, 14), th.muted),
          t(String.pad_trailing(label, 22), th.text),
          t(note, th.border)
        ])
      end)

    sample =
      box(
        width: w,
        theme: th,
        title: [t("WHAT A COMPRESSED RESULT LOOKS LIKE", th.warning)],
        body: [
          [
            t("⎿ ", th.border),
            t(Theme.glyph(:done) <> " ", th.success, nil, th.emphasis),
            t("cargo test --workspace", th.muted),
            t("  41.2s · manus", th.border)
          ],
          [t("  running 212 tests", th.border)],
          [t("  test session::store::roundtrip ", th.border), t("... ok", th.success)],
          [
            t("  … 1,184 lines held on disk · ", th.border),
            t("out-9f21c4", th.warning),
            t(" · 84 KB", th.border)
          ],
          [t("  test result: ", th.border), t("ok", th.success), t(". 212 passed; 0 failed", th.border)],
          [],
          [t("  retrieve_output out-9f21c4 --lines 600-640", th.text)],
          [
            t("⎿ ", th.border),
            t(Theme.glyph(:attention) <> " ", th.warning, nil, th.emphasis),
            t("the model asked for the middle", th.muted)
          ]
        ]
      )

    header =
      line([
        t(String.pad_trailing("HELD ON DISK", @id + 1), th.border),
        t(String.pad_trailing("TOOL", @tool + 1), th.border),
        t(String.pad_trailing("CALL", 30), th.border),
        t("SIZE", th.border)
      ])

    stored =
      Enum.map(status.stored, fn entry ->
        color = if entry.stale, do: th.border, else: th.muted

        line([
          t(String.pad_trailing(entry.id, @id + 1), if(entry.stale, do: th.border, else: th.warning)),
          t(String.pad_trailing(entry.tool, @tool + 1), color),
          t(String.pad_trailing(clip(entry.call, 30), 31), color),
          t(entry.size, th.border)
        ])
      end)

    footer =
      Enum.map(
        wrap(
          "compresses above 8 KB or 300 lines, keeping 40 head, 40 tail and 20 " <>
            "lines it judges important. Nothing is deleted.",
          measure()
        ),
        fn row -> line([t(row, th.muted)]) end
      ) ++
        [
          line([t(status.store_dir, th.border)]),
          line([
            t("enter open an output", th.border),
            t(" · ", th.border),
            t("d dedupe on/off", th.border),
            t(" · ", th.border),
            t("l anti-loop on/off", th.border)
          ]),
          line([
            t("r reset counters", th.border),
            t(" · ", th.border),
            t("esc close", th.border)
          ])
        ]

    counters ++
      [blank()] ++ sample ++ [blank(), header] ++ stored ++ [blank()] ++ footer
  end

  defp color_for(:primary, th), do: th.primary
  defp color_for(:secondary, th), do: th.secondary
  defp color_for(:warning, th), do: th.warning
  defp color_for(_other, th), do: th.success
end
