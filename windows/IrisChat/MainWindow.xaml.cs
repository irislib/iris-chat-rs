using System.Windows;

namespace IrisChat;

public partial class MainWindow : Window
{
    public MainWindow(bool startMinimized = false)
    {
        InitializeComponent();
        DataContext = App.CurrentManager;
        Activated += (_, _) => App.CurrentManager.AppWindowActivated();
        Deactivated += (_, _) => App.CurrentManager.AppWindowDeactivated();
        if (startMinimized)
        {
            WindowState = System.Windows.WindowState.Minimized;
        }
    }
}
