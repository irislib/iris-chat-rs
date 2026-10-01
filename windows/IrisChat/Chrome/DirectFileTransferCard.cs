using System;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Controls;
using System.Windows.Media;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

public sealed class DirectFileTransferCard : StackPanel
{
    public DirectFileTransferCard(string chatId, DirectFileTransferSnapshot transfer)
    {
        AutomationProperties.SetAutomationId(this, $"chatDirectTransfer-{transfer.id}");
        Children.Add(Label("Direct files", bold: true));
        foreach (var file in transfer.files)
        {
            var row = new DockPanel { Margin = new Thickness(0, 3, 0, 3) };
            if (transfer.status == DirectFileTransferStatus.Completed && file.localPath is {} path)
            {
                var open = Action("Open", transfer.id, () => PlatformDocumentOpener.Open(path));
                DockPanel.SetDock(open, Dock.Right);
                row.Children.Add(open);
            }
            row.Children.Add(Label($"{file.filename} · {Size(file.sizeBytes)}"));
            Children.Add(row);
        }
        Children.Add(Label(Status(transfer)));
        if (transfer.status is DirectFileTransferStatus.Connecting or DirectFileTransferStatus.Transferring)
        {
            Children.Add(new ProgressBar {
                Minimum = 0, Maximum = 1, Height = 4, Margin = new Thickness(0, 5, 0, 5),
                Value = transfer.totalBytes == 0 ? 0 : Math.Clamp((double)transfer.transferredBytes / transfer.totalBytes, 0, 1),
            });
        }
        if (!string.IsNullOrEmpty(transfer.error)) Children.Add(Label(transfer.error));
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 5, 0, 0) };
        // Account-level direction is insufficient: another own device can accept a self-chat offer.
        if (transfer.status == DirectFileTransferStatus.Offered && !transfer.isSender)
        {
            actions.Children.Add(Action("Accept", transfer.id, () => App.CurrentManager.AcceptDirectFiles(chatId, transfer.id)));
            actions.Children.Add(Action("Decline", transfer.id, () => App.CurrentManager.DeclineDirectFiles(chatId, transfer.id)));
        }
        else if (transfer.status is DirectFileTransferStatus.Offered or DirectFileTransferStatus.Connecting or DirectFileTransferStatus.Transferring)
        {
            actions.Children.Add(Action("Cancel", transfer.id, () => App.CurrentManager.CancelDirectFiles(chatId, transfer.id)));
        }
        Children.Add(actions);
    }

    private static TextBlock Label(string text, bool bold = false) => new() {
        Text = text, Foreground = Brushes.White, TextWrapping = TextWrapping.Wrap,
        FontWeight = bold ? FontWeights.SemiBold : FontWeights.Normal,
        VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Left, MaxWidth = 360,
    };

    private Button Action(string label, string transferId, Action callback)
    {
        var button = new Button {
            Content = label, Style = (Style)FindResource("GhostButton"), Margin = new Thickness(0, 0, 6, 0),
        };
        AutomationProperties.SetAutomationId(button, $"chatDirectTransfer{label}-{transferId}");
        button.Click += (_, _) => callback();
        return button;
    }

    private static string Status(DirectFileTransferSnapshot transfer) => transfer.status switch {
        DirectFileTransferStatus.Offered => transfer.isSender ? "Waiting for acceptance" : "Ready to receive",
        DirectFileTransferStatus.Connecting => "Connecting…",
        DirectFileTransferStatus.Transferring => transfer.isSender ? "Sending…" : "Receiving…",
        DirectFileTransferStatus.Completed => "Complete",
        DirectFileTransferStatus.Declined => "Declined",
        DirectFileTransferStatus.Cancelled => "Cancelled",
        DirectFileTransferStatus.Failed => "Couldn’t send files",
        _ => "Unavailable",
    };

    private static string Size(ulong bytes) => bytes < 1024 ? $"{bytes} B"
        : bytes < 1024 * 1024 ? $"{bytes / 1024.0:F1} KB" : $"{bytes / (1024.0 * 1024.0):F1} MB";
}
