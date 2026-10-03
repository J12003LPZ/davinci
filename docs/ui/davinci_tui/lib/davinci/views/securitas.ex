defmodule Davinci.Views.Securitas do
  @moduledoc """
  5d — `sec-status` / `sec-report`. A scan you can audit.

  Every finding carries the rule that produced it, the file and line it was read
  out of, and the evidence, so a claim can be checked rather than trusted; the
  selected one expands to its attack path. Coverage and the report seal are
  stated on the same screen as the findings — a count of criticals means
  nothing without how much of the tree was read and whether the scan reached
  the network.
  """

  import Davinci.Ui

  alias Davinci.{Model, Theme}

  @severity 9

  def lines(model) do
    th = model.theme
    w = min(model.width, measure() + 14)
    scan = Model.security_scan()
    list = scan.findings
    selected = rem(model.security_index, length(list))

    progress =
      line([
        t(Theme.spinner(model.tick, model.animate) <> " ", th.primary, nil, th.emphasis),
        t("validating candidate #{scan.validated} of #{scan.candidates}", th.text),
        t("  ", th.border)
      ] ++ meter(scan.fraction, 18, th))

    coverage =
      line([
        t("#{scan.files} files", th.muted),
        t(" · ", th.border),
        t("#{scan.skipped} skipped", th.muted),
        t(" · ", th.border),
        t("#{scan.bytes} read", th.muted),
        t(" · ", th.border),
        t(Theme.glyph(:done) <> " network not used", th.success)
      ])

    chips = [
      line(
        Enum.flat_map(scan.severities, fn {label, count, color} ->
          [t(" #{label} #{count} ", severity_color(color, th)), t(" ", th.border)]
        end)
      ),
      line([t("#{scan.dismissed} candidates dismissed as false positives", th.border)])
    ]

    # Below 88 the severity word goes, not the location: the chips above already
    # count the severities, and a finding without its line cannot be checked.
    severity? = model.width >= 88

    rows =
      list
      |> Enum.with_index()
      |> Enum.flat_map(fn {finding, index} ->
        if index == selected,
          do: [row(finding, true, severity?, w, th)] ++ expansion(finding, th),
          else: [row(finding, false, severity?, w, th)]
      end)

    seal = [
      line([
        t("every finding was read out of the file, not guessed", th.muted),
        t(" · line and evidence attached", th.border)
      ]),
      line([
        t("report sealed ", th.muted),
        t(Theme.glyph(:done), th.success, nil, th.emphasis),
        t(" sha256 " <> scan.seal, th.border),
        t("  " <> scan.report, th.border)
      ]),
      line([
        t("enter open the file at the line", th.border),
        t(" · ", th.border),
        t("f mark false positive", th.border)
      ]),
      line([
        t("a abort scan", th.border),
        t(" · ", th.border),
        t("esc close", th.border)
      ])
    ]

    [progress, coverage, blank()] ++ chips ++ [blank()] ++ rows ++ [blank()] ++ seal
  end

  defp row(finding, selected?, severity?, w, th) do
    bg = if selected?, do: th.surface, else: nil
    state = state_for(finding.severity)
    color = severity_color(finding.severity, th)

    left = [
      t(Theme.glyph(state) <> " ", color, bg, th.emphasis),
      t(clip(finding.message, 40), if(selected?, do: th.text, else: th.muted), bg)
    ]

    # The location keeps its line number: a finding you cannot open is a rumour.
    right =
      [t(String.pad_trailing(clip(finding.location, 34), 35), th.border, bg)] ++
        if severity? do
          [t(String.pad_leading(to_string(finding.severity), @severity), color, bg)]
        else
          []
        end

    pad = max(w - seg_len(left) - seg_len(right), 1)
    line([left, sp(pad, bg), right])
  end

  defp expansion(finding, th) do
    [
      indent(2, [
        t("rule ", th.muted),
        t(finding.rule, th.text),
        t(" · validated ", th.muted),
        t(Theme.glyph(:done), th.success, nil, th.emphasis),
        t(" · evidence ", th.muted),
        t(clip(finding.evidence, 40), th.border)
      ]),
      indent(2, [t("path " <> finding.path, th.border)])
    ]
  end

  defp state_for(:critical), do: :failed
  defp state_for(:dismissed), do: :skipped
  defp state_for(_), do: :attention

  defp severity_color(:critical, th), do: th.error
  defp severity_color(:high, th), do: th.warning
  defp severity_color(:medium, th), do: th.muted
  defp severity_color(:dismissed, th), do: th.border
  defp severity_color(_low, th), do: th.border
end
