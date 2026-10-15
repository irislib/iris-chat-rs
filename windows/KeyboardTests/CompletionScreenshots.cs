using System;
using System.IO;
using System.Reflection;
using System.Windows;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using IrisChat.Chrome;

internal static class CompletionScreenshots
{
    internal static void Menu(ComposerBar composer, string controller, string name)
    {
        var completion = typeof(ComposerBar).GetField(controller, BindingFlags.Instance | BindingFlags.NonPublic)!
            .GetValue(composer)!;
        var popup = (Popup)completion.GetType().GetField("_popup", BindingFlags.Instance | BindingFlags.NonPublic)!
            .GetValue(completion)!;
        if (!popup.IsOpen) throw new Exception("Completion screenshot requires the real visible popup");
        Save((FrameworkElement)popup.Child, name + "-suggestions");
        Save(composer, name + "-composer");
    }

    internal static void Save(FrameworkElement view, string name)
    {
        view.UpdateLayout();
        var bitmap = new RenderTargetBitmap((int)Math.Ceiling(view.ActualWidth),
            (int)Math.Ceiling(view.ActualHeight), 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(view);
        var png = new PngBitmapEncoder();
        png.Frames.Add(BitmapFrame.Create(bitmap));
        var output = Path.GetDirectoryName(Environment.GetEnvironmentVariable("IRIS_UI_TEST_DATA_DIR"))!;
        using var file = File.Create(Path.Combine(output, name + ".png"));
        png.Save(file);
    }
}
