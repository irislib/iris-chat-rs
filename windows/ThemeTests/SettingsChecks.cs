using System;
using System.IO;
using System.Linq;
using System.Windows;
using System.Windows.Automation;
using System.Windows.Automation.Peers;
using System.Windows.Automation.Provider;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using IrisChat;
using IrisChat.Bindings;
using IrisChat.Chrome;
using IrisChat.Views;

internal static class SettingsChecks
{
    internal static void Run(MainWindow main, AppManager manager, string output)
    {
        var settings = Program.Descendants<SettingsView>(main).Single();
        var navigation = (ListBox)settings.FindName("SettingsNavigation");
        var content = (ScrollViewer)settings.FindName("PageScroll");
        var title = (TextBlock)settings.FindName("PageTitle");
        var menus = navigation.Items.Cast<ListBoxItem>().ToArray();
        Program.Check(menus.Length == 13, "Settings includes every category");
        Program.Check(!Program.Descendants<DesktopShell>(main).Any(), "Settings fills the window without a second navigation pane");
        main.Activate();
        navigation.SelectedIndex = 0;
        menus[0].Focus();
        Press(menus[0], Key.Down);
        Program.Check(navigation.SelectedIndex == 1 && title.Text == "General", "Down selects and opens the next settings category");
        Press(menus[1], Key.Up);
        Program.Check(navigation.SelectedIndex == 0 && title.Text == "Profile", "Up selects and opens the previous category");
        menus[0].MoveFocus(new TraversalRequest(FocusNavigationDirection.Next));
        Program.Check(content.IsKeyboardFocusWithin, "Tab leaves category navigation and enters the current page");
        // Edits survive navigation and layout changes until explicitly saved.
        var name = (TextBox)settings.FindName("ProfileNameInput");
        name.Focus(); name.Text = "Unsaved profile name";
        var about = (TextBox)settings.FindName("ProfileAboutInput");
        about.Text = "Unsaved about text";
        foreach (var light in new[] { true, false })
        {
            Program.SetPalette(light);
            var theme = light ? "light" : "dark";
            foreach (var (width, height, scale) in new[] { (1100, 720, 1.0), (720, 540, 1.0), (720, 540, 1.5) })
            {
                main.Width = width; main.Height = height;
                settings.LayoutTransform = new ScaleTransform(scale, scale);
                Program.Pump();
                foreach (var menu in menus)
                {
                    menu.IsSelected = true; Program.Pump();
                    var label = AutomationProperties.GetName(menu);
                    Program.Check(title.Text == label, "The heading follows the selected category");
                    var page = (FrameworkElement)settings.FindName(menu.Tag + "Page");
                    Program.Check(page.Visibility == Visibility.Visible, label + " content is visible");
                    var indicator = (Border)menu.Template.FindName("SelectedIndicator", menu);
                    Program.Check(indicator.IsVisible, label + " has a persistent selection indicator");
                    Program.Check(content.ScrollableWidth < 1, label + " requires no horizontal scrolling");
                    CheckHorizontalBounds(page, content, label);
                    Contrast.CheckTree(main, $"{theme} {width}px {scale}x {label}");
                }
                if (width == 720)
                {
                    var scroll = Program.Descendants<ScrollViewer>(navigation).Single();
                    menus[^1].Focus(); menus[^1].BringIntoView(); Program.Pump();
                    Program.Check(scroll.VerticalOffset > 0, "The final category can scroll into view");
                    var bar = Program.Descendants<ScrollBar>(scroll).Single();
                    ScrollBar.PageUpCommand.Execute(null, bar); Program.Pump();
                    Program.Check(scroll.VerticalOffset < scroll.ScrollableHeight, "The navigation scrollbar pages upward");
                }
                Select("Messaging");
                Program.Save(main, Path.Combine(output, $"{theme}-settings-{width}-{scale:0.0}x.png"));
                if (width == 1100)
                {
                    Select("Profile"); Program.Save(main, Path.Combine(output, theme + "-settings-profile.png"));
                    Select("General"); Program.Save(main, Path.Combine(output, theme + "-settings-general.png"));
                }
            }
        }
        Program.Check(name.Text == "Unsaved profile name" && about.Text == "Unsaved about text",
            "Unsaved profile text survives category and size changes");
        settings.LayoutTransform = Transform.Identity;
        main.Width = 1100; main.Height = 720;
        Select("Notifications");
        var toggle = (CheckBox)settings.FindName("NotificationsToggle");
        var original = manager.Preferences.desktopNotificationsEnabled;
        var peer = new CheckBoxAutomationPeer(toggle);
        var pattern = (IToggleProvider)peer.GetPattern(PatternInterface.Toggle);
        pattern.Toggle();
        Program.PumpUntil(() => manager.Preferences.desktopNotificationsEnabled == !original);
        Program.Check(manager.Preferences.desktopNotificationsEnabled == !original, "The settings switch persists its new value");
        pattern.Toggle();
        Program.PumpUntil(() => manager.Preferences.desktopNotificationsEnabled == original);
        Program.Check(manager.Preferences.desktopNotificationsEnabled == original, "The settings switch restores its original value");
        Program.Check(name.Text == "Unsaved profile name" && about.Text == "Unsaved about text",
            "Unrelated preference updates preserve both profile edits");
        Select("Profile");
        ((Button)settings.FindName("SaveProfileButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
        Program.PumpUntil(() => manager.Account?.displayName == "Unsaved profile name" && manager.Account?.about == "Unsaved about text");
        Program.Check(name.Text == manager.Account!.displayName, "Save commits the retained profile edits");
        ((Button)settings.FindName("BackButton")).RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
        Program.PumpUntil(() => manager.ActiveScreen is Screen.ChatList && Program.Descendants<DesktopShell>(main).Any());
        Program.Check(manager.ActiveScreen is Screen.ChatList && Program.Descendants<DesktopShell>(main).Any(),
            "Back restores the chat list and its normal navigation: " + manager.ActiveScreen.GetType().Name);
        Console.WriteLine("PASS: full-window settings, arrow/Tab navigation, all 13 pages at 1100/720px and 150% zoom, persistent edits, switch actions and back to chats");

        void Select(string tag) { menus.Single(item => Equals(item.Tag, tag)).IsSelected = true; Program.Pump(); }
    }

    private static void CheckHorizontalBounds(FrameworkElement page, ScrollViewer viewport, string context)
    {
        foreach (var element in Program.Descendants<FrameworkElement>(page)
            .Where(element => element.IsVisible && element is Button or CheckBox or TextBox))
        {
            var bounds = element.TransformToAncestor(viewport).TransformBounds(new Rect(element.RenderSize));
            Program.Check(bounds.Left >= -1 && bounds.Right <= viewport.ViewportWidth + 1,
                $"{context}: {element.Name}/{element.GetType().Name} fits horizontally ({bounds.Left:F0}..{bounds.Right:F0} within {viewport.ViewportWidth:F0})");
        }
    }

    private static void Press(UIElement target, Key key)
    {
        target.RaiseEvent(new KeyEventArgs(Keyboard.PrimaryDevice, PresentationSource.FromVisual(target), 0, key)
            { RoutedEvent = Keyboard.KeyDownEvent });
        Program.Pump();
    }
}
