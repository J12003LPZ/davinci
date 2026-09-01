defmodule Davinci.Views.Resume do
  @moduledoc """
  4a — `/resume`. The Memoria session list at full width: what each session was,
  where it was, and what resuming it would cost you.

  The selected row expands one line rather than opening a second panel (design
  .md §1), and a session whose branch no longer exists says so on the row — the
  thing you want to know before you resume it, not after.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @name 24
  @branch 9
  @turns 5
  @tokens 7
  @model 7

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    list = Model.resume_sessions()
    selected = rem(model.resume_index, length(list))
    current = Enum.at(list, selected)

    query =
      box(
        width: w,
        theme: th,
        border: th.secondary,
        right: [
          t("#{length(list)} of #{Model.session_count()}", th.border),
          t(" · ", th.border),
          t("sort recent", th.muted)
        ],
        body: [
          [
            t(Theme.glyph(:search) <> " ", th.secondary),
            t("filter sessions…", th.muted),
            caret(model, th)
          ]
        ]
      )

    header =
      line([
        sp(2),
        t(String.pad_trailing("SESSION", @name), th.border),
        t(String.pad_trailing("BRANCH", @branch + 1), th.border),
        t(String.pad_leading("TURNS", @turns) <> " ", th.border),
        t(String.pad_leading("TOKENS", @tokens) <> " ", th.border),
        t(String.pad_trailing("MODEL", @model + 1), th.border),
        t("TOUCHED", th.border)
      ])

    rows =
      list
      |> Enum.with_index()
      |> Enum.flat_map(fn {session, index} ->
        row = row(session, index == selected, th)

        cond do
          index == selected -> [row, note(session.note, th.border, th)]
          session.warning -> [row, note(session.warning, th.warning, th)]
          true -> [row]
        end
      end)

    footer =
      [
        line([
          t("selected ", th.muted),
          t(current.name, th.text),
          t(" · last message ", th.muted),
          t("“" <> clip(current.last, 38) <> "”", th.border)
        ]),
        line([t(current.path, th.border)])
      ] ++
        Enum.map(
          wrap(
            "resuming replays the transcript, not the tools — nothing runs until " <>
              "you send the next turn",
            measure()
          ),
          fn row -> line([t(row, th.muted)]) end
        ) ++
        [
          line([
            t("enter resume", th.border),
            t(" · ", th.border),
            t("f fork", th.border),
            t(" · ", th.border),
            t("ctrl+r rename", th.border),
            t(" · ", th.border),
            t("ctrl+s sort", th.border),
            t(" · ", th.border),
            t("esc close", th.border)
          ])
        ]

    query ++ [blank(), header] ++ rows ++ [blank()] ++ footer
  end

  defp row(session, selected?, th) do
    bg = if selected?, do: th.surface, else: nil

    state =
      cond do
        selected? -> :active
        session.warning -> :attention
        true -> :queued
      end

    name_color =
      cond do
        selected? -> th.text
        session.named -> th.muted
        true -> th.border
      end

    detail = if session.named, do: th.muted, else: th.border

    line([
      t(Theme.glyph(state) <> " ", Theme.state_color(th, state), bg, th.emphasis),
      t(String.pad_trailing(clip(session.name, @name - 2), @name - 2), name_color, bg),
      t(String.pad_trailing(session.branch, @branch + 1), th.secondary, bg),
      t(String.pad_leading(session.turns, @turns) <> " ", detail, bg),
      t(String.pad_leading(session.tokens, @tokens) <> " ", detail, bg),
      t(String.pad_trailing(session.model, @model + 1), detail, bg),
      t(session.touched, detail, bg)
    ])
  end

  defp note(nil, _color, _th), do: blank()
  defp note(text, color, th), do: indent(2, [t(clip(text, measure()), color || th.border)])

  defp caret(model, th) do
    if Model.blink?(model),
      do: t(" ", th.background, th.secondary),
      else: t(" ", th.background, th.background)
  end
end
