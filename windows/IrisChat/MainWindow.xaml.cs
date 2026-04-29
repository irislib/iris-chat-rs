using System.Windows;

namespace IrisChat;

public partial class MainWindow : Window
{
    public MainWindow(bool startMinimized = false)
    {
        InitializeComponent();
        DataContext = App.CurrentManager;
        if (startMinimized)
        {
            WindowState = System.Windows.WindowState.Minimized;
        }
    }
}
