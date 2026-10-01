using System;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using IrisChat.Chrome;

internal static class RestoredDraftLayoutTests
{
    internal static void Verify(Window window, ComposerBar composer, string text, Action pump, string output)
    {
        var input = (TextBox)composer.FindName("Input");
        var width = window.Width;
        composer.Clear();
        using var probe = new PasteLayoutProbe();
        // Restored drafts enter through the native Text property, not Paste.
        input.Text = text;
        input.Select(20, 6);
        window.UpdateLayout(); pump();
        probe.Finish();
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Visible,
            "restored bulk drafts reserve scrollbar width");
        Check(input.SelectionStart == 20 && input.SelectionLength == 6 && input.VerticalOffset == 0,
            "restored draft layout preserves its selection without jumping to the end");
        input.ScrollToVerticalOffset(300);
        window.UpdateLayout(); pump();
        var userOffset = input.VerticalOffset;
        Check(userOffset > 0, "native user scrolling moves within the restored draft");
        try
        {
            window.Width += 20;
            window.UpdateLayout(); pump();
            Check(input.SelectionStart == 20 && input.SelectionLength == 6 &&
                Math.Abs(input.VerticalOffset - userOffset) <= 1,
                "relayout retains the user's scroll position and selection");
            File.WriteAllText(Path.Combine(output, "windows-restored-draft-timings.json"), JsonSerializer.Serialize(new
            {
                utf16_code_units = text.Length, restore_layout_drain_ms = probe.ElapsedMs,
                first_render_ms = probe.FirstRenderMs,
                longest_dispatcher_gap_ms = probe.LongestDispatcherGapMs,
                restored_selection_start = input.SelectionStart, restored_selection_length = input.SelectionLength,
                user_scroll_offset = userOffset, after_resize_scroll_offset = input.VerticalOffset,
                boundary = "Restored Text property and selection through first layout/drain; no forced tail geometry or scroll-to-caret. Native user scroll and resize are verified separately.",
            }, new JsonSerializerOptions { WriteIndented = true }));
        }
        finally
        {
            window.Width = width;
            composer.Clear(); pump();
        }
        Check(input.VerticalScrollBarVisibility == ScrollBarVisibility.Auto,
            "clearing a restored draft restores the normal small input");
        Console.WriteLine("PASS: restored bulk draft preserves selection and user scroll, then returns to small-draft layout");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
