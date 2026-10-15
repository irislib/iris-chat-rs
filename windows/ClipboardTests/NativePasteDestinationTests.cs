using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;

internal static class NativePasteDestinationTests
{
    internal static void Verify(ComposerBar composer, TextBox input, string text, string output)
    {
        var originalScope = composer.AttachmentPasteScope;
        var direct = (CheckBox)composer.FindName("DirectMode");
        var results = new List<object>();
        var restored = Native.FormatMessageMention(new string('2', 64), "Alex");
        string saved = "";
        Action<string> recordDraft = value => saved = value;
        composer.DraftChanged += recordDraft;
        try
        {
            foreach (var payload in new[] { text, "short clipboard text" })
            foreach (var action in new[] { "clear", "chat", "mode", "typing", "restore", "provider-error" })
            {
                composer.Clear();
                var chat = "native-paste-chat";
                composer.AttachmentPasteScope = () => new AttachmentPasteDestination("account", chat);
                input.Text = "Caption ";
                input.CaretIndex = input.Text.Length;
                direct.IsChecked = true;
                Clipboard.SetText(payload);
                int? caretBeforeTyping = null, caretAfterAppend = null, deferredLengthAfterAppend = null, caretAfterTyping = null;
                var callbackCompleted = false;
                var delayed = new DelayedText(payload, () =>
                {
                    if (action == "clear") composer.Clear();
                    else if (action == "chat") chat = "another-chat";
                    else if (action == "mode") direct.IsChecked = false;
                    else if (action == "restore")
                    {
                        // A rapid second selection supersedes the first restore,
                        // even when both people have the same visible name.
                        composer.RestoreDraft(Native.FormatMessageMention(new string('1', 64), "Alex"));
                        composer.RestoreDraft(restored);
                    }
                    else if (action == "provider-error") throw new InvalidOperationException("unavailable clipboard provider");
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
                    callbackCompleted = true;
                });
                var pasteEvents = 0;
                DataObjectPastingEventHandler replace = (_, e) =>
                {
                    pasteEvents++;
                    e.DataObject = delayed;
                    e.FormatToApply = DataFormats.UnicodeText;
                };
                DataObject.AddPastingHandler(input, replace);
                try { ApplicationCommands.Paste.Execute(null, input); }
                finally { DataObject.RemovePastingHandler(input, replace); }
                results.Add(new
                {
                    action, payload_length = payload.Length, data_reads = delayed.Reads, paste_events = pasteEvents,
                    callback_completed = callbackCompleted,
                    caret_before_typing = caretBeforeTyping, caret_after_append = caretAfterAppend,
                    deferred_text_length_after_append = deferredLengthAfterAppend,
                    live_caret_after_typing = caretAfterTyping,
                    final_text_length = input.Text.Length,
                    typed_caption_and_native_payload_preserved = action == "typing"
                        ? input.Text == "Caption typed " + payload : (bool?)null,
                });
                File.WriteAllText(Path.Combine(output, "windows-reentrant-text-timings.json"),
                    JsonSerializer.Serialize(new { cases = results }, new JsonSerializerOptions { WriteIndented = true }));
                Check(delayed.Reads == 1, "native Paste reads delayed replacement exactly once");
                Check(pasteEvents == 1, "a failed or canceled owned Paste cannot route a second native Paste");
                Check(callbackCompleted == (action != "provider-error"), "reentrant draft changes complete without a swallowed undo exception");
                if (action == "typing")
                    Check(input.Text == "Caption typed " + payload,
                        "typing during native clipboard rendering keeps the same draft and native insertion");
                else
                    Check(input.Text == (action == "clear" ? "" : action == "restore" ? "@Alex" : "Caption "),
                        $"native clipboard rendering cannot insert into a changed {action} destination");
                if (action is "clear" or "restore")
                {
                    Check(input.IsUndoEnabled && !input.IsReadOnly && !input.CanUndo && !input.CanRedo,
                        "the new draft is editable with fresh undo history before Paste returns");
                    input.CaretIndex = input.Text.Length;
                    input.SelectedText = "!";
                    if (action == "restore") Check(saved == restored + "!", "immediate editing preserves the last restored identity");
                    input.Undo();
                    Check(input.Text == (action == "clear" ? "" : "@Alex") && !input.CanUndo,
                        "immediate editing can undo only within the new draft");
                    if (action == "restore") Check(saved == restored, "native undo preserves the restored identity");
                }
                Check(composer.StagedFilePaths.Count == 0, "delayed text never stages attachments");
            }
            VerifyEditCancellation(composer, input, text);
            VerifyNativeTextFallback(composer, input, text);
        }
        finally
        {
            composer.Clear();
            composer.DraftChanged -= recordDraft;
            composer.AttachmentPasteScope = originalScope;
            direct.IsChecked = false;
        }
    }

    private static void VerifyEditCancellation(ComposerBar composer, TextBox input, string text)
    {
        foreach (var payload in new[] { text, "short clipboard text" })
        {
            composer.Clear();
            composer.RestoreDraft("Unsent caption");
            composer.BeginEdit("message", "Earlier sent message");
            Clipboard.SetText(payload);
            var delayed = new DelayedText(payload, composer.CancelEdit);
            DataObjectPastingEventHandler replace = (_, e) =>
            {
                e.DataObject = delayed;
                e.FormatToApply = DataFormats.UnicodeText;
            };
            DataObject.AddPastingHandler(input, replace);
            try { ApplicationCommands.Paste.Execute(null, input); }
            finally { DataObject.RemovePastingHandler(input, replace); }
            Check(delayed.Reads == 1 && composer.EditingMessageId == null && input.Text == "Unsent caption",
                "canceling a sent-message edit during Paste restores the unsent draft without clipboard content");
            Check(input.IsUndoEnabled && !input.IsReadOnly && !input.CanUndo && !input.CanRedo,
                "canceling an edit closes the native undo unit before resetting restored draft history");
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
