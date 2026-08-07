param(
    [Parameter(Mandatory = $true)]
    [string]$FilePath
)

$ErrorActionPreference = "Stop"

$resolvedExecutable = (Resolve-Path -LiteralPath $FilePath).Path
if ([System.IO.Path]::GetExtension($resolvedExecutable) -ine ".exe") {
    throw "Windows app icon verification requires an executable: $resolvedExecutable"
}

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class NativeIconResourceVerifier
{
    private const uint LOAD_LIBRARY_AS_DATAFILE = 0x00000002;
    private const int RT_GROUP_ICON = 14;

    private delegate bool EnumResourceNameCallback(
        IntPtr module,
        IntPtr type,
        IntPtr name,
        IntPtr parameter
    );

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr LoadLibraryEx(
        string fileName,
        IntPtr file,
        uint flags
    );

    [DllImport("kernel32.dll", SetLastError = true)]
    private static extern bool FreeLibrary(IntPtr module);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool EnumResourceNames(
        IntPtr module,
        IntPtr type,
        EnumResourceNameCallback callback,
        IntPtr parameter
    );

    public static bool HasGroupIcon(string fileName)
    {
        IntPtr module = LoadLibraryEx(fileName, IntPtr.Zero, LOAD_LIBRARY_AS_DATAFILE);
        if (module == IntPtr.Zero)
        {
            throw new System.ComponentModel.Win32Exception(
                Marshal.GetLastWin32Error(),
                "Unable to load executable resources"
            );
        }

        try
        {
            bool found = false;
            EnumResourceNameCallback callback = delegate(
                IntPtr callbackModule,
                IntPtr type,
                IntPtr name,
                IntPtr parameter
            ) {
                found = true;
                return false;
            };
            EnumResourceNames(module, new IntPtr(RT_GROUP_ICON), callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            return found;
        }
        finally
        {
            FreeLibrary(module);
        }
    }
}
'@

if (-not [NativeIconResourceVerifier]::HasGroupIcon($resolvedExecutable)) {
    throw "Windows executable does not contain an RT_GROUP_ICON resource: $resolvedExecutable"
}

Add-Type -AssemblyName System.Drawing
$associatedIcon = [System.Drawing.Icon]::ExtractAssociatedIcon($resolvedExecutable)
if ($null -eq $associatedIcon) {
    throw "Windows could not extract the taskbar icon from: $resolvedExecutable"
}
try {
    if ($associatedIcon.Width -lt 16 -or $associatedIcon.Height -lt 16) {
        throw "Extracted Windows icon is unexpectedly small: $($associatedIcon.Width)x$($associatedIcon.Height)"
    }
} finally {
    $associatedIcon.Dispose()
}

Write-Host "Verified embedded Windows executable and taskbar icon resources: $resolvedExecutable"
