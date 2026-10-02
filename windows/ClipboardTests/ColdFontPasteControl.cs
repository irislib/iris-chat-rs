using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Chrome;

internal static class ColdFontPasteControl
{
    internal static int Run(string output)
    {
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_RUN_ID", Guid.NewGuid().ToString());
        Environment.SetEnvironmentVariable("IRIS_UI_TEST_DATA_DIR", Path.Combine(output, "font-control-data"));
        var originalClipboard = Clipboard.GetDataObject();
        var app = new App();
        app.InitializeComponent();
        var composer = new ComposerBar { DirectSendAllowed = true,
            AttachmentPasteScope = () => new AttachmentPasteDestination("font-control", "chat") };
        var window = new Window { Title = "Clipboard font control", Width = 620, Height = 280, Content = composer };
        var input = (TextBox)composer.FindName("Input");
        var results = new List<object>();
        try
        {
            window.Show(); Pump();
            // Match the primary case's short ASCII native-paste preparation.
            // No CJK text is displayed in this process before the tiny control.
            input.Text = "Unsent caption "; input.CaretIndex = input.Text.Length;
            Clipboard.SetText("plain text");
            ApplicationCommands.Paste.Execute(null, input); Pump();
            foreach (var (name, text) in new[]
            {
                ("matched_ascii_first_bulk", string.Concat(Enumerable.Repeat("A long pasted note, with Unicode WW.\r\n", 4096))),
                ("tiny_cjk_first_use_after_ascii", "世界"),
            })
            {
                composer.Clear(); input.Focus(); Pump();
                Clipboard.SetText(text);
                var changes = 0;
                TextChangedEventHandler changed = (_, _) => changes++;
                input.TextChanged += changed;
                IReadOnlyDictionary<string, double>? phases = null;
                PastePhaseDiagnostics.Observer = value => phases = value;
                using var probe = new PasteLayoutProbe();
                try
                {
                    if (!ApplicationCommands.Paste.CanExecute(null, input)) throw new InvalidOperationException("Control native Paste disabled");
                    var canExecuteMs = probe.ElapsedMs;
                    ApplicationCommands.Paste.Execute(null, input);
                    var executedMs = probe.ElapsedMs;
                    Pump(); window.UpdateLayout(); Pump();
                    _ = input.GetRectFromCharacterIndex(input.CaretIndex);
                    window.UpdateLayout(); Pump();
                    probe.WaitForFirstRender();
                    var caret = input.GetRectFromCharacterIndex(input.CaretIndex);
                    var settledMs = probe.ElapsedMs;
                    var completeText = input.Text == text && changes == 1;
                    var visibleCaret = ComposerViewport.Contains(input, caret);
                    input.SelectedText = "!";
                    window.UpdateLayout(); Pump();
                    probe.Finish();
                    var completeEdit = input.Text == text + "!" && changes == 2;
                    results.Add(new
                    {
                        name, utf16_code_units = text.Length, utf8_bytes = Encoding.UTF8.GetByteCount(text),
                        line_breaks = text.Count(character => character == '\n'),
                        can_execute_ms = canExecuteMs, paste_execute_ms = executedMs - canExecuteMs,
                        complete_paste_ms = settledMs, first_render_ms = probe.FirstRenderMs,
                        subsequent_edit_ms = probe.ElapsedMs - settledMs,
                        complete_paste_and_edit_ms = probe.ElapsedMs,
                        longest_dispatcher_gap_ms = probe.LongestDispatcherGapMs,
                        native_paste_phases_ms = phases, complete_text_preserved = completeText,
                        complete_edit_preserved = completeEdit, caret_visible = visibleCaret, text_change_events = changes,
                    });
                    File.WriteAllText(Path.Combine(output, "windows-cold-font-control-timings.json"),
                        JsonSerializer.Serialize(new
                        {
                            process = "Separate fresh child after the original production child; one short ASCII paste prelude, then matched-size ASCII bulk paste, then first tiny CJK paste. Production order, content and 500 ms gate unchanged.",
                            limitation = "Managed/editor state is process-isolated; Windows shared font caches are not reset. ASCII/CJK glyph widths differ, although these fixtures retain equal UTF-16 length and 4096 explicit line breaks. This is a diagnostic control, not a cold-font causal proof or production warmup.",
                            phase_interpretation = "The optional phase trace separates pre-Pasting pending-input/setup, guarded payload reads, insertion-to-TextChanged, application callback and native post-notification completion. It does not independently measure managed JIT, font initialization or internal native formatting.",
                            boundary = "Native Paste through layout/drain, refreshed caret geometry and first WPF Rendering callback, then a native edit and layout/drain. Rendering is not display scanout; heartbeat gaps include scheduler noise.",
                            cases = results,
                        }, new JsonSerializerOptions { WriteIndented = true }));
                    if (!completeText || !completeEdit || !visibleCaret)
                        throw new InvalidOperationException($"{name}: native text, edit and caret must remain intact");
                    if (probe.ElapsedMs >= 5_000) throw new InvalidOperationException($"{name}: control exceeded 5 s budget");
                }
                finally { input.TextChanged -= changed; PastePhaseDiagnostics.Observer = null; }
            }
            Console.WriteLine("PASS: separate-process ASCII and first-use tiny CJK controls preserve native text, edit and caret");
            return 0;
        }
        catch (Exception error) { Console.Error.WriteLine(error); return 1; }
        finally
        {
            composer.Clear(); window.Close();
            if (originalClipboard != null) Clipboard.SetDataObject(originalClipboard, true);
            else Clipboard.Clear();
        }
    }

    private static void Pump()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }
}
