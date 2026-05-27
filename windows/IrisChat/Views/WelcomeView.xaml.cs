using System.Windows;
using System.Windows.Controls;
using IrisChat.Bindings;

namespace IrisChat.Views;

public partial class WelcomeView : UserControl
{
    public WelcomeView()
    {
        InitializeComponent();
    }

    private void OnCreate(object sender, RoutedEventArgs e) =>
        App.CurrentManager.Push(new Screen.CreateAccount());

    private void OnRestore(object sender, RoutedEventArgs e) =>
        App.CurrentManager.Push(new Screen.RestoreAccount());
}
