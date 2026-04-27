using System;
using System.ComponentModel;
using System.Linq;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Views;

public partial class NewGroupView : UserControl
{
    public NewGroupView()
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
        CreateButton.IsEnabled = !App.CurrentManager.Busy.creatingGroup;

    private void OnCreate(object sender, RoutedEventArgs e)
    {
        var name = NameInput.Text?.Trim();
        if (string.IsNullOrEmpty(name)) return;

        var members = (MembersInput.Text ?? string.Empty)
            .Split(new[] { '\n', '\r', ',', ';' }, StringSplitOptions.RemoveEmptyEntries)
            .Select(s => s.Trim())
            .Where(s => s.Length > 0)
            .ToArray();
        if (members.Length == 0) return;

        App.CurrentManager.CreateGroup(name, members);
    }
}
