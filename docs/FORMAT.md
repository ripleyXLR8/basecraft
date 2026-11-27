# The `.fmp12` file format (HBAM7)

This is OpenMaker's working specification of the format FileMaker Pro 12–26
uses for `.fmp12` files. We are rebuilding it for interoperability. Sources:

* the public notes and MIT-licensed code of
  [fmptools](https://github.com/evanmiller/fmptools) by Evan Miller, which we
  used as the starting point for the sector and chunk layer;
* our own diff-driven analysis: change one thing in FileMaker, save, compare
  the files with `fmdump`.

Status flags: ✅ implemented and tested · 🔍 observed but not decoded · ❓ unknown.

## 1. Sectors ✅

The file is a sequence of 4096-byte sectors. Sector 0 is the **header**:

| Offset | Length | Meaning |
|-------:|-------:|---------|
| 0 | 15 | magic `00 01 00 00 00 02 00 01 00 05 00 02 00 02 C0` |
| 15 | 5 | `HBAM7` (plain) or `HBAMe` (encryption at rest, not supported) |
| 541 | 1+n | Pascal string naming the creator, e.g. `Pro 12.0` |

The format version has not changed since 12, so FileMaker 26 files still say `Pro 12.0`.

Body sector layout:

| Offset | Length | Meaning |
|-------:|-------:|---------|
| 0 | 1 | deleted flag |
| 4 | 4 | previous sector id (BE) |
| 8 | 4 | next sector id (BE) |
| 12 | 2 | B-tree level (0 = leaf) |
| 14 | 6 | ❓ |
| 20 | 4076 | payload (byte code, §2) |

Sector *k* starts at offset `k × 4096`. For sector 1 (the B-tree root), the
"next" field holds the number of body sectors, so
`(next + 1) × 4096 == file size`.

The sectors form a B-tree. All data lives in the leaves (level 0), which are
chained through prev/next. The chain starts at the leaf whose prev is 0.
Index sectors (level ≥ 1) map paths to child sectors. Readers can ignore
them, but a writer must keep them consistent.

## 2. Payload byte code ✅

Each payload is a stream of opcodes. Each sector rebuilds its *path* (a stack
of byte strings) from an empty stack. Chunks are:
*Push* / *Pop* (path), *Data* (anonymous), *Kv* (integer key → value),
*KvLong* (bytes key → value) and *Segment* (part *n* of a long value).

If `op & 0xC0 == 0xC0`, the chunk is decoded as `op & 0x3F` and followed by a Pop.

| Op | Chunk | Encoding after the op byte |
|----|-------|----------------------------|
| 00 | Data / end | 1 byte. `00 00` or end of buffer ends the payload |
| 01 | Kv | key u8, 1 byte value |
| 02–05 | Kv | key u8, 2×(op−1) bytes |
| 06 | Kv | key u8, len u8, value |
| 07 | Segment | index u8, len u16, value |
| 08 | Data | 2 bytes |
| 09 | Kv | key path-int(2), 1 byte |
| 0A–0D | Kv | key path-int(2), 2×(op−9) bytes |
| 0E FF | Data | 6 bytes (FF included) |
| 0E | Kv | key path-int(2), len u8, value |
| 0F | Segment | index path-int(2), len u16, value |
| 10 | Data | 3 bytes |
| 11–15 | Data | 3 + (op==11) + 2×(op−11) bytes |
| 16 | KvLong | key 3 bytes, len u8, value |
| 17 | KvLong | key 3 bytes, len u16, value |
| 18, 23 | Data | len u8, value |
| 1B 00 | Kv | key u8, 4 bytes |
| 19–1D | Data | len u8, value, then (op==19)+2×(op−19) trailing bytes 🔍 |
| 1E | KvLong | klen u8, key, len u8, value |
| 1F | KvLong | klen u8, key, len u16, value |
| 20 | Push | 1 byte, or `FE` + 8 bytes |
| 28 | Push | 2 bytes |
| 30 | Push | 3 bytes |
| 38 | Push | len u8, bytes |
| 3D, 40 | Pop | — |
| 80, 88 | no-op | — |

**Path integers.** 1 byte: value. 2 bytes: `0x80 + ((b0 & 0x7F) << 8) + b1`.
3 bytes: `0x80 + (b1 << 8) + b2`.

## 3. Text ✅

Text values are XOR-ed with `0x5A`, then encoded with
[SCSU](https://www.unicode.org/reports/tr6/). CR (`\r`) is FileMaker's
paragraph separator (¶) and is kept as is.

## 4. Path map

`T` is a table id (≥ 128).

| Path | Content | Status |
|------|---------|--------|
| `[2]` | file-level settings | 🔍 |
| `[3].[16].[5].[T]` | table definition: key 16 = name, 216/217 = UUIDs, 31873/31874 = created/modified by account, 31875 = timestamp | ✅ name |
| `[3].[17]` | table occurrences / relationship graph | 🔍 |
| `[4]` | layouts | 🔍 |
| `[6]` | ❓ | 🔍 |
| `[17]` | scripts | 🔍 |
| `[23]` | accounts / privileges | 🔍 |
| `[T].[3].[5].[F]` | field definition: key 16 = name | ✅ name |
| `[T].[3].[1]` | field index (collation keys) | 🔍 |
| `[T].[5].[R]` | record R: key F = value of field F | ✅ |
| `[T].[5].[R].[F]` | long value of field F, keys/segments 1…n (key 0 = style runs) | ✅ |
| Long names | `[node].[16]` with keys 1…n when the name does not fit inline | ✅ |

Rows marked 🔍 are hypotheses to confirm with differential files (see
`PROGRESS.md`, milestone M1).
