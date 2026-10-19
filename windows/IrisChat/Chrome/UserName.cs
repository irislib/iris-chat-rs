using System;
using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using IrisChat.Bindings;

namespace IrisChat.Chrome;

/// Current-account name selected only by identity. Group titles remain ordinary text.
public sealed class UserName : TextBlock
{
    public static readonly DependencyProperty OwnerPubkeyHexProperty =
        DependencyProperty.Register(nameof(OwnerPubkeyHex), typeof(string), typeof(UserName),
            new PropertyMetadata("", (d, _) => ((UserName)d).Refresh()));
    public string OwnerPubkeyHex
    {
        get => (string)GetValue(OwnerPubkeyHexProperty);
        set => SetValue(OwnerPubkeyHexProperty, value);
    }
    private AppManager? _manager;
    private (string?, ulong, string)? _key;
    public UserName()
    {
        Loaded += (_, _) => {
            _manager = Application.Current is App app ? app.Manager : null;
            if (_manager != null) _manager.PropertyChanged += Changed;
            Refresh();
        };
        Unloaded += (_, _) => {
            if (_manager != null) _manager.PropertyChanged -= Changed;
            _manager = null; _key = null;
        };
    }
    private void Changed(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(AppManager.ProfileRevision) or nameof(AppManager.Account)) Refresh();
    }
    private void Refresh()
    {
        var key = (_manager?.Account?.publicKeyHex, _manager?.ProfileRevision ?? 0, OwnerPubkeyHex);
        if (_key == key) return;
        _key = key;
        var name = _manager?.Account != null ? _manager.UserProfile(OwnerPubkeyHex).name
            : Native.PresentPersonName("", OwnerPubkeyHex, null);
        Text = name.name;
        FontStyle = PersonNames.Style(name);
    }
}
