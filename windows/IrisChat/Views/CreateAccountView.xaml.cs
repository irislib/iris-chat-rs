using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;

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
            NameInput.Focus();
        };
        Unloaded += (_, _) => App.CurrentManager.PropertyChanged -= OnChanged;
    }

    private void OnChanged(object? sender, PropertyChangedEventArgs e) => UpdateBusy();

    private void UpdateBusy() =>
        CreateButton.IsEnabled = !App.CurrentManager.Busy.creatingAccount;

    private void OnCreate(object sender, RoutedEventArgs e)
    {
        var name = NameInput.Text?.Trim();
        if (string.IsNullOrEmpty(name)) return;
        App.CurrentManager.CreateAccount(name);
    }
}
