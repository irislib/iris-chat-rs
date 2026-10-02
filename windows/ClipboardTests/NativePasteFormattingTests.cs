using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using IrisChat;
using IrisChat.Chrome;

internal static class NativePasteFormattingTests
{
    internal static void Verify(Window window, ComposerBar composer, string text, Action pump, string output)
    {
        var input = (TextBox)composer.FindName("Input");
        var originalMode = TextOptions.GetTextFormattingMode(input);
        var cases = new List<object>();
        try
        {
            foreach (var mode in new[] { TextFormattingMode.Ideal, TextFormattingMode.Display })
            {
                composer.Clear();
                TextOptions.SetTextFormattingMode(input, mode);
                input.Focus(); pump();
                Clipboard.SetText(text);
                var changes = 0;
                TextChangedEventHandler changed = (_, _) => changes++;
                input.TextChanged += changed;
                IReadOnlyDictionary<string, double>? phases = null;
                PastePhaseDiagnostics.Observer = value => phases = value;
                using var probe = new PasteLayoutProbe();
                try
                {
                    ApplicationCommands.Paste.Execute(null, input);
                    var executeMs = probe.ElapsedMs;
                    pump(); window.UpdateLayout(); pump();
                    var caret = input.GetRectFromCharacterIndex(input.CaretIndex);
                    window.UpdateLayout(); pump();
                    probe.WaitForFirstRender();
                    caret = input.GetRectFromCharacterIndex(input.CaretIndex);
                    var settledMs = probe.ElapsedMs;
                    var visible = ComposerViewport.Contains(input, caret);
                    var completeText = input.Text == text && changes == 1;
                    var stableWidth = input.VerticalScrollBarVisibility == ScrollBarVisibility.Visible;
                    input.SelectedText = "!";
                    window.UpdateLayout(); pump();
                    probe.Finish();
                    var completeEdit = input.Text == text + "!" && changes == 2;
                    cases.Add(new
                    {
                        mode = mode.ToString(), utf16_code_units = text.Length,
                        utf8_bytes = Encoding.UTF8.GetByteCount(text),
                        paste_execute_ms = executeMs, complete_paste_ms = settledMs,
                        first_render_ms = probe.FirstRenderMs,
                        subsequent_edit_ms = probe.ElapsedMs - settledMs,
                        complete_paste_and_edit_ms = probe.ElapsedMs,
                        longest_dispatcher_gap_ms = probe.LongestDispatcherGapMs,
                        native_paste_phases_ms = phases, stable_visible_scrollbar = stableWidth,
                        complete_text_preserved = completeText, complete_edit_preserved = completeEdit,
                        caret_visible = visible, text_change_events = changes,
                    });
                    File.WriteAllText(Path.Combine(output, "windows-native-format-timings.json"),
                        JsonSerializer.Serialize(new
                        {
                            boundary = "Paired warm native Paste after the unchanged cold production case; production reserves Visible width before insertion. Completion includes layout/drain, refreshed end-caret geometry and the first WPF Rendering callback, then a native edit and layout/drain. Rendering is not display scanout; heartbeat gaps include scheduler noise.",
                            cases,
                        }, new JsonSerializerOptions { WriteIndented = true }));
                    if (!completeText || !completeEdit || !visible || !stableWidth)
                        throw new InvalidOperationException($"{mode}: native paste must preserve complete text, next edit and visible caret at stable width");
                    if (probe.ElapsedMs >= 5_000)
                        throw new InvalidOperationException($"{mode}: native paste and next edit exceeded the unchanged 5 s budget");
                }
                finally { input.TextChanged -= changed; PastePhaseDiagnostics.Observer = null; }
            }
        }
        finally { composer.Clear(); TextOptions.SetTextFormattingMode(input, originalMode); pump(); }
        Console.WriteLine("PASS: native Ideal/Display comparison preserves text, caret and following edit with stable Visible width");
    }
}
