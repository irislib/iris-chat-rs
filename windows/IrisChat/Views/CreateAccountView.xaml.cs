using System;
using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace IrisChat.Views;

public partial class CreateAccountView : UserControl
{
    public CreateAccountView()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            App.CurrentManager.PropertyChanged += OnChanged;
            UpdateBusy();
            Dispatcher.BeginInvoke(new Action(() =>
            {
                NameInput.Focus();
                Keyboard.Focus(NameInput);
            }));
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => UpdateBusy();

    private void UpdateBusy() =>
        CreateButton.IsEnabled = !App.CurrentManager.Busy.creatingAccount;

    private void OnCreate(object sender, RoutedEventArgs e)
    {
        SubmitCreateAccount();
    }

    private void OnNameInputKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter) return;
        e.Handled = true;
        SubmitCreateAccount();
    }

    private void SubmitCreateAccount()
    {
        var name = NameInput.Text?.Trim();
        if (string.IsNullOrEmpty(name) || App.CurrentManager.Busy.creatingAccount) return;
        CreateButton.IsEnabled = false;
        App.CurrentManager.CreateAccount(name);
    }
}
