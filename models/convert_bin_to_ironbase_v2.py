#!/usr/bin/env python3
"""Convert FastText .bin (binary) format to IronBase v2 binary format.

Streaming architecture: reads vectors on-demand from mmap, never loads
the full matrix into Python memory. Peak memory: O(vocab_words_strings + dim).

IronBase v2 format:
  Header (32 bytes):
    magic:         [u8; 4]  = b"IBv2"
    dim:           u32
    vocab_size:    u32
    bucket_count:  u32
    minn:          u32
    maxn:          u32
    bucket_offset: u64      = byte offset to bucket section

  Word Section (byte 32 .. bucket_offset):
    [word_bytes\0][f32 × dim] × vocab_size

  Bucket Section (bucket_offset .. EOF):
    [f32 × dim] × bucket_count
"""

import mmap
import struct
import sys
import time
import array


def fnv_hash(data: bytes) -> int:
    """FNV-1a hash (FastText convention)."""
    h = 2166136261
    for b in data:
        h ^= b
        h = (h * 16777619) & 0xFFFFFFFF
    return h


def char_ngrams(word: str, minn: int, maxn: int) -> list[str]:
    """Extract character n-grams from padded word '<word>'."""
    padded = f"<{word}>"
    ngrams = []
    for n in range(minn, maxn + 1):
        for i in range(len(padded) - n + 1):
            ngrams.append(padded[i:i + n])
    return ngrams


def read_vec_from_mmap(mm, offset: int, dim: int) -> array.array:
    """Read a single f32 vector from mmap into an array (compact memory)."""
    vec = array.array('f')
    vec.frombytes(mm[offset:offset + dim * 4])
    return vec


def compute_word_vector_streaming(mm, word_vec_offset: int, bucket_base_offset: int,
                                  word: str, minn: int, maxn: int,
                                  bucket_count: int, dim: int) -> bytes:
    """Compute final word vector from mmap on-demand, return as raw bytes.

    Reads raw word vector + each bucket vector directly from mmap.
    Memory: O(dim) — one accumulator array.
    """
    vec_bytes = dim * 4

    # Read raw word vector into accumulator
    result = array.array('f')
    result.frombytes(mm[word_vec_offset:word_vec_offset + vec_bytes])
    count = 1

    # Add subword bucket vectors
    ngrams = char_ngrams(word, minn, maxn)
    for ng in ngrams:
        bucket_id = fnv_hash(ng.encode('utf-8')) % bucket_count
        boffset = bucket_base_offset + bucket_id * vec_bytes
        bvec = array.array('f')
        bvec.frombytes(mm[boffset:boffset + vec_bytes])
        for i in range(dim):
            result[i] += bvec[i]
        count += 1

    # Average
    for i in range(dim):
        result[i] /= count

    return result.tobytes()


def convert_streaming(input_path: str, output_path: str):
    """Stream-convert FastText .bin to IronBase v2. O(vocab_strings + dim) memory."""
    with open(input_path, 'rb') as f:
        mm = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)

        # ---- Parse header ----
        magic, version = struct.unpack_from('<ii', mm, 0)
        print(f"FastText magic={magic}, version={version}")

        offset = 8
        (dim, ws, epoch, minCount, neg, wordNgrams, loss, model,
         bucket, minn, maxn, lrUpdateRate, t) = struct.unpack_from('<iiiiiiiiiiiid', mm, offset)
        offset += struct.calcsize('<iiiiiiiiiiiid')

        print(f"dim={dim}, bucket={bucket}, minn={minn}, maxn={maxn}")
        print(f"ws={ws}, epoch={epoch}, minCount={minCount}")

        # ---- Vocabulary (only word strings, no vectors) ----
        vocab_size, nwords, nlabels = struct.unpack_from('<iii', mm, offset)
        offset += 12
        print(f"vocab_size={vocab_size}, nwords={nwords}, nlabels={nlabels}")

        ntokens = struct.unpack_from('<q', mm, offset)[0]
        offset += 8
        pruneidx_size = struct.unpack_from('<q', mm, offset)[0]
        offset += 8
        print(f"ntokens={ntokens}, pruneidx_size={pruneidx_size}")

        if pruneidx_size > 0:
            offset += pruneidx_size * 8

        # Read only word strings (not vectors) — O(vocab) for strings
        words = []
        for i in range(vocab_size):
            word_start = offset
            while mm[offset] != 0:
                offset += 1
            word = mm[word_start:offset].decode('utf-8', errors='replace')
            offset += 1  # skip null
            offset += 9  # skip count(i64) + type(u8)
            words.append(word)
            if (i + 1) % 500_000 == 0:
                print(f"  Vocab: {i + 1}/{vocab_size}...")

        print(f"Vocabulary read: {len(words)} words (strings only, no vectors)")

        # ---- Input matrix location ----
        quant_input = struct.unpack_from('<B', mm, offset)[0]
        offset += 1
        if quant_input != 0:
            print("ERROR: Quantized models not supported!", file=sys.stderr)
            sys.exit(1)

        rows, cols = struct.unpack_from('<qq', mm, offset)
        offset += 16
        print(f"Input matrix: {rows} rows x {cols} cols")
        assert cols == dim
        assert rows == vocab_size + bucket

        matrix_offset = offset  # word vectors start here
        vec_bytes = dim * 4
        bucket_base_offset = matrix_offset + vocab_size * vec_bytes  # buckets start here

        print(f"Matrix at byte {matrix_offset}, buckets at byte {bucket_base_offset}")
        print(f"Input file size: {len(mm):,} bytes")

        # ---- Write output (streaming) ----
        print(f"\nWriting IronBase v2: {vocab_size} words, {bucket} buckets, dim={dim}")

        with open(output_path, 'wb') as out:
            # Placeholder header
            out.write(b'\0' * 32)

            # Word section: compute and write one word at a time
            print("Computing word vectors (streaming)...")
            t0 = time.time()

            for i, word in enumerate(words):
                word_vec_offset = matrix_offset + i * vec_bytes

                # Compute final vector: mean(raw_word_vec, subword_bucket_vecs...)
                final_vec_bytes = compute_word_vector_streaming(
                    mm, word_vec_offset, bucket_base_offset,
                    word, minn, maxn, bucket, dim
                )

                # Write word (null-terminated) + vector
                out.write(word.encode('utf-8') + b'\0')
                out.write(final_vec_bytes)

                if (i + 1) % 100_000 == 0:
                    elapsed = time.time() - t0
                    rate = (i + 1) / elapsed
                    eta = (vocab_size - i - 1) / rate
                    print(f"  Words: {i + 1}/{vocab_size} ({elapsed:.0f}s, ~{eta:.0f}s remaining)")

            print(f"Word section done in {time.time() - t0:.1f}s")

            # Record bucket offset
            out_bucket_offset = out.tell()
            print(f"Bucket offset: {out_bucket_offset}")

            # Bucket section: raw copy from input mmap (no Python float conversion!)
            print("Copying bucket vectors (raw bytes)...")
            t0 = time.time()

            chunk_size = 1000 * vec_bytes  # copy 1000 vectors at a time
            src_offset = bucket_base_offset
            remaining = bucket * vec_bytes

            while remaining > 0:
                n = min(chunk_size, remaining)
                out.write(mm[src_offset:src_offset + n])
                src_offset += n
                remaining -= n

            print(f"Bucket section done in {time.time() - t0:.1f}s")

            # Write final header
            out.seek(0)
            header = struct.pack('<4sIIIIIQ',
                                 b'IBv2', dim, vocab_size, bucket,
                                 minn, maxn, out_bucket_offset)
            assert len(header) == 32
            out.write(header)

        mm.close()

    import os
    file_size = os.path.getsize(output_path)
    print(f"\nDone! Wrote {output_path} ({file_size / (1024**3):.2f} GB)")


if __name__ == '__main__':
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} input.bin output.ironbase.v2.bin")
        print(f"\nExample:")
        print(f"  python3 {sys.argv[0]} cc.hu.300.bin cc.hu.300.ironbase.v2.bin")
        sys.exit(1)

    convert_streaming(sys.argv[1], sys.argv[2])
