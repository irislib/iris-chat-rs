using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
using System.Windows.Media;
using System.Windows.Threading;

namespace IrisChat;

/// Owns one decrypted attachment and its in-app player on the UI dispatcher.
public sealed class AudioMessagePlayback : IDisposable
{
    private static readonly Dictionary<string, AudioMessagePlayback> Players = new();
    private static AudioMessagePlayback? _active;
    private static bool _callActive;
    private readonly string _key;
    private readonly string _filename;
    private readonly Func<Task<byte[]?>> _download;
    private readonly Dispatcher _dispatcher = Dispatcher.CurrentDispatcher;
    private readonly DispatcherTimer _timer = new() { Interval = TimeSpan.FromMilliseconds(100) };
    private MediaPlayer? _player;
    private string? _directory;
    private int _generation;
    private int _leases;
    private bool _disposed;
    private bool _wantsToPlay;
    private bool _ready;

    public bool IsPlaying { get; private set; }
    public bool IsLoading { get; private set; }
    public double Elapsed { get; private set; }
    public double Duration { get; private set; }
    public float[] Peaks { get; private set; } = Array.Empty<float>();
    public double Rate { get; private set; } = 1;
    public string? Error { get; private set; }
    public event Action? Changed;

    private AudioMessagePlayback(string key, string filename, Func<Task<byte[]?>> download)
    {
        _key = key;
        _filename = filename;
        _download = download;
        _timer.Tick += (_, _) => { if (_player != null) Elapsed = _player.Position.TotalSeconds; Changed?.Invoke(); };
    }

    public static AudioMessagePlayback Attach(string key, string filename, Func<Task<byte[]?>> download)
    {
        if (!Players.TryGetValue(key, out var playback))
            Players[key] = playback = new AudioMessagePlayback(key, filename, download);
        playback._leases++;
        return playback;
    }

    public void Detach()
    {
        _leases--;
        // A delivery receipt rebuilds message rows. Let the replacement row
        // attach before deciding that the user has left this conversation.
        _dispatcher.BeginInvoke(DispatcherPriority.ContextIdle, new Action(() =>
        {
            if (_leases == 0) Dispose();
        }));
    }

    public static bool IsAudio(bool isAudio, string filename) => isAudio ||
        Path.GetExtension(filename).ToLowerInvariant() is ".aac" or ".aiff" or ".flac" or ".m4a" or ".mp3" or ".ogg" or ".opus" or ".wav" or ".wma";

    public static void SetCallActive(bool active)
    {
        _callActive = active;
        if (active) _active?.Pause();
    }

    public async Task ToggleAsync()
    {
        if (IsPlaying || IsLoading) Pause();
        else await PlayAsync();
    }

    public async Task PlayAsync()
    {
        if (_disposed) return;
        if (_callActive) { Error = "Finish your call to play audio."; Changed?.Invoke(); return; }
        if (_active != this) _active?.Pause();
        _active = this;
        _wantsToPlay = true;
        Error = null;
        if (_ready) { Start(); return; }
        if (IsLoading) return;
        IsLoading = true;
        Changed?.Invoke();
        var generation = ++_generation;
        string? directory = null;
        try
        {
            var data = await _download();
            if (_disposed || generation != _generation) return;
            if (data == null || data.Length == 0) throw new IOException("Audio unavailable");
            directory = Path.Combine(Path.GetTempPath(), "iris-audio-" + Guid.NewGuid().ToString("N"));
            var suffix = Path.GetExtension(_filename);
            var path = Path.Combine(directory, "audio" + (IsAudio(false, _filename) ? suffix : ".audio"));
            var peaks = await Task.Run(() => {
                Directory.CreateDirectory(directory); File.WriteAllBytes(path, data);
                return AudioWaveform.Decode(path);
            });
            if (_disposed || generation != _generation) { Remove(directory); return; }
            ClosePlayer();
            Peaks = peaks;
            _directory = directory;
            var player = new MediaPlayer();
            _player = player;
            player.MediaOpened += (_, _) =>
            {
                if (_player != player) return;
                if (!player.NaturalDuration.HasTimeSpan) { Fail(); return; }
                Duration = player.NaturalDuration.TimeSpan.TotalSeconds;
                _ready = true;
                IsLoading = false;
                if (_wantsToPlay && !_callActive) Start();
                else Changed?.Invoke();
            };
            player.MediaEnded += (_, _) =>
            {
                if (_player != player) return;
                Pause();
                Elapsed = Duration;
                Changed?.Invoke();
            };
            player.MediaFailed += (_, error) =>
            {
                System.Diagnostics.Trace.TraceError("Audio playback failed: {0}", error.ErrorException);
                if (_player == player) Fail();
            };
            player.Open(new Uri(path));
        }
        catch
        {
            if (directory != _directory) Remove(directory);
            if (!_disposed && generation == _generation) Fail();
        }
    }

    private void Start()
    {
        if (_player == null || !_ready || _callActive || !_wantsToPlay) return;
        if (Elapsed >= Duration - 0.05) Seek(0);
        _player.SpeedRatio = Rate;
        _player.Play();
        IsLoading = false;
        IsPlaying = true;
        _timer.Start();
        Changed?.Invoke();
    }

    public void Pause()
    {
        _generation++;
        _wantsToPlay = false;
        if (IsLoading && !_ready) ClosePlayer();
        else _player?.Pause();
        _timer.Stop();
        IsPlaying = false;
        IsLoading = false;
        Changed?.Invoke();
    }

    public void Seek(double seconds)
    {
        if (_player == null || !_ready || !double.IsFinite(seconds)) return;
        Elapsed = Math.Clamp(seconds, 0, Duration);
        _player.Position = TimeSpan.FromSeconds(Elapsed);
        Changed?.Invoke();
    }

    public void CycleRate()
    {
        Rate = Rate switch { 1 => 1.5, 1.5 => 2, 2 => 0.5, _ => 1 };
        if (_player != null) _player.SpeedRatio = Rate;
        Changed?.Invoke();
    }

    private void Fail()
    {
        Pause();
        ClosePlayer();
        Error = "Couldn't play audio. Try again.";
        Changed?.Invoke();
    }

    private void ClosePlayer()
    {
        _player?.Close();
        _player = null;
        _ready = false;
        Remove(_directory);
        _directory = null;
    }

    private static void Remove(string? directory)
    {
        if (directory == null) return;
        try { Directory.Delete(directory, true); } catch (IOException) { } catch (UnauthorizedAccessException) { }
    }

    public void Dispose()
    {
        if (_disposed) return;
        _disposed = true;
        Pause();
        ClosePlayer();
        Players.Remove(_key);
        if (_active == this) _active = null;
    }
}
