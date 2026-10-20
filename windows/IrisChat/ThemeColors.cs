using System.Collections.Generic;
using System.ComponentModel;
using System.Windows.Data;
using System.Windows.Media;

namespace IrisChat;

// A shared binding source also updates brush clones created by sealed styles.
// Inheritance-context resource lookup on a shared Freezable cannot do that
// reliably, and replacing a brush leaves static-resource consumers behind.
public sealed class ThemeColors : Dictionary<string, Color>, INotifyPropertyChanged
{
    public event PropertyChangedEventHandler? PropertyChanged;

    public void Refresh() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(Binding.IndexerName));
}
