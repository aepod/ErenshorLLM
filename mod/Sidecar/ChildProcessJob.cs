using System;
using System.Diagnostics;
using System.Runtime.InteropServices;

namespace ErenshorLLMDialog.Sidecar
{
    /// <summary>
    /// Windows Job Object wrapper that ensures child processes are killed when
    /// the parent (Unity/Erenshor) exits -- even on crash or force-close.
    ///
    /// Usage: call AssignProcess(process) after Process.Start(). When the game
    /// process exits for any reason, Windows automatically terminates all
    /// assigned child processes.
    ///
    /// This is a singleton -- one Job Object handles all child processes.
    /// On non-Windows platforms, this is a no-op.
    /// </summary>
    internal static class ChildProcessJob
    {
        private static IntPtr _jobHandle = IntPtr.Zero;
        private static bool _initialized;

        /// <summary>
        /// Assign a child process to the job. The process will be killed
        /// automatically when the parent process exits.
        /// </summary>
        public static void AssignProcess(Process process)
        {
            if (process == null || process.HasExited)
                return;

            // Only works on Windows
            if (!IsWindows())
                return;

            try
            {
                EnsureInitialized();

                if (_jobHandle == IntPtr.Zero)
                    return;

                if (!AssignProcessToJobObject(_jobHandle, process.Handle))
                {
                    int error = Marshal.GetLastWin32Error();
                    // Error 5 = Access Denied (can happen if process is already in a job)
                    // This is non-fatal -- the process just won't auto-terminate.
                    if (error != 5)
                    {
                        UnityEngine.Debug.LogWarning(
                            "[ChildProcessJob] AssignProcessToJobObject failed: error " + error);
                    }
                }
            }
            catch (Exception e)
            {
                UnityEngine.Debug.LogWarning(
                    "[ChildProcessJob] Failed to assign process to job: " + e.Message);
            }
        }

        private static void EnsureInitialized()
        {
            if (_initialized)
                return;

            _initialized = true;

            try
            {
                _jobHandle = CreateJobObject(IntPtr.Zero, null);
                if (_jobHandle == IntPtr.Zero)
                {
                    UnityEngine.Debug.LogWarning(
                        "[ChildProcessJob] CreateJobObject failed: " + Marshal.GetLastWin32Error());
                    return;
                }

                // Configure the job to kill all child processes when the job handle
                // is closed (which happens when the parent process exits).
                var info = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

                int size = Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION));
                IntPtr ptr = Marshal.AllocHGlobal(size);
                try
                {
                    Marshal.StructureToPtr(info, ptr, false);
                    if (!SetInformationJobObject(_jobHandle,
                        JobObjectInfoType.ExtendedLimitInformation, ptr, (uint)size))
                    {
                        UnityEngine.Debug.LogWarning(
                            "[ChildProcessJob] SetInformationJobObject failed: " +
                            Marshal.GetLastWin32Error());
                        CloseHandle(_jobHandle);
                        _jobHandle = IntPtr.Zero;
                    }
                }
                finally
                {
                    Marshal.FreeHGlobal(ptr);
                }
            }
            catch (Exception e)
            {
                UnityEngine.Debug.LogWarning(
                    "[ChildProcessJob] Initialization failed: " + e.Message);
                _jobHandle = IntPtr.Zero;
            }
        }

        private static bool IsWindows()
        {
            // Unity on Windows: Environment.OSVersion.Platform == PlatformID.Win32NT
            return Environment.OSVersion.Platform == PlatformID.Win32NT;
        }

        // --- Win32 P/Invoke ---

        private const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;

        private enum JobObjectInfoType
        {
            ExtendedLimitInformation = 9
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_LIMIT_INFORMATION
        {
            public long PerProcessUserTimeLimit;
            public long PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct IO_COUNTERS
        {
            public ulong ReadOperationCount;
            public ulong WriteOperationCount;
            public ulong OtherOperationCount;
            public ulong ReadTransferCount;
            public ulong WriteTransferCount;
            public ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
            public IO_COUNTERS IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        private static extern IntPtr CreateJobObject(IntPtr lpJobAttributes, string lpName);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool SetInformationJobObject(
            IntPtr hJob, JobObjectInfoType infoType, IntPtr lpJobObjectInfo, uint cbJobObjectInfoLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool AssignProcessToJobObject(IntPtr hJob, IntPtr hProcess);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CloseHandle(IntPtr hObject);
    }
}
