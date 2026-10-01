using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using IrisChat;
using IrisChat.Chrome;

internal static class ReentrantPasteTests
{
    internal static void Verify(Window window, BitmapSource bitmap, string originalFile)
    {
        foreach (var action in new[] { "clear", "send", "mode", "typing" })
        {
            var composer = new ComposerBar { DirectSendAllowed = true,
                AttachmentPasteScope = () => new AttachmentPasteDestination("account", "same-chat") };
            window.Content = composer;
            Pump();
            var input = (TextBox)composer.FindName("Input");
            var direct = (CheckBox)composer.FindName("DirectMode");
            input.Text = "Draft being pasted into";
            composer.AddAttachments(new[] { originalFile });
            direct.IsChecked = true;
            var submitted = 0;
            composer.Submitted += (_, _) => submitted++;
            var data = new ReentrantData(bitmap);
            Clipboard.SetDataObject(data, false);
            if (!ApplicationCommands.Paste.CanExecute(null, input)) throw new Exception("Paste disabled");
            data.OnRead = () =>
            {
                if (action == "clear") composer.Clear();
                else if (action == "send") ((Button)composer.FindName("SendButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
                else if (action == "mode") direct.IsChecked = false;
                else input.AppendText(" while pasting");
            };
            ApplicationCommands.Paste.Execute(null, input);
            Pump();
            if (data.Reads != 1) throw new Exception("Clipboard fixture did not render reentrantly");
            var expectedCount = action == "mode" ? 1 : action == "typing" ? 2 : 0;
            if (composer.StagedFilePaths.Count != expectedCount)
                throw new Exception($"Late clipboard image entered a changed {action} draft");
            if (action == "typing" && !input.Text.EndsWith(" while pasting"))
                throw new Exception("Paste discarded a caption edit");
            if (submitted != (action == "send" ? 1 : 0)) throw new Exception("Paste unexpectedly submitted a message");
            composer.Clear();
            window.Content = null;
            Pump();
        }
        Console.WriteLine("PASS: reentrant native clipboard rendering respects Clear, Send, direct mode, and caption edits");
    }

    private static void Pump()
    {
        var frame = new DispatcherFrame();
        Dispatcher.CurrentDispatcher.BeginInvoke(DispatcherPriority.Background, new Action(() => frame.Continue = false));
        Dispatcher.PushFrame(frame);
    }

    // OleGetClipboard/GetData uses this actual delayed data provider. Only data
    // rendering reenters; format enumeration and CanExecute remain side-effect free.
    private sealed class ReentrantData : IDataObject
    {
        private readonly DataObject _data = new();
        internal Action? OnRead;
        internal int Reads;
        internal ReentrantData(BitmapSource bitmap) => _data.SetData(DataFormats.Bitmap, bitmap);
        public object? GetData(string format, bool autoConvert)
        {
            if (OnRead is {} read) { OnRead = null; Reads++; read(); }
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
