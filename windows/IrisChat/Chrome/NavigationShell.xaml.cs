using System;
using System.Windows;
using System.Windows.Controls;

namespace IrisChat.Chrome;

public partial class NavigationShell : UserControl
{
    public static readonly DependencyProperty TitleProperty =
        DependencyProperty.Register(nameof(Title), typeof(string), typeof(NavigationShell),
            new PropertyMetadata(string.Empty, (d, e) => ((NavigationShell)d).TitleText.Text = (string)e.NewValue));

    public static readonly DependencyProperty CanGoBackProperty =
        DependencyProperty.Register(nameof(CanGoBack), typeof(bool), typeof(NavigationShell),
            new PropertyMetadata(false, (d, _) => ((NavigationShell)d).Refresh()));

    public static readonly DependencyProperty BackBadgeCountProperty =
        DependencyProperty.Register(nameof(BackBadgeCount), typeof(ulong), typeof(NavigationShell),
            new PropertyMetadata((ulong)0, (d, _) => ((NavigationShell)d).Refresh()));

    public static readonly DependencyProperty LeadingProperty =
        DependencyProperty.Register(nameof(Leading), typeof(object), typeof(NavigationShell),
            new PropertyMetadata(null, (d, _) => ((NavigationShell)d).Refresh()));

    public static readonly DependencyProperty TrailingProperty =
        DependencyProperty.Register(nameof(Trailing), typeof(object), typeof(NavigationShell),
            new PropertyMetadata(null, (d, _) => ((NavigationShell)d).Refresh()));

    public static readonly DependencyProperty BodyProperty =
        DependencyProperty.Register(nameof(Body), typeof(object), typeof(NavigationShell),
            new PropertyMetadata(null, (d, e) => ((NavigationShell)d).ContentHost.Content = e.NewValue));

    public string Title
    {
        get => (string)GetValue(TitleProperty);
        set => SetValue(TitleProperty, value);
    }

    public bool CanGoBack
    {
        get => (bool)GetValue(CanGoBackProperty);
        set => SetValue(CanGoBackProperty, value);
    }

    public ulong BackBadgeCount
    {
        get => (ulong)GetValue(BackBadgeCountProperty);
        set => SetValue(BackBadgeCountProperty, value);
    }

    public object? Leading
    {
        get => GetValue(LeadingProperty);
        set => SetValue(LeadingProperty, value);
    }

    public object? Trailing
    {
        get => GetValue(TrailingProperty);
        set => SetValue(TrailingProperty, value);
    }

    public object? Body
    {
        get => GetValue(BodyProperty);
        set => SetValue(BodyProperty, value);
    }

    public event Action? BackRequested;

    public NavigationShell()
    {
        InitializeComponent();
        Refresh();
    }

    private void Refresh()
    {
        if (CanGoBack)
        {
            LeadingHost.Content = BuildBackButton();
        }
        else
        {
            LeadingHost.Content = Leading;
        }

        TrailingHost.Content = Trailing;
    }

    private FrameworkElement BuildBackButton()
    {
        var grid = new Grid { Width = 44, Height = 44 };

        var btn = new Button
        {
            Style = (Style)FindResource("IconButton"),
            Padding = new Thickness(8),
            Content = new TextBlock
            {
                Text = "‹", // ‹
                FontSize = 24,
                FontWeight = FontWeights.Bold,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                Margin = new Thickness(0, -4, 0, 0),
            },
            Width = 44,
            Height = 44,
        };
        btn.Click += (_, _) => BackRequested?.Invoke();
        grid.Children.Add(btn);

        if (BackBadgeCount > 0)
        {
            var badge = new Border
            {
                Background = (System.Windows.Media.Brush)FindResource("Accent"),
                CornerRadius = new CornerRadius(9),
                Padding = new Thickness(5, 1, 5, 1),
                MinWidth = 18,
                Height = 18,
                HorizontalAlignment = HorizontalAlignment.Right,
                VerticalAlignment = VerticalAlignment.Top,
                Margin = new Thickness(0, 2, 2, 0),
                Child = new TextBlock
                {
                    Text = BackBadgeCount > 99 ? "99+" : BackBadgeCount.ToString(),
                    Foreground = System.Windows.Media.Brushes.White,
                    FontSize = 10,
                    FontWeight = FontWeights.Bold,
                    HorizontalAlignment = HorizontalAlignment.Center,
                    VerticalAlignment = VerticalAlignment.Center,
                },
            };
            grid.Children.Add(badge);
        }

        return grid;
    }
}
