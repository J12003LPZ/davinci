defmodule Davinci.Views.Login do
  @moduledoc """
  3d — `/login`. A device-code flow in progress over a ledger of every provider
  and where its credential came from: an environment variable, the auth file, a
  refresh token, or nothing.

  The waiting spinner is the one already on the 250ms clock (design.md §8), and
  the panel says plainly that ctrl+c cancels the login and not the session.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @provider 16
  @method 10
  @state 12

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    device = Model.device_code()

    echo =
      line([
        t(Theme.glyph(:user) <> " ", th.primary),
        t("/login anthropic", th.muted)
      ])

    flow =
      box(
        width: w,
        theme: th,
        border: th.primary,
        title: [
          t("DEVICE AUTHORISATION", th.primary),
          t(" · ", th.border),
          t("ANTHROPIC", th.muted)
        ],
        right: [t("expires in " <> device.expires, th.warning)],
        body: [
          [
            t("1 ", th.border),
            t("open ", th.muted),
            t(device.url, th.secondary)
          ],
          [
            t("2 ", th.border),
            t("enter ", th.muted),
            t(device.code, th.text, nil, th.emphasis)
          ],
          [
            t("3 ", th.border),
            t("davinci writes the refresh token and returns here", th.muted)
          ],
          []
        ] ++
          Enum.map(
            wrap(
              "the browser was not opened for you — nothing leaves this terminal " <>
                "until you approve it there",
              w - 6
            ),
            fn row -> [t(row, th.border)] end
          ) ++
          [
            [],
            [
              t(Theme.spinner(model.tick, model.animate) <> " ", th.primary, nil, th.emphasis),
              t("waiting for approval", th.text),
              t("  ·  ", th.border),
              t("polled #{device.polls}×", th.muted)
            ],
            [t("ctrl+c cancels the login, not the session", th.border)]
          ]
      )

    # At 80 columns the source column goes rather than the row wrapping
    # (design.md §7): the state is the answer, the source is the footnote.
    source? = model.width >= 88

    header =
      line(
        [
          sp(2),
          t(String.pad_trailing("PROVIDER", @provider - 1), th.border),
          t(String.pad_trailing("METHOD", @method + 1), th.border)
        ] ++
          if(source?, do: [t(String.pad_trailing("SOURCE", 30), th.border)], else: []) ++
          [t("STATE", th.border)]
      )

    list = Model.providers()
    selected = rem(model.login_index, length(list))

    rows =
      list
      |> Enum.with_index()
      |> Enum.map(fn {provider, index} -> row(provider, index == selected, source?, th) end)

    footer =
      Enum.map(
        wrap(
          "keys are never echoed, never written to the transcript, never sent to " <>
            "another provider",
          measure()
        ),
        fn row -> line([t(row, th.muted)]) end
      ) ++
        [
          line([t("davinci auth print-bearer-token --provider openai-codex", th.text)]),
          line([t("hands one to an external client", th.border)]),
          line([
            t("enter re-authenticate", th.border),
            t(" · ", th.border),
            t("k paste api key", th.border),
            t(" · ", th.border),
            t("d /logout provider", th.border)
          ]),
          line([
            t("r refresh now", th.border),
            t(" · ", th.border),
            t("esc close", th.border)
          ])
        ]

    [echo, blank()] ++ flow ++ [blank(), header] ++ rows ++ [blank()] ++ footer
  end

  defp row(provider, selected?, source?, th) do
    state = state_glyph(provider.state)
    color = Theme.state_color(th, state)
    dim? = provider.state == :absent
    bg = if selected?, do: th.surface, else: nil
    name_color = cond do
      selected? -> th.text
      dim? -> th.border
      true -> th.muted
    end

    line(
      [
        t(Theme.glyph(state) <> " ", color, bg, th.emphasis),
        t(String.pad_trailing(clip(provider.name, @provider - 2), @provider - 1), name_color, bg),
        t(String.pad_trailing(provider.method, @method + 1), if(dim?, do: th.border, else: th.muted), bg)
      ] ++
        if(source?,
          do: [t(String.pad_trailing(clip(provider.source, 29), 30), th.border, bg)],
          else: []
        ) ++
        [t(String.pad_trailing(label(provider.state), @state), color, bg)]
    )
  end

  defp state_glyph(:ready), do: :done
  defp state_glyph(:local), do: :done
  defp state_glyph(:pending), do: :active
  defp state_glyph(:expired), do: :attention
  defp state_glyph(_), do: :queued

  defp label(:ready), do: "ready"
  defp label(:pending), do: "pending"
  defp label(:expired), do: "expired"
  defp label(:local), do: "running"
  defp label(_), do: "absent"
end
