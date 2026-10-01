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
                ("Display+Auto", TextFormattingMode.Display, ScrollBarVisibility.Auto),
                ("Display+Hidden", TextFormattingMode.Display, ScrollBarVisibility.Hidden),
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
                using var probe = new PasteLayoutProbe();
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
                // Hidden can appear faster by leaving tail layout unfinished.
                var caret = input.GetRectFromCharacterIndex(input.CaretIndex);
                window.UpdateLayout(); pump();
                var caretReadyMs = probe.ElapsedMs;
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
                    longest_gap_before_forced_caret_ms = gapBeforeForcedCaretMs,
                    caret_geometry_layout_drain_ms = caretReadyMs - renderedMs,
                    caret_ready_ms = caretReadyMs,
                    subsequent_edit_ms = probe.ElapsedMs - caretReadyMs,
                    complete_ms = probe.ElapsedMs,
                    longest_dispatcher_gap_ms = probe.LongestDispatcherGapMs,
                    caret_visible = !caret.IsEmpty && caret.Top >= 0 && caret.Bottom <= input.ActualHeight,
                    text_change_events = changes, complete_text_and_edit_preserved = correct,
                });
                File.WriteAllText(Path.Combine(output, "windows-layout-comparison-timings.json"), JsonSerializer.Serialize(new
                {
                    boundary = "Native Paste through first WPF Rendering callback, explicit end-caret geometry (forces any pending tail layout), then SelectedText edit and layout/drain. Input-priority 1 ms heartbeat gaps include scheduling noise; not a frame-rate measurement.",
                    configurations = results,
                }, new JsonSerializerOptions { WriteIndented = true }));
                if (!correct || probe.ElapsedMs >= 5_000)
                    throw new InvalidOperationException($"{name}: whole paste and following edit must remain intact and complete within the existing 5 s budget");
            }
        }
        finally
        {
            composer.Clear();
            TextOptions.SetTextFormattingMode(input, originalMode);
            input.VerticalScrollBarVisibility = originalScroll;
            pump();
        }
        Console.WriteLine("PASS: real composer layout comparison preserves whole text and next edit in all three configurations");
    }
}
