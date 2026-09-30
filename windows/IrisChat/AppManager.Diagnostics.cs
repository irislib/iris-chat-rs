using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace IrisChat;

public sealed partial class AppManager
{
    private void LogFfiFailure(string category, Exception error, string detail = "")
    {
        var logDetail = string.IsNullOrWhiteSpace(detail)
            ? ErrorSummary(error)
            : $"{detail}: {ErrorSummary(error)}";
        AppendClientDebugLog(category, logDetail);
        var message = $"Iris Chat FFI call failed ({category}): {logDetail}\n{error}";
        Trace.TraceError(message);
        Debug.WriteLine(message);
    }

    private void AppendClientDebugLog(string category, string detail)
    {
        var truncated = detail.Length <= MaxClientDebugLogDetailChars
            ? detail
            : detail[..MaxClientDebugLogDetailChars];
        lock (_clientDebugLogLock)
        {
            _clientDebugLog.Add(new ClientDebugLogEntry(
                DateTimeOffset.UtcNow.ToUnixTimeSeconds(),
                category,
                truncated
            ));
            if (_clientDebugLog.Count > MaxClientDebugLogEntries)
            {
                _clientDebugLog.RemoveRange(0, _clientDebugLog.Count - MaxClientDebugLogEntries);
            }
        }
    }

    private List<ClientDebugLogEntry> SnapshotClientDebugLog()
    {
        lock (_clientDebugLogLock)
        {
            return _clientDebugLog.ToList();
        }
    }

    private string SupportBundleJsonWithClientLog(string rustJson)
    {
        var clientLog = SnapshotClientDebugLog();
        if (clientLog.Count == 0) return rustJson;
        try
        {
            var root = JsonNode.Parse(string.IsNullOrWhiteSpace(rustJson) ? "{}" : rustJson) as JsonObject
                       ?? new JsonObject();
            var array = new JsonArray();
            foreach (var entry in clientLog)
            {
                array.Add(new JsonObject
                {
                    ["timestamp_secs"] = entry.TimestampSecs,
                    ["category"] = entry.Category,
                    ["detail"] = entry.Detail
                });
            }
            root["client_log"] = array;
            return root.ToJsonString(new JsonSerializerOptions { WriteIndented = true });
        }
        catch
        {
            return rustJson;
        }
    }

    private static string ErrorSummary(Exception error)
    {
        var message = error.Message;
        return string.IsNullOrWhiteSpace(message)
            ? error.GetType().Name
            : $"{error.GetType().Name}: {message}";
    }
}
