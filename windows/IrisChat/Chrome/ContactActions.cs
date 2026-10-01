using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace IrisChat.Chrome;

public sealed class ContactActions : StackPanel
{
    private string _chatId = "";
    private bool _showProfileActions;
    private AppManager? _manager;
    private object? _rendered;

    public string ChatId
    {
        get => _chatId;
        set
        {
            if (_chatId == value) return;
            _chatId = value;
            if (IsLoaded) Refresh();
        }
    }

    public bool ShowProfileActions
    {
        get => _showProfileActions;
        set
        {
            if (_showProfileActions == value) return;
            _showProfileActions = value;
            if (IsLoaded) Refresh();
        }
    }

    public ContactActions()
    {
        Loaded += (_, _) =>
        {
            _manager = App.CurrentManager;
            _manager.PropertyChanged += Changed;
            Refresh();
        };
        Unloaded += (_, _) =>
        {
            if (_manager != null) _manager.PropertyChanged -= Changed;
            _manager = null;
        };
    }

    private void Changed(object? sender, PropertyChangedEventArgs e) => Refresh();

    private void Refresh()
    {
        var chat = _manager?.CurrentChat;
        var contact = chat?.chatId == ChatId ? chat.contactIdentity : null;
        var key = (ChatId, ShowProfileActions, contact?.isFollowing, contact?.canFollow,
            contact?.updatingFollow, contact?.isFavorite, contact?.pendingName,
            contact?.firstSeenName, contact?.savedName);
        if (object.Equals(_rendered, key)) return;
        _rendered = key;
        Children.Clear();
        if (contact == null) return;
        var owner = ChatId;
        if (ShowProfileActions)
        {
            var actions = new WrapPanel();
            var follow = Button(contact.updatingFollow ? "Saving…" : contact.isFollowing ? "Unfollow (public)" : "Follow (public)");
            follow.IsEnabled = contact.canFollow && !contact.updatingFollow;
            follow.ToolTip = contact.canFollow ? "Visible to everyone" : "Use your main device to change public follows";
            follow.Click += (_, _) => _manager?.SetPublicFollow(owner, !contact.isFollowing);
            actions.Children.Add(follow);
            var favorite = Button(contact.isFavorite ? "★ Favorited" : "☆ Favorite");
            favorite.ToolTip = "Only you can see this";
            favorite.Click += (_, _) => _manager?.SetContactFavorite(owner, !contact.isFavorite);
            actions.Children.Add(favorite);
            Children.Add(actions);
            Children.Add(Label("Favorites are only visible to you"));
            if (contact.firstSeenName is { } first && first != contact.savedName)
                Children.Add(Label($"First known as {first}"));
        }
        if (contact.pendingName is { } proposed)
        {
            Children.Add(Label($"New profile name: {proposed}"));
            var approve = Button("Use new name");
            approve.Click += (_, _) => _manager?.ApproveContactName(owner, proposed);
            Children.Add(approve);
        }
    }

    private static Button Button(string text) => new() { Content = text, HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(0, 6, 8, 6) };
    private static TextBlock Label(string text) => new() { Text = text, TextWrapping = TextWrapping.Wrap, Foreground = (Brush)Application.Current.FindResource("TextMuted"), Margin = new Thickness(0, 4, 0, 4) };
}
