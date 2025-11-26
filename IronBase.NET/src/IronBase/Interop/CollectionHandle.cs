using System;
using System.Runtime.InteropServices;

namespace IronBase.Interop
{
    /// <summary>
    /// SafeHandle wrapper for native collection handle.
    /// Ensures proper cleanup of native resources.
    /// </summary>
    public sealed class CollectionHandle : SafeHandle
    {
        public CollectionHandle() : base(IntPtr.Zero, ownsHandle: true)
        {
        }

        public override bool IsInvalid => handle == IntPtr.Zero;

        protected override bool ReleaseHandle()
        {
            if (!IsInvalid)
            {
                unsafe
                {
                    NativeMethods.ironbase_collection_release((CollectionHandle*)handle);
                }
            }
            return true;
        }

        internal new void SetHandle(IntPtr ptr)
        {
            base.SetHandle(ptr);
        }
    }
}
