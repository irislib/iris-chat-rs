using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using IrisChat;
using IrisChat.Chrome;

internal static class NativePasteDestinationTests
{
    internal static void Verify(ComposerBar composer, TextBox input, string text, string output)
    {
        var originalScope = composer.AttachmentPasteScope;
        var direct = (CheckBox)composer.FindName("DirectMode");
        var results = new List<object>();
        try
        {
            foreach (var action in new[] { "clear", "chat", "mode", "typing" })
            {
                composer.Clear();
                var chat = "native-paste-chat";
                composer.AttachmentPasteScope = () => new AttachmentPasteDestination("account", chat);
                input.Text = "Caption ";
                input.CaretIndex = input.Text.Length;
                direct.IsChecked = true;
                Clipboard.SetText(text);
                int? caretBeforeTyping = null, caretAfterAppend = null, deferredLengthAfterAppend = null, caretAfterTyping = null;
                var delayed = new DelayedText(text, () =>
                {
                    if (action == "clear") composer.Clear();
                    else if (action == "chat") chat = "another-chat";
                    else if (action == "mode") direct.IsChecked = false;
                    else
                    {
                        const string typed = "typed ";
                        caretBeforeTyping = input.CaretIndex;
                        input.AppendText(typed);
                        caretAfterAppend = input.CaretIndex;
                        deferredLengthAfterAppend = input.Text.Length;
                        // Native Paste holds a change block: the Text DP can
                        // still contain the old draft until that block closes.
                        // Real typing advances the live insertion position.
                        input.CaretIndex = caretBeforeTyping.Value + typed.Length;
                        caretAfterTyping = input.CaretIndex;
                    }
                });
                DataObjectPastingEventHandler replace = (_, e) =>
                {
                    e.DataObject = delayed;
                    e.FormatToApply = DataFormats.UnicodeText;
                };
                DataObject.AddPastingHandler(input, replace);
                try { ApplicationCommands.Paste.Execute(null, input); }
                finally { DataObject.RemovePastingHandler(input, replace); }
                results.Add(new
                {
                    action, data_reads = delayed.Reads,
                    caret_before_typing = caretBeforeTyping, caret_after_append = caretAfterAppend,
                    deferred_text_length_after_append = deferredLengthAfterAppend,
                    live_caret_after_typing = caretAfterTyping,
                    final_text_length = input.Text.Length,
                    typed_caption_and_native_payload_preserved = action == "typing"
                        ? input.Text == "Caption typed " + text : (bool?)null,
                });
                File.WriteAllText(Path.Combine(output, "windows-reentrant-text-timings.json"),
                    JsonSerializer.Serialize(new { cases = results }, new JsonSerializerOptions { WriteIndented = true }));
                Check(delayed.Reads == 1, "native Paste reads delayed replacement exactly once");
                if (action == "typing")
                    Check(input.Text == "Caption typed " + text,
                        "typing during native clipboard rendering keeps the same draft and native insertion");
                else
                    Check(input.Text == (action == "clear" ? "" : "Caption "),
                        $"native clipboard rendering cannot insert into a changed {action} destination");
                Check(composer.StagedFilePaths.Count == 0, "delayed text never stages attachments");
            }
            VerifyNativeTextFallback(composer, input, text);
        }
        finally
        {
            composer.Clear();
            composer.AttachmentPasteScope = originalScope;
            direct.IsChecked = false;
        }
    }

    private static void VerifyNativeTextFallback(ComposerBar composer, TextBox input, string text)
    {
        foreach (var clearDuringRead in new[] { false, true })
        {
            composer.Clear();
            input.Text = "Caption ";
            input.CaretIndex = input.Text.Length;
            var source = new DelayedText(text, () => { if (clearDuringRead) composer.Clear(); },
                onReadFormat: DataFormats.Text);
            Clipboard.SetDataObject(source, false);
            var missingUnicode = new DelayedText(text, () => {}, missingUnicode: true);
            DataObjectPastingEventHandler replace = (_, e) =>
            {
                e.DataObject = missingUnicode;
                e.FormatToApply = DataFormats.UnicodeText;
            };
            DataObject.AddPastingHandler(input, replace);
            try { ApplicationCommands.Paste.Execute(null, input); }
            finally { DataObject.RemovePastingHandler(input, replace); }
            Check(source.Reads == 1, "Unicode-null fallback reads the original native Text provider");
            Check(input.Text == (clearDuringRead ? "" : "Caption wrong first format"),
                "native Text fallback is preserved and cannot enter a changed draft");
        }
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }

    private sealed class DelayedText : IDataObject
    {
        private readonly DataObject _data = new();
        private Action? _onRead;
        private readonly string? _onReadFormat;
        private readonly bool _missingUnicode;
        internal int Reads;
        internal DelayedText(string text, Action onRead, string? onReadFormat = null, bool missingUnicode = false)
        {
            _data.SetData(DataFormats.Text, "wrong first format");
            _data.SetData(DataFormats.UnicodeText, text);
            _onRead = onRead;
            _onReadFormat = onReadFormat;
            _missingUnicode = missingUnicode;
        }
        public object? GetData(string format, bool autoConvert)
        {
            if (_missingUnicode && format == DataFormats.UnicodeText) return null;
            if (_onRead is {} callback && (_onReadFormat == null || _onReadFormat == format))
            { _onRead = null; Reads++; callback(); }
            return _data.GetData(format, autoConvert);
        }
        public object? GetData(string format) => GetData(format, true);
        public object? GetData(Type format) => GetData(format.FullName!, true);
        public bool GetDataPresent(string format, bool autoConvert) => _data.GetDataPresent(format, autoConvert);
        public bool GetDataPresent(string format) => _data.GetDataPresent(format);
        public bool GetDataPresent(Type format) => _data.GetDataPresent(format);
        public string[] GetFormats(bool autoConvert) => _data.GetFormats(autoConvert);
        public string[] GetFormats() => _data.GetFormats();
        public void SetData(string format, object data, bool autoConvert) => _data.SetData(format, data, autoConvert);
        public void SetData(string format, object data) => _data.SetData(format, data);
        public void SetData(Type format, object data) => _data.SetData(format, data);
        public void SetData(object data) => _data.SetData(data);
    }
}
