using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace IrisChat;

/// Persists the owner/device secret bundle in the Windows Credential Manager.
/// Mirrors KeychainSecretStore (iOS/macOS) and the Android Keystore-backed
/// store: secrets stay outside the Rust plaintext app snapshot.
public sealed class WindowsCredentialStore
{
    public sealed record StoredAccountBundle(
        string? OwnerNsec,
        string OwnerPubkeyHex,
        string DeviceNsec
    );

    private readonly string _targetName;

    public WindowsCredentialStore(string targetName = "to.iris.chat/stored-account-bundle")
    {
        _targetName = targetName;
    }

    public StoredAccountBundle? Load()
    {
        if (!CredRead(_targetName, CRED_TYPE_GENERIC, 0, out var credPtr))
        {
            return null;
        }

        try
        {
            var cred = Marshal.PtrToStructure<CREDENTIAL>(credPtr);
            if (cred.CredentialBlob == IntPtr.Zero || cred.CredentialBlobSize == 0)
            {
                return null;
            }

            var data = new byte[cred.CredentialBlobSize];
            Marshal.Copy(cred.CredentialBlob, data, 0, (int)cred.CredentialBlobSize);
            var json = Encoding.UTF8.GetString(data);
            return JsonSerializer.Deserialize<StoredAccountBundle>(json);
        }
        catch
        {
            return null;
        }
        finally
        {
            CredFree(credPtr);
        }
    }

    public void Save(StoredAccountBundle bundle)
    {
        var json = JsonSerializer.Serialize(bundle);
        var blob = Encoding.UTF8.GetBytes(json);
        var blobPtr = Marshal.AllocHGlobal(blob.Length);
        try
        {
            Marshal.Copy(blob, 0, blobPtr, blob.Length);
            var cred = new CREDENTIAL
            {
                Type = CRED_TYPE_GENERIC,
                TargetName = _targetName,
                CredentialBlobSize = (uint)blob.Length,
                CredentialBlob = blobPtr,
                Persist = CRED_PERSIST_LOCAL_MACHINE,
                UserName = Environment.UserName,
            };
            if (!CredWrite(ref cred, 0))
            {
                throw new InvalidOperationException(
                    $"CredWrite failed: {Marshal.GetLastWin32Error()}"
                );
            }
        }
        finally
        {
            Marshal.FreeHGlobal(blobPtr);
        }
    }

    public bool Clear()
    {
        var deleted = CredDelete(_targetName, CRED_TYPE_GENERIC, 0);
        var error = deleted ? 0 : Marshal.GetLastWin32Error();
        if (!deleted && error != ERROR_NOT_FOUND) return false;
        return Load() == null;
    }

    private const uint CRED_TYPE_GENERIC = 1;
    private const uint CRED_PERSIST_LOCAL_MACHINE = 2;
    private const int ERROR_NOT_FOUND = 1168;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct CREDENTIAL
    {
        public uint Flags;
        public uint Type;
        public string TargetName;
        public string? Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize;
        public IntPtr CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public IntPtr Attributes;
        public string? TargetAlias;
        public string UserName;
    }

    [DllImport("Advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "CredReadW")]
    private static extern bool CredRead(string target, uint type, uint reservedFlag, out IntPtr credentialPtr);

    [DllImport("Advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "CredWriteW")]
    private static extern bool CredWrite(ref CREDENTIAL credential, uint flags);

    [DllImport("Advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode, EntryPoint = "CredDeleteW")]
    private static extern bool CredDelete(string target, uint type, uint flags);

    [DllImport("Advapi32.dll")]
    private static extern void CredFree(IntPtr buffer);
}
