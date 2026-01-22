// Manual additions to NativeMethods
// These methods are added manually and won't be overwritten by csbindgen

using System;
using System.Runtime.InteropServices;

namespace IronBase.Interop
{
    internal static unsafe partial class NativeMethods
    {
        /// <summary>
        /// Update one document with upsert support.
        ///
        /// If upsert is true (non-zero) and no document matches the filter,
        /// a new document is created from filter + update operations.
        /// </summary>
        /// <param name="handle">Collection handle</param>
        /// <param name="query_json">Query filter as JSON (UTF-8)</param>
        /// <param name="update_json">Update operations as JSON (UTF-8)</param>
        /// <param name="upsert">Non-zero to enable upsert</param>
        /// <param name="out_matched">Receives matched count</param>
        /// <param name="out_modified">Receives modified count</param>
        /// <param name="out_upserted_id">Receives upserted ID (null if no upsert occurred)</param>
        /// <returns>0 on success, error code on failure</returns>
        [DllImport(__DllName, EntryPoint = "ironbase_update_one_with_options", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]
        internal static extern int ironbase_update_one_with_options(
            CollectionHandle* handle,
            byte* query_json,
            byte* update_json,
            int upsert,
            ulong* out_matched,
            ulong* out_modified,
            byte** out_upserted_id
        );
    }
}
