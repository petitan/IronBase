using System;

namespace IronBase
{
    /// <summary>
    /// Base exception for all IronBase errors.
    /// </summary>
    public class IronBaseException : Exception
    {
        /// <summary>
        /// Native error code.
        /// </summary>
        public int ErrorCode { get; }

        public IronBaseException(string message) : base(message)
        {
            ErrorCode = -99;
        }

        public IronBaseException(int errorCode, string message) : base(message)
        {
            ErrorCode = errorCode;
        }

        public IronBaseException(string message, Exception innerException) : base(message, innerException)
        {
            ErrorCode = -99;
        }

        /// <summary>
        /// Create an appropriate exception from a native error code.
        /// </summary>
        internal static IronBaseException FromErrorCode(int errorCode, string message)
        {
            return errorCode switch
            {
                -1 => new IronBaseArgumentException(message),
                -2 => new IronBaseInvalidHandleException(message),
                -3 => new IronBaseIOException(message),
                -4 => new IronBaseSerializationException(message),
                -5 => new IronBaseCollectionNotFoundException(message),
                -6 => new IronBaseCollectionExistsException(message),
                -7 => new IronBaseDocumentNotFoundException(message),
                -8 => new IronBaseQueryException(message),
                -9 => new IronBaseCorruptionException(message),
                -10 => new IronBaseIndexException(message),
                -11 => new IronBaseAggregationException(message),
                -12 => new IronBaseSchemaException(message),
                -13 => new IronBaseTransactionException("Transaction already committed or aborted"),
                -14 => new IronBaseTransactionException(message),
                -15 => new IronBaseCorruptionException("WAL corruption detected"),
                _ => new IronBaseException(errorCode, message)
            };
        }
    }

    /// <summary>
    /// Thrown when a null or invalid argument is passed.
    /// </summary>
    public class IronBaseArgumentException : IronBaseException
    {
        public IronBaseArgumentException(string message) : base(-1, message) { }
    }

    /// <summary>
    /// Thrown when using an invalid or closed handle.
    /// </summary>
    public class IronBaseInvalidHandleException : IronBaseException
    {
        public IronBaseInvalidHandleException(string message) : base(-2, message) { }
    }

    /// <summary>
    /// Thrown on I/O errors (file system, etc.).
    /// </summary>
    public class IronBaseIOException : IronBaseException
    {
        public IronBaseIOException(string message) : base(-3, message) { }
    }

    /// <summary>
    /// Thrown on serialization/deserialization errors.
    /// </summary>
    public class IronBaseSerializationException : IronBaseException
    {
        public IronBaseSerializationException(string message) : base(-4, message) { }
    }

    /// <summary>
    /// Thrown when a collection is not found.
    /// </summary>
    public class IronBaseCollectionNotFoundException : IronBaseException
    {
        public IronBaseCollectionNotFoundException(string message) : base(-5, message) { }
    }

    /// <summary>
    /// Thrown when trying to create a collection that already exists.
    /// </summary>
    public class IronBaseCollectionExistsException : IronBaseException
    {
        public IronBaseCollectionExistsException(string message) : base(-6, message) { }
    }

    /// <summary>
    /// Thrown when a document is not found.
    /// </summary>
    public class IronBaseDocumentNotFoundException : IronBaseException
    {
        public IronBaseDocumentNotFoundException(string message) : base(-7, message) { }
    }

    /// <summary>
    /// Thrown on invalid query syntax.
    /// </summary>
    public class IronBaseQueryException : IronBaseException
    {
        public IronBaseQueryException(string message) : base(-8, message) { }
    }

    /// <summary>
    /// Thrown when database corruption is detected.
    /// </summary>
    public class IronBaseCorruptionException : IronBaseException
    {
        public IronBaseCorruptionException(string message) : base(-9, message) { }
    }

    /// <summary>
    /// Thrown on index operation errors.
    /// </summary>
    public class IronBaseIndexException : IronBaseException
    {
        public IronBaseIndexException(string message) : base(-10, message) { }
    }

    /// <summary>
    /// Thrown on aggregation pipeline errors.
    /// </summary>
    public class IronBaseAggregationException : IronBaseException
    {
        public IronBaseAggregationException(string message) : base(-11, message) { }
    }

    /// <summary>
    /// Thrown on schema validation errors.
    /// </summary>
    public class IronBaseSchemaException : IronBaseException
    {
        public IronBaseSchemaException(string message) : base(-12, message) { }
    }

    /// <summary>
    /// Thrown on transaction errors.
    /// </summary>
    public class IronBaseTransactionException : IronBaseException
    {
        public IronBaseTransactionException(string message) : base(-13, message) { }
    }
}
