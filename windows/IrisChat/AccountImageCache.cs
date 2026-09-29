using System.Collections.Generic;

namespace IrisChat;

/// Retains images only for the current local account session.
internal sealed class AccountImageCache<T> where T : class
{
    private readonly object _gate = new();
    private readonly Dictionary<string, T> _images = new();
    private long _generation;

    public long Generation { get { lock (_gate) return _generation; } }
    public bool IsCurrent(long generation) { lock (_gate) return generation == _generation; }
    public bool TryGetValue(string key, out T? image) { lock (_gate) return _images.TryGetValue(key, out image); }
    public bool TryStore(string key, T image, long generation)
    {
        lock (_gate)
        {
            if (generation != _generation) return false;
            _images[key] = image;
            return true;
        }
    }
    public void Clear()
    {
        lock (_gate) { _generation++; _images.Clear(); }
    }
}
