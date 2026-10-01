using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using IrisChat.Chrome;

internal static class PasteLayoutComparison
{
    internal static void Verify(Window window, ComposerBar composer, string text, Action pump, string output)
    {
        var input = (TextBox)composer.FindName("Input");
        var originalMode = TextOptions.GetTextFormattingMode(input);
        var originalScroll = input.VerticalScrollBarVisibility;
        var results = new List<object>();
        try
        {
            foreach (var (name, mode, scroll) in new[]
            {
                ("Ideal+Auto", TextFormattingMode.Ideal, ScrollBarVisibility.Auto),
                ("Ideal+Visible", TextFormattingMode.Ideal, ScrollBarVisibility.Visible),
                ("Ideal+Hidden", TextFormattingMode.Ideal, ScrollBarVisibility.Hidden),
            })
            {
                composer.Clear();
                TextOptions.SetTextFormattingMode(input, mode);
                input.VerticalScrollBarVisibility = scroll;
                input.Focus(); pump();
                Clipboard.SetText(text);
                var changes = 0;
                TextChangedEventHandler changed = (_, _) => changes++;
                input.TextChanged += changed;
                double? firstExtent = null, firstOffset = null, firstViewport = null;
                using var probe = new PasteLayoutProbe(() =>
                {
                    // These getters read existing scroll metrics without
                    // validating caret geometry or formatting the document tail.
                    firstExtent = input.ExtentHeight;
                    firstOffset = input.VerticalOffset;
                    firstViewport = input.ViewportHeight;
                });
                ApplicationCommands.Paste.Execute(null, input);
                var executeMs = probe.ElapsedMs;
                pump();
                var firstDrainMs = probe.ElapsedMs;
                window.UpdateLayout();
                var layoutMs = probe.ElapsedMs;
                pump();
                var finalDrainMs = probe.ElapsedMs;
                probe.WaitForFirstRender();
                var renderedMs = probe.ElapsedMs;
                var gapBeforeForcedCaretMs = probe.CheckpointGap();
                // Force completion at the actual caret separately. Otherwise
                // Background layout can otherwise leave the tail unfinished.
                var caret = input.GetRectFromCharacterIndex(input.CaretIndex);
                window.UpdateLayout(); pump();
                // Geometry captured before the drain can precede queued native
                // scrolling. Refresh it before deciding whether help is needed.
                caret = input.GetRectFromCharacterIndex(input.CaretIndex);
                var geometryReadyMs = probe.ElapsedMs;
                var visibleBeforeScroll = IsCaretVisible(input, caret);
                if (!visibleBeforeScroll)
                {
                    input.ScrollToLine(input.GetLineIndexFromCharacterIndex(input.CaretIndex));
                    window.UpdateLayout(); pump();
                    caret = input.GetRectFromCharacterIndex(input.CaretIndex);
                }
                var caretReadyMs = probe.ElapsedMs;
                var caretVisible = IsCaretVisible(input, caret);
                var completedExtent = input.ExtentHeight;
                input.SelectedText = "!";
                window.UpdateLayout(); pump();
                probe.Finish();
                input.TextChanged -= changed;
                var correct = input.Text == text + "!" && changes == 2;
                results.Add(new
                {
                    configuration = name, utf16_code_units = text.Length,
                    utf8_bytes = Encoding.UTF8.GetByteCount(text),
                    paste_execute_ms = executeMs,
                    first_dispatcher_drain_ms = firstDrainMs - executeMs,
                    explicit_layout_ms = layoutMs - firstDrainMs,
                    final_dispatcher_drain_ms = finalDrainMs - layoutMs,
                    first_render_wait_ms = renderedMs - finalDrainMs,
                    first_render_ms = probe.FirstRenderMs,
                    first_render_extent_height = firstExtent,
                    first_render_vertical_offset = firstOffset,
                    first_render_viewport_height = firstViewport,
                    completed_extent_height = completedExtent,
                    first_render_document_tail_visible = firstExtent >= completedExtent - 1 &&
                        firstOffset + firstViewport >= completedExtent - 1,
                    longest_gap_before_forced_caret_ms = gapBeforeForcedCaretMs,
                    caret_geometry_layout_drain_ms = geometryReadyMs - renderedMs,
                    caret_visible_before_explicit_scroll = visibleBeforeScroll,
                    explicit_native_caret_scroll_ms = caretReadyMs - geometryReadyMs,
                    caret_ready_ms = caretReadyMs,
                    subsequent_edit_ms = probe.ElapsedMs - caretReadyMs,
                    complete_ms = probe.ElapsedMs,
                    longest_dispatcher_gap_ms = probe.LongestDispatcherGapMs,
                    caret_visible = caretVisible,
                    text_change_events = changes, complete_text_and_edit_preserved = correct,
                });
                File.WriteAllText(Path.Combine(output, "windows-layout-comparison-timings.json"), JsonSerializer.Serialize(new
                {
                    boundary = "Native Paste through first WPF Rendering callback (only existing scroll metrics sampled), explicit end-caret geometry (forces pending tail layout), native ScrollToLine only if caret remains outside viewport after drain, then SelectedText edit and layout/drain. First-frame document-tail visibility compares that frame's viewport with the completed extent; final caret visibility uses refreshed geometry. Input-priority 1 ms heartbeat gaps include scheduling noise; not a frame-rate measurement.",
                    configurations = results,
                }, new JsonSerializerOptions { WriteIndented = true }));
                if (!correct || !caretVisible || probe.ElapsedMs >= 5_000)
                    throw new InvalidOperationException($"{name}: whole paste and following edit must remain intact, caret visible, and complete within the existing 5 s budget");
            }
        }
        finally
        {
            composer.Clear();
            TextOptions.SetTextFormattingMode(input, originalMode);
            input.VerticalScrollBarVisibility = originalScroll;
            pump();
        }
        Console.WriteLine("PASS: real composer Auto/Visible/Hidden layout comparison preserves whole text, visible caret and next edit");
    }

    private static bool IsCaretVisible(TextBox input, Rect caret) =>
        !caret.IsEmpty && caret.Top >= 0 && caret.Bottom <= input.ActualHeight;
}
