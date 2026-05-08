using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Pipes;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace IrisChat;

public sealed class SingleInstanceService : IDisposable
{
    private const string MutexName = @"Local\IrisChat.Windows";
    private const string PipeName = "IrisChat.Windows.Launch";

    private readonly Mutex _mutex;
    private readonly CancellationTokenSource _shutdown = new();
    private Task? _listener;

    private SingleInstanceService(Mutex mutex)
    {
        _mutex = mutex;
    }

    public static SingleInstanceService? ClaimOrSignal(string[] args)
    {
        var mutex = new Mutex(initiallyOwned: true, MutexName, out var ownsMutex);
        if (ownsMutex)
        {
            return new SingleInstanceService(mutex);
        }

        mutex.Dispose();
        SignalPrimary(args);
        return null;
    }

    public void Start(Action<IReadOnlyList<string>> onLaunchArgs)
    {
        _listener = Task.Run(() => ListenAsync(onLaunchArgs, _shutdown.Token));
    }

    public void Dispose()
    {
        _shutdown.Cancel();
        try
        {
            _listener?.Wait(TimeSpan.FromMilliseconds(300));
        }
        catch
        {
            // Shutdown should not be blocked by a pipe cancellation race.
        }
        _shutdown.Dispose();
        _mutex.ReleaseMutex();
        _mutex.Dispose();
    }

    private static void SignalPrimary(string[] args)
    {
        for (var attempt = 0; attempt < 8; attempt++)
        {
            try
            {
                using var client = new NamedPipeClientStream(".", PipeName, PipeDirection.Out);
                client.Connect(300);
                using var writer = new StreamWriter(client, new UTF8Encoding(false));
                writer.Write(JsonSerializer.Serialize(args));
                return;
            }
            catch
            {
                Thread.Sleep(100);
            }
        }
    }

    private static async Task ListenAsync(Action<IReadOnlyList<string>> onLaunchArgs, CancellationToken token)
    {
        while (!token.IsCancellationRequested)
        {
            try
            {
                using var server = new NamedPipeServerStream(
                    PipeName,
                    PipeDirection.In,
                    maxNumberOfServerInstances: 1,
                    PipeTransmissionMode.Byte,
                    PipeOptions.Asynchronous);
                await server.WaitForConnectionAsync(token);
                using var reader = new StreamReader(server, Encoding.UTF8);
                var payload = await reader.ReadToEndAsync(token);
                var args = JsonSerializer.Deserialize<string[]>(payload) ?? [];
                onLaunchArgs(args);
            }
            catch (OperationCanceledException)
            {
                break;
            }
            catch
            {
                if (!token.IsCancellationRequested)
                {
                    await Task.Delay(200, token);
                }
            }
        }
    }
}
