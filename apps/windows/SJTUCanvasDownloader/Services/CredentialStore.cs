using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;

namespace CanvasDownloader.Services;

/// <summary>
/// The key that protects the saved Canvas login, in Windows Credential Manager
/// ("Windows 凭据" → "SJTU Canvas Downloader/session-key"), scoped to the
/// current user. The engine encrypts the login cookies with it and only ever
/// receives it in memory.
/// </summary>
public static partial class CredentialStore
{
    private const string Target = "SJTU Canvas Downloader/session-key";
    private const uint CRED_TYPE_GENERIC = 1;
    private const uint CRED_PERSIST_LOCAL_MACHINE = 2;

    /// <summary>The stored key, or a new one saved now; null if Credential Manager is unavailable.</summary>
    public static string? GetOrCreateSessionKey()
    {
        try
        {
            if (Read() is { Length: > 0 } existing)
            {
                return existing;
            }
            var key = Convert.ToBase64String(RandomNumberGenerator.GetBytes(32));
            Write(key);
            return Read() == key ? key : null;
        }
        catch (Exception error)
        {
            AppLog.Error("credential store", error);
            return null;
        }
    }

    private static string? Read()
    {
        if (!CredReadW(Target, CRED_TYPE_GENERIC, 0, out var pointer))
        {
            return null;
        }

        try
        {
            var credential = Marshal.PtrToStructure<CREDENTIAL>(pointer);
            if (credential.CredentialBlobSize == 0 || credential.CredentialBlob == IntPtr.Zero)
            {
                return null;
            }
            var bytes = new byte[credential.CredentialBlobSize];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            try
            {
                return Encoding.Unicode.GetString(bytes);
            }
            finally
            {
                Array.Clear(bytes);
            }
        }
        finally
        {
            CredFree(pointer);
        }
    }

    private static void Write(string secret)
    {
        var blob = Encoding.Unicode.GetBytes(secret);
        var blobPointer = Marshal.AllocHGlobal(blob.Length);
        var target = Marshal.StringToHGlobalUni(Target);
        var user = Marshal.StringToHGlobalUni(Environment.UserName);
        try
        {
            Marshal.Copy(blob, 0, blobPointer, blob.Length);
            var credential = new CREDENTIAL
            {
                Type = CRED_TYPE_GENERIC,
                TargetName = target,
                CredentialBlobSize = (uint)blob.Length,
                CredentialBlob = blobPointer,
                Persist = CRED_PERSIST_LOCAL_MACHINE,
                UserName = user,
            };
            if (!CredWriteW(ref credential, 0))
            {
                throw new InvalidOperationException($"无法写入 Windows 凭据（错误 {Marshal.GetLastWin32Error()}）");
            }
        }
        finally
        {
            Array.Clear(blob);
            Marshal.FreeHGlobal(blobPointer);
            Marshal.FreeHGlobal(target);
            Marshal.FreeHGlobal(user);
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CREDENTIAL
    {
        public uint Flags;
        public uint Type;
        public IntPtr TargetName;
        public IntPtr Comment;
        // FILETIME as two DWORDs keeps the struct blittable for LibraryImport.
        public uint LastWrittenLow;
        public uint LastWrittenHigh;
        public uint CredentialBlobSize;
        public IntPtr CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public IntPtr Attributes;
        public IntPtr TargetAlias;
        public IntPtr UserName;
    }

    [LibraryImport("advapi32.dll", SetLastError = true, StringMarshalling = StringMarshalling.Utf16)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CredReadW(string target, uint type, uint flags, out IntPtr credential);

    [LibraryImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CredWriteW(ref CREDENTIAL credential, uint flags);

    [LibraryImport("advapi32.dll")]
    private static partial void CredFree(IntPtr buffer);
}
