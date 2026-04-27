using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.IO;
using System.Linq;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace IrisChat.Chrome;

public partial class ComposerBar : UserControl
{
    public event Action<string, IList<string>>? Submitted;
    public event Action? AttachRequested;
    public event Action? Typing;
    public event Action? StoppedTyping;

    private bool _wasTyping;
    private readonly ObservableCollection<StagedAttachmentItem> _staged = new();

    public ComposerBar()
    {
        InitializeComponent();
        StagedAttachmentsList.ItemsSource = _staged;
    }

    public void Clear()
    {
        Input.Clear();
        _staged.Clear();
        UpdateStagedVisibility();
        _wasTyping = false;
    }

    public void AddAttachments(IEnumerable<string> filePaths)
    {
        foreach (var path in filePaths)
        {
            if (string.IsNullOrWhiteSpace(path)) continue;
            if (_staged.Any(a => a.FilePath == path)) continue;
            _staged.Add(new StagedAttachmentItem(path));
        }
        UpdateStagedVisibility();
    }

    public IList<string> StagedFilePaths => _staged.Select(a => a.FilePath).ToList();

    public void FocusInput()
    {
        Input.Focus();
        Keyboard.Focus(Input);
    }

    private void OnInputKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter && (Keyboard.Modifiers & ModifierKeys.Shift) == 0)
        {
            e.Handled = true;
            Submit();
        }
    }

    private void OnInputTextChanged(object sender, TextChangedEventArgs e)
    {
        var hasText = !string.IsNullOrWhiteSpace(Input.Text);
        if (hasText && !_wasTyping) { _wasTyping = true; Typing?.Invoke(); }
        else if (!hasText && _wasTyping) { _wasTyping = false; StoppedTyping?.Invoke(); }
    }

    private void OnSend(object sender, RoutedEventArgs e) => Submit();

    private void OnAttach(object sender, RoutedEventArgs e) => AttachRequested?.Invoke();

    private void OnRemoveAttachment(object sender, RoutedEventArgs e)
    {
        if (sender is Button btn && btn.Tag is string path)
        {
            var match = _staged.FirstOrDefault(a => a.FilePath == path);
            if (match != null) _staged.Remove(match);
            UpdateStagedVisibility();
        }
    }

    private void Submit()
    {
        var text = Input.Text?.Trim() ?? string.Empty;
        var paths = StagedFilePaths;
        if (string.IsNullOrEmpty(text) && paths.Count == 0) return;
        Submitted?.Invoke(text, paths);
        Input.Clear();
        _staged.Clear();
        UpdateStagedVisibility();
        if (_wasTyping) { _wasTyping = false; StoppedTyping?.Invoke(); }
    }

    private void UpdateStagedVisibility()
    {
        StagedAttachmentsList.Visibility = _staged.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
    }
}

public sealed class StagedAttachmentItem
{
    public string FilePath { get; }
    public string Filename { get; }
    public string Icon { get; }

    public StagedAttachmentItem(string filePath)
    {
        FilePath = filePath;
        Filename = Path.GetFileName(filePath);
        Icon = IconForExtension(Path.GetExtension(filePath));
    }

    private static string IconForExtension(string ext)
    {
        ext = (ext ?? string.Empty).TrimStart('.').ToLowerInvariant();
        return ext switch
        {
            "jpg" or "jpeg" or "png" or "gif" or "webp" or "bmp" or "heic" or "heif" => "🖼",
            "mp4" or "mov" or "mkv" or "webm" or "avi" => "🎬",
            "mp3" or "wav" or "ogg" or "flac" or "m4a" or "aac" => "🎵",
            "zip" or "tar" or "gz" or "rar" or "7z" => "🗜",
            "pdf" => "📕",
            "doc" or "docx" or "txt" or "md" => "📄",
            _ => "📎",
        };
    }
}
