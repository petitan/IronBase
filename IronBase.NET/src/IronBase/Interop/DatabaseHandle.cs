using System;
using System.Runtime.InteropServices;

namespace IronBase.Interop
{
    /// <summary>
    /// SafeHandle wrapper for native database handle.
    /// Ensures proper cleanup of native resources.
    /// </summary>
    public sealed class DatabaseHandle : SafeHandle
    {
        public DatabaseHandle() : base(IntPtr.Zero, ownsHandle: true)
        {
        }

        public override bool IsInvalid => handle == IntPtr.Zero;

        protected override bool ReleaseHandle()
        {
            if (!IsInvalid)
            {
                unsafe
                {
                    NativeMethods.ironbase_close((DatabaseHandle*)handle);
                }
            }
            return true;
        }

        internal void SetHandle(IntPtr ptr)
        {
            SetHandle(ptr);
        }
    }
}
