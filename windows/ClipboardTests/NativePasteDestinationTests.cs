using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using IrisChat;
using IrisChat.Chrome;

internal static class NativePasteDestinationTests
{
    internal static void Verify(ComposerBar composer, TextBox input, string text)
    {
        var originalScope = composer.AttachmentPasteScope;
        var direct = (CheckBox)composer.FindName("DirectMode");
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
                var delayed = new DelayedText(text, () =>
                {
                    if (action == "clear") composer.Clear();
                    else if (action == "chat") chat = "another-chat";
                    else if (action == "mode") direct.IsChecked = false;
                    else { input.AppendText("typed "); input.CaretIndex = input.Text.Length; }
                });
                DataObjectPastingEventHandler replace = (_, e) =>
                {
                    e.DataObject = delayed;
                    e.FormatToApply = DataFormats.UnicodeText;
                };
                DataObject.AddPastingHandler(input, replace);
                try { ApplicationCommands.Paste.Execute(null, input); }
                finally { DataObject.RemovePastingHandler(input, replace); }
                Check(delayed.Reads == 1, "native Paste reads delayed replacement exactly once");
                if (action == "typing")
                    Check(input.Text == "Caption typed " + text.Replace('\t', ' '),
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
