using System;
using System.Collections;
using System.Collections.Generic;
using System.Text.Json;
using IronBase.Interop;

namespace IronBase
{
    /// <summary>
    /// Cursor for streaming through large query result sets.
    /// Provides memory-efficient iteration over documents.
    /// </summary>
    /// <typeparam name="T">Document type</typeparam>
    public sealed class IronBaseCursor<T> : IEnumerable<T>, IDisposable where T : class
    {
        private IntPtr _handle;
        private bool _disposed;

        internal IronBaseCursor(IntPtr handle)
        {
            _handle = handle;
        }

        /// <summary>
        /// Get the next document, or null if exhausted.
        /// </summary>
        public T? Next()
        {
            ThrowIfDisposed();

            unsafe
            {
                var jsonPtr = NativeMethods.ironbase_cursor_next((CursorHandle*)_handle);
                if (jsonPtr == null)
                    return default;

                var json = NativeHelper.PtrToStringUtf8AndFree(jsonPtr);
                if (string.IsNullOrEmpty(json))
                    return default;

                return JsonSerializer.Deserialize<T>(json);
            }
        }

        /// <summary>
        /// Get the next batch of documents.
        /// </summary>
        public List<T> NextBatch()
        {
            ThrowIfDisposed();

            unsafe
            {
                var jsonPtr = NativeMethods.ironbase_cursor_next_batch((CursorHandle*)_handle);
                if (jsonPtr == null)
                    return new List<T>();

                var json = NativeHelper.PtrToStringUtf8AndFree(jsonPtr);
                if (string.IsNullOrEmpty(json) || json == "[]")
                    return new List<T>();

                return JsonSerializer.Deserialize<List<T>>(json) ?? new List<T>();
            }
        }

        /// <summary>
        /// Get a specific chunk of documents.
        /// </summary>
        public List<T> NextChunk(uint chunkSize)
        {
            ThrowIfDisposed();

            unsafe
            {
                var jsonPtr = NativeMethods.ironbase_cursor_next_chunk((CursorHandle*)_handle, chunkSize);
                if (jsonPtr == null)
                    return new List<T>();

                var json = NativeHelper.PtrToStringUtf8AndFree(jsonPtr);
                if (string.IsNullOrEmpty(json) || json == "[]")
                    return new List<T>();

                return JsonSerializer.Deserialize<List<T>>(json) ?? new List<T>();
            }
        }

        /// <summary>
        /// Get remaining document count.
        /// </summary>
        public ulong Remaining
        {
            get
            {
                ThrowIfDisposed();
                unsafe
                {
                    return NativeMethods.ironbase_cursor_remaining((CursorHandle*)_handle);
                }
            }
        }

        /// <summary>
        /// Get total document count.
        /// </summary>
        public ulong Total
        {
            get
            {
                ThrowIfDisposed();
                unsafe
                {
                    return NativeMethods.ironbase_cursor_total((CursorHandle*)_handle);
                }
            }
        }

        /// <summary>
        /// Get current position.
        /// </summary>
        public ulong Position
        {
            get
            {
                ThrowIfDisposed();
                unsafe
                {
                    return NativeMethods.ironbase_cursor_position((CursorHandle*)_handle);
                }
            }
        }

        /// <summary>
        /// Check if cursor is exhausted.
        /// </summary>
        public bool IsFinished
        {
            get
            {
                ThrowIfDisposed();
                unsafe
                {
                    return NativeMethods.ironbase_cursor_is_finished((CursorHandle*)_handle) != 0;
                }
            }
        }

        /// <summary>
        /// Reset cursor to the beginning.
        /// </summary>
        public void Rewind()
        {
            ThrowIfDisposed();
            unsafe
            {
                NativeMethods.ironbase_cursor_rewind((CursorHandle*)_handle);
            }
        }

        /// <summary>
        /// Skip the next N documents.
        /// </summary>
        public void Skip(ulong n)
        {
            ThrowIfDisposed();
            unsafe
            {
                NativeMethods.ironbase_cursor_skip((CursorHandle*)_handle, n);
            }
        }

        /// <summary>
        /// Collect all remaining documents into a list.
        /// </summary>
        public List<T> CollectAll()
        {
            ThrowIfDisposed();

            unsafe
            {
                var jsonPtr = NativeMethods.ironbase_cursor_collect_all((CursorHandle*)_handle);
                if (jsonPtr == null)
                    return new List<T>();

                var json = NativeHelper.PtrToStringUtf8AndFree(jsonPtr);
                if (string.IsNullOrEmpty(json) || json == "[]")
                    return new List<T>();

                return JsonSerializer.Deserialize<List<T>>(json) ?? new List<T>();
            }
        }

        /// <summary>
        /// Take the next N documents.
        /// </summary>
        public List<T> Take(uint n)
        {
            return NextChunk(n);
        }

        private void ThrowIfDisposed()
        {
            if (_disposed)
                throw new ObjectDisposedException(nameof(IronBaseCursor<T>));
        }

        public IEnumerator<T> GetEnumerator()
        {
            while (!IsFinished)
            {
                var doc = Next();
                if (doc != null)
                    yield return doc;
            }
        }

        IEnumerator IEnumerable.GetEnumerator() => GetEnumerator();

        public void Dispose()
        {
            if (!_disposed)
            {
                _disposed = true;
                if (_handle != IntPtr.Zero)
                {
                    unsafe
                    {
                        NativeMethods.ironbase_cursor_release((CursorHandle*)_handle);
                    }
                    _handle = IntPtr.Zero;
                }
            }
        }
    }
}
