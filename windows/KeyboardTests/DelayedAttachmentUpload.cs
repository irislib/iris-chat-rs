using System;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

// Exercise the real upload without depending on a public server or a timing race.
internal sealed class DelayedAttachmentUpload : IDisposable
{
    private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
    private readonly CancellationTokenSource _stop = new();
    private readonly TaskCompletionSource _release = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "iris-draft-upload-" + Guid.NewGuid());
    private readonly string? _previousConfig = Environment.GetEnvironmentVariable("HTREE_CONFIG_DIR");
    private readonly Task _server;
    private int _requests;

    public string SourcePath { get; }
    public bool RequestReceived => Volatile.Read(ref _requests) > 0;

    public DelayedAttachmentUpload()
    {
        Directory.CreateDirectory(_directory);
        SourcePath = Path.Combine(_directory, "upload-draft.txt");
        File.WriteAllText(SourcePath, "Isolated draft upload " + Guid.NewGuid());
        _listener.Start();
        var port = ((IPEndPoint)_listener.LocalEndpoint).Port;
        File.WriteAllText(Path.Combine(_directory, "config.toml"),
            $"[blossom]\nservers = []\nread_servers = []\nwrite_servers = [\"http://127.0.0.1:{port}\"]\n");
        Environment.SetEnvironmentVariable("HTREE_CONFIG_DIR", _directory);
        _server = Serve();
    }

    public void Complete() => _release.TrySetResult();

    private async Task Serve()
    {
        try
        {
            while (!_stop.IsCancellationRequested)
            {
                using var client = await _listener.AcceptTcpClientAsync(_stop.Token).ConfigureAwait(false);
                using var stream = client.GetStream();
                using var request = new MemoryStream();
                var buffer = new byte[4096];
                int? expectedLength = null;
                while (expectedLength == null || request.Length < expectedLength)
                {
                    var count = await stream.ReadAsync(buffer.AsMemory(), _stop.Token).ConfigureAwait(false);
                    if (count == 0) throw new IOException("Upload ended before its body was received");
                    request.Write(buffer, 0, count);
                    if (request.Length > 1024 * 1024) throw new IOException("Draft fixture upload is unexpectedly large");
                    if (expectedLength != null) continue;
                    var headers = Encoding.ASCII.GetString(request.ToArray());
                    var headerEnd = headers.IndexOf("\r\n\r\n", StringComparison.Ordinal);
                    if (headerEnd < 0) continue;
                    var contentLength = headers[..headerEnd].Split("\r\n")
                        .Single(line => line.StartsWith("Content-Length:", StringComparison.OrdinalIgnoreCase));
                    expectedLength = headerEnd + 4 + int.Parse(contentLength[(contentLength.IndexOf(':') + 1)..].Trim());
                }
                Interlocked.Increment(ref _requests);
                await _release.Task.WaitAsync(_stop.Token).ConfigureAwait(false);
                var response = Encoding.ASCII.GetBytes("HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                await stream.WriteAsync(response.AsMemory(), _stop.Token).ConfigureAwait(false);
            }
        }
        catch (Exception) when (_stop.IsCancellationRequested) { }
    }

    public void Dispose()
    {
        Complete();
        _stop.Cancel();
        _listener.Stop();
        try { _server.GetAwaiter().GetResult(); }
        finally
        {
            Environment.SetEnvironmentVariable("HTREE_CONFIG_DIR", _previousConfig);
            _stop.Dispose();
            Directory.Delete(_directory, recursive: true);
        }
    }
}
