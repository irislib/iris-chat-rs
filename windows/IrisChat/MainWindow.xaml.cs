using System.Windows;

namespace IrisChat;

public partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();
        DataContext = App.CurrentManager;
    }
}
