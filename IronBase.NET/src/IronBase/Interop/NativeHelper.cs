using System;
using System.Runtime.InteropServices;
using System.Text;

namespace IronBase.Interop
{
    /// <summary>
    /// Helper methods for native interop.
    /// </summary>
    internal static class NativeHelper
    {
        /// <summary>
        /// Convert a managed string to a null-terminated UTF-8 byte array.
        /// </summary>
        public static byte[] ToUtf8(string? str)
        {
            if (str == null)
                return new byte[] { 0 };

            var bytes = Encoding.UTF8.GetBytes(str);
            var result = new byte[bytes.Length + 1];
            Array.Copy(bytes, result, bytes.Length);
            result[bytes.Length] = 0;
            return result;
        }

        /// <summary>
        /// Convert a native UTF-8 string pointer to a managed string.
        /// </summary>
        public static unsafe string? PtrToStringUtf8(byte* ptr)
        {
            if (ptr == null)
                return null;

            int length = 0;
            while (ptr[length] != 0)
                length++;

            if (length == 0)
                return string.Empty;

            return Encoding.UTF8.GetString(ptr, length);
        }

        /// <summary>
        /// Convert a native UTF-8 string pointer to a managed string and free the native memory.
        /// </summary>
        public static unsafe string? PtrToStringUtf8AndFree(byte* ptr)
        {
            if (ptr == null)
                return null;

            var result = PtrToStringUtf8(ptr);
            NativeMethods.ironbase_free_string(ptr);
            return result;
        }

        /// <summary>
        /// Get the last error message from the native library.
        /// </summary>
        public static unsafe string? GetLastError()
        {
            var ptr = NativeMethods.ironbase_get_last_error();
            return PtrToStringUtf8(ptr);
        }

        /// <summary>
        /// Check if an error code indicates success.
        /// </summary>
        public static bool IsSuccess(int errorCode) => errorCode == 0;

        /// <summary>
        /// Throw an exception if the error code indicates failure.
        /// </summary>
        public static void ThrowIfError(int errorCode)
        {
            if (errorCode != 0)
            {
                var message = GetLastError() ?? $"Unknown error (code: {errorCode})";
                throw IronBaseException.FromErrorCode(errorCode, message);
            }
        }
    }
}
