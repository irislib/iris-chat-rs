using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace IrisChat.Chrome;

public partial class ComposerBar : UserControl
{
    public event Action<string>? Submitted;
    public event Action? AttachRequested;
    public event Action? Typing;
    public event Action? StoppedTyping;

    private bool _wasTyping;

    public ComposerBar()
    {
        InitializeComponent();
    }

    public void Clear()
    {
        Input.Clear();
        _wasTyping = false;
    }

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

    private void Submit()
    {
        var text = Input.Text?.Trim();
        if (string.IsNullOrEmpty(text)) return;
        Submitted?.Invoke(text);
        Input.Clear();
        if (_wasTyping) { _wasTyping = false; StoppedTyping?.Invoke(); }
    }
}
