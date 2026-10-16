using System;
using System.Collections.Concurrent;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

// Hold the real replication ACK while local sending and draft editing continue.
internal sealed class DelayedAttachmentUpload : IDisposable
{
    private readonly TcpListener _listener = new(IPAddress.Loopback, 0);
    private readonly CancellationTokenSource _stop = new();
    private readonly TaskCompletionSource _release = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private readonly string _directory = Path.Combine(Path.GetTempPath(), "iris-draft-upload-" + Guid.NewGuid());
    private readonly string? _previousConfig = Environment.GetEnvironmentVariable("HTREE_CONFIG_DIR");
    private readonly ConcurrentDictionary<string, byte[]> _blobs = new();
    private readonly ConcurrentDictionary<string, byte> _servedUrls = new();
    private readonly Task _server;
    private readonly string _url;
    private int _requests;

    public string SourcePath { get; }
    public bool RequestReceived
    {
        get { ThrowIfFailed(); return Volatile.Read(ref _requests) > 0; }
    }

    public bool ServedCiphertext(string url)
    {
        ThrowIfFailed();
        return _servedUrls.ContainsKey(url);
    }

    public DelayedAttachmentUpload()
    {
        Directory.CreateDirectory(_directory);
        SourcePath = Path.Combine(_directory, "upload-draft.txt");
        File.WriteAllText(SourcePath, "Isolated draft upload " + Guid.NewGuid());
        _listener.Start();
        var port = ((IPEndPoint)_listener.LocalEndpoint).Port;
        _url = $"http://127.0.0.1:{port}";
        File.WriteAllText(Path.Combine(_directory, "config.toml"),
            $"[blossom]\nservers = []\nread_servers = []\nwrite_servers = [\"{_url}\"]\n");
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
                int bodyStart = 0;
                string[] headers = Array.Empty<string>();
                while (expectedLength == null || request.Length < expectedLength)
                {
                    var count = await stream.ReadAsync(buffer.AsMemory(), _stop.Token).ConfigureAwait(false);
                    if (count == 0) throw new IOException("Upload ended before its body was received");
                    request.Write(buffer, 0, count);
                    if (request.Length > 1024 * 1024) throw new IOException("Draft fixture upload is unexpectedly large");
                    if (expectedLength != null) continue;
                    var text = Encoding.ASCII.GetString(request.ToArray());
                    var headerEnd = text.IndexOf("\r\n\r\n", StringComparison.Ordinal);
                    if (headerEnd < 0) continue;
                    headers = text[..headerEnd].Split("\r\n");
                    var contentLength = headers.SingleOrDefault(line => line.StartsWith("Content-Length:", StringComparison.OrdinalIgnoreCase));
                    var length = contentLength == null ? 0 : int.Parse(contentLength[(contentLength.IndexOf(':') + 1)..].Trim());
                    if (length < 0 || length > 1024 * 1024) throw new IOException("Invalid draft fixture body length");
                    bodyStart = headerEnd + 4;
                    expectedLength = bodyStart + length;
                }
                var route = headers[0].Split(' ');
                if (route[0] == "PUT" && route[1] == "/upload")
                {
                    var uploadBody = request.ToArray()[bodyStart..expectedLength!.Value];
                    var hash = Convert.ToHexString(SHA256.HashData(uploadBody)).ToLowerInvariant();
                    var declared = headers.Single(line => line.StartsWith("x-sha-256:", StringComparison.OrdinalIgnoreCase));
                    if (declared[(declared.IndexOf(':') + 1)..].Trim() != hash)
                        throw new IOException("Upload checksum did not match its declared ciphertext hash");
                    if (_blobs.Count >= 8 && !_blobs.ContainsKey(hash)) throw new IOException("Too many draft fixture blocks");
                    _blobs[hash] = uploadBody;
                    Interlocked.Increment(ref _requests);
                    await _release.Task.WaitAsync(_stop.Token).ConfigureAwait(false);
                    var descriptor = Encoding.UTF8.GetBytes($"{{\"sha256\":\"{hash}\"}}");
                    await Respond(stream, "201 Created", descriptor).ConfigureAwait(false);
                }
                else if (route[0] == "GET" && route[1].StartsWith('/') && route[1].EndsWith(".bin", StringComparison.Ordinal)
                    && _blobs.TryGetValue(route[1][1..^4], out var cachedBody))
                {
                    await Respond(stream, "200 OK", cachedBody).ConfigureAwait(false);
                    _servedUrls.TryAdd(_url + route[1], 0);
                }
                else throw new IOException("Unexpected draft fixture request: " + headers[0]);
            }
        }
        catch (Exception) when (_stop.IsCancellationRequested) { }
    }

    private async Task Respond(NetworkStream stream, string status, byte[] body)
    {
        var header = Encoding.ASCII.GetBytes($"HTTP/1.1 {status}\r\nContent-Length: {body.Length}\r\nConnection: close\r\n\r\n");
        await stream.WriteAsync(header.AsMemory(), _stop.Token).ConfigureAwait(false);
        await stream.WriteAsync(body.AsMemory(), _stop.Token).ConfigureAwait(false);
    }

    private void ThrowIfFailed()
    {
        if (_server.IsFaulted) _server.GetAwaiter().GetResult();
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
