# The `.fmp12` file format (HBAM7)

This is BaseCraft's working specification of the format FileMaker Pro 12–26
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
| 0x800 | 1 | **clean-close flag**: `AC` = closed cleanly; FileMaker clears it while it writes the file and checks a file without it on open |
| 0x802 | 8 | **file sequence number** (u64 BE): FileMaker sets it on close to the largest sector write sequence number (offset 16 below) |

The format version has not changed since 12, so FileMaker 26 files still say `Pro 12.0`.

Only the flag and the sequence number change when FileMaker writes a file
(the rest of sector 0 stays as created; same finding in OpenMaker's notes,
`HBFile::WriteCleanCloseAndSCN`). Files FileMaker closed have, e.g.,
`AC 00 … 0B FB` with 0x0BFB = 3067 = the largest sector sequence number.
BaseCraft's writer (`write::build`) copies sector 0, sets the flag to `AC`
and keeps a sequence number at least that of its sectors (it stamps them
all 1), so FileMaker opens its files as cleanly closed.

Checked in FileMaker Pro 26: a copy FileMaker was killed on had `00` at
0x800 and sectors stamped up to 2162 over a file number of 2090; merely
opening and closing it, FileMaker rewrote it (7 → 6 sectors, number 2194,
flag `AC`): the consistency check. The same file saved by BaseCraft (`AC`)
opened and closed byte for byte unchanged.

Body sector layout:

| Offset | Length | Meaning |
|-------:|-------:|---------|
| 0 | 1 | deleted flag |
| 4 | 4 | previous sector id (BE) |
| 8 | 4 | next sector id (BE) |
| 12 | 2 | B-tree level (0 = leaf) |
| 14 | 2 | free bytes in the payload (4076 − used) |
| 16 | 4 | write sequence number (FileMaker's consistency check resets the maximum) |
| 20 | 4076 | payload (byte code, §2) |

Sector *k* starts at offset `k × 4096`. For sector 1 (the B-tree root), the
"next" field holds the number of body sectors, so
`(next + 1) × 4096 == file size`.

The sectors form a B-tree. All data lives in the leaves (level 0), which are
chained through prev/next. The chain starts at the leaf whose prev is 0.
Every leaf payload starts from an empty path and pushes the full path of its
first chunk. A chunk never spans two sectors. When the next chunk does not
fit, FileMaker still writes the pops towards its path at the end of the full
sector (`… 40 40`). **These pops are required**: a chunk only goes into a
sector when the pops that follow it still fit. A full sector that ends
deeper than the common prefix of its last path and the next sector's first
path is a bad block for FileMaker Server's consistency check
(FMDeveloperTool, error 805), although FileMaker Pro opens the file. Every
leaf of the 66 templates ends that way; BaseCraft's packer
(`write::pack`) dropped the pops when they did not fit, so about one
rewrite in five (14 of the 66 templates, and files FileMaker had edited,
e.g. after a layout's window was resized) got such a sector.

**Sort order.** Store order is byte order on the sequence of path elements
followed by the chunk's key bytes. A key is just one more element:
`[04.05.03.05.11.03] key 6` sorts before `[04.05.03.05.11] key 7` because
element `03` < key `07`. The *spelling* of a key matters, not its value.
FileMaker writes key 216 as `06 D8 …` (1-byte key) but key 134 as
`0B 00 06 …` (2-byte path integer). Both read back as integers, so a writer
must keep the original bytes (BaseCraft's writer does).

### Index sectors ✅

Index sectors (level ≥ 1) map separators to child sectors. Readers can ignore
them, but **FileMaker validates them** on open ("Les autorisations d'accès du
fichier ont été endommagées"). All 66 templates have a single level-1 root.
Like a leaf, each index payload starts from an empty stack.

An entry is a run of pushes (`20 x`, `28 xx`, `30 xxx`, `38 len …`,
`20 FE` + 8), then an item whose last 4 bytes are the child sector id:

| Item | Separator |
|------|-----------|
| `03 k id` | path + 1-byte key `k` |
| `0B k k id` | path + 2-byte key |
| `13 k k k id` | path + 3-byte key |
| `1B len key id` | path + long key |
| `1B 00 01 id` | the path itself, sorting before everything under it |
| `1B 00 FF id` | the path, sorting after everything under it (seen 6×) |

**Pop flags** (`| 0xC0`, e.g. `E0 05`, `F8 04 …`, `C3 …`) replace explicit
pops. A pushed element stays on the stack whether flagged or not. After a
flagged item, the decoder pops every flagged element on top of the stack,
then one unflagged element. An encoder that knows the next separator
therefore flags an element exactly when it leaves the stack together with
the element below it. Elements left at the end of the sector are all
flagged, and so is the last item. This rule re-encodes all 66 FileMaker
index sectors byte for byte (`tests/templates.rs`,
`index_sectors_reencode_byte_for_byte`).

**Choice of separator.** FileMaker normally names the child's first chunk
exactly: path, then that chunk's key in the forms above. A child that starts
with a node marker (op `80`, §2) gets `1B 00 01` on that path, and the
leftmost child of the root gets `1B 00 01` on the empty path. A few
separators are stale but still valid. They lie strictly between the left
child's last key and the right child's first key, for instance
`8003.00.07.d00003.d00002` before a child starting at `8003.07`. They are
left over from earlier content, so the choice cannot always be reproduced
from the file alone. Any separator in that interval seems acceptable.
BaseCraft always writes the exact first key.

**Proven with FileMaker Pro 26.** On a copy of Contacts with its original
leaves and only the root rebuilt by these rules, the file opens. The full
`fmp12::write::rewrite` output also opens, with all sequence numbers set to 1
and the header sector unchanged. This holds for Contacts, Tasks, Assets,
Content Management, Inventory and Meetings (en), Assets (ja), Inventory (it),
Tasks (zh) and Contacts (fr). For Contacts, Tasks and Assets (en), the *Save a
Copy as XML* exports of the original and the rewrite are identical. The only
exception is DDR_INFO: its order and the `DDRREF` ids pointing into it
change from one export to the next even for the same file. After resolving
each `DDRREF` to the hash it points to, the diff is empty.

**In-place edits work.** Copies of real files with single bytes changed
(same length) open in FileMaker without complaint: there is no checksum on
sectors or on the file as a whole. A few catalogs do carry digests of their
own content, checked when the file opens: the script list (`[17].[1]` key 8,
§8) and the accounts and privilege catalogs (`[23]`, §15): plain MD5s
that BaseCraft recomputes when it writes them.
We use this to reverse-engineer field semantics (`tools/oracle`).

## 2. Payload byte code ✅

Each payload is a stream of opcodes. Each sector rebuilds its *path* (a stack
of byte strings) from an empty stack. Chunks are:
*Push* / *Pop* (path), *Data* (anonymous), *Kv* (integer key → value),
and *KvLong* (bytes key → value). Opcodes 07/0F, which fmptools reads as "segments", are plain Kv chunks with a 2-byte length. Values over ~64 KB are stored in a sub-path as keys 1…n.

If `op & 0xC0 == 0xC0`, the chunk is decoded as `op & 0x3F` and followed by a Pop.

Ops 00, 08 and 10–15, which the reader still returns as *Data*, are really Kv
chunks with a 1, 2 or 3-byte key: 00/08/10 with an empty value, 11–15 with
1/2/4/6/8 bytes (FileMaker's index separators name them by that key).
`0E FF …` is an ordinary 2-byte-key Kv (key `FF xx`, length byte, value),
not 6 bytes of data.

| Op | Chunk | Encoding after the op byte |
|----|-------|----------------------------|
| 00 | Data / end | 1 byte: `00 k` is key k with an empty value (`00 00` = key 0, e.g. an omit find request). Only zero padding ends the payload: the sector header's free count (offset 14) gives the used length, and the rest is zeros |
| 01 | Kv | key u8, 1 byte value |
| 02–05 | Kv | key u8, 2×(op−1) bytes |
| 06 | Kv | key u8, len u8, value |
| 07 | Kv | key u8, len u16, value (long form of 06) |
| 08 | Data | 2 bytes |
| 09 | Kv | key path-int(2), 1 byte |
| 0A–0D | Kv | key path-int(2), 2×(op−9) bytes |
| 0E FF | Data | 6 bytes (FF included) |
| 0E | Kv | key path-int(2), len u8, value |
| 0F | Kv | key path-int(2), len u16, value (long form of 0E) |
| 10 | Data | 3 bytes |
| 11–15 | Data | 3 + (op==11) + 2×(op−11) bytes |
| 16 | KvLong | key 3 bytes, len u8, value |
| 17 | KvLong | key 3 bytes, len u16, value |
| 18, 23 | Data | len u8, value |
| 1B 00 | Kv | key u8, 4 bytes |
| 19–1D | KvLong | klen u8, key, then a fixed-size value of (op==19)+2×(op−19) bytes |
| 1E | KvLong | klen u8, key, len u8, value |
| 1F | KvLong | klen u8, key, len u16, value |
| 20 | Push | 1 byte, or `FE` + 8 bytes |
| 28 | Push | 2 bytes |
| 30 | Push | 3 bytes |
| 38 | Push | len u8, bytes |
| 3D, 40 | Pop | — |
| 80 | Marker | — : the current path exists as a node (46,642 in the templates, always right after a push). Sorts before the node's contents |
| 88 | no-op | — (never seen) |

**Path integers.** 1 byte: value. 2 bytes: `0x80 + ((b0 & 0x7F) << 8) + b1`.
3 bytes: `0x80 + (b1 << 8) + b2`.

## 3. Text ✅

Text values are XOR-ed with `0x5A`, then encoded with
[SCSU](https://www.unicode.org/reports/tr6/). CR (`\r`) is FileMaker's
paragraph separator (¶) and is kept as is.

## 4. Path map

Verified by matching the object UUIDs of a *Save a Copy as XML* export with
the binary (`tools/oracle/uuidmap.py`). `T` is a base-table id (≥ 128).

Every catalog follows the same pattern:

* `[cat].[1]`: catalog header: key 0 = last id ever used, key 4 = number
  of objects (u32), key 216 = catalog UUID, key 252 = change counter
  (length-prefixed integer, +1 per change); `[cat].[1].[1]` = name index
  (collation key of the name → object reference, §14); `[cat].[1].[3]` =
  display order (u32 position 1…n → object reference, renumbered when an
  object is deleted). An object reference is a length + path integer
  (`02 80 02` = 130), or `03 D0 hi lo` for table occurrences;
* `[cat].[5].[id]`: one object per id. Common keys: **16** name,
  **216** UUID, **217** source UUID, **31873** user name,
  **31874** account name, **31875** timestamp (modification metadata);
* `[cat].[5].[id].[5].[n]`: child objects (script steps, layout objects).

| Path | XML catalog | Status |
|------|-------------|--------|
| `[2]` | file-level settings; key 13 = locale (§13), key 134 = data formats (§14) | 🔍 |
| `[3].[16]` | BaseTableCatalog; `[3].[16].[5].[T]` key 251 = change counter (§13) | ✅ read, written (§14) |
| `[3].[17]` | TableOccurrenceCatalog; `[3].[17].[1].[14].[T]` = evaluation plan cache (§7) | ✅ read, written (§14) |
| `[3].[251]` | RelationshipCatalog (251 spelled as one byte `FB`, §7) | ✅ read, written (§14) |
| `[4]` | LayoutCatalog; `[4].[5].[L].[5].[n]` = LayoutObject | 🔍 |
| `[6]` | ThemeCatalog (§12) | ✅ styles |
| `[25]` | font catalog (§12) | ✅ names |
| `[16]` | CustomFunctionsCatalog (§16) | ✅ read, written |
| `[17]` | ScriptCatalog; `[17].[5].[S].[5].[n]` = Step | 🔍 |
| `[23].[1]` | AccountsCatalog (§15) | ✅ read, written |
| `[23].[2]` | PrivilegeSetsCatalog (§15) | ✅ read, written |
| `[23].[3]` | ExtendedPrivilegesCatalog (§15) | ✅ read, written |
| `[23].[4]` | FileAccessCatalog; `[23].[4].[1]` key 16 enters the account digests (§15) | 🔍 |
| `[33]` | ValueListCatalog (§9) | ✅ read, written (§14) |
| `[38]` | BaseDirectoryCatalog | 🔍 |
| `[65]` | CustomMenuCatalog | 🔍 |
| `[T].[3].[5].[F]` | FieldCatalog of table T | ✅ names |
| `[T].[3].[1]` | field catalog header: name index `[1]` (collation keys, §14), order `[3]`, type index `[6]`, dependencies `[14]` (§5) | ✅ |
| `[T]` key 0 | last record id ever used (§13) | ✅ |
| `[T].[1].[F]` | index of field F's values (§13) | ✅ |
| `[T].[5].[R]` | record R: key F = value of field F (§13) | ✅ |
| `[T].[8]` | values of the table's global fields, keyed like a record's (§13) | ✅ |
| `[T].[5].[R].[F]` | long value of field F: key 0 = sizes, keys 1…n = chunks (§13) | ✅ |
| `[T].[13]` | set of record ids (§13) | ✅ |
| `[TO].[7]` | per-table-occurrence record list cache (`D0 hi lo` element, §13) | 🔍 |
| `[node].[16]` | long name split over keys 1…n when it does not fit inline | ✅ |

## 5. Field definitions ✅

`[T].[3].[5].[F]`: key 16 = name, key 3 = comment, key 218 = tag list,
key 2 = a 26-byte **option block**. Sub-path `.[5]` holds the compiled
auto-enter calculation (§6).

Decoded by flipping single bits in copies of real files and letting
FileMaker export XML. All 214 template fields match the export (`tests/oracle.rs`).

| Byte | Bits | Meaning |
|-----:|------|---------|
| 0 | — | field type: 1 Normal, 2 Calculated, 3 Summary |
| 1 | — | data type: 1 Text, 2 Number, 3 Date, 4 Time, 5 Timestamp, 6 Container |
| 3 | — | auto-entered stamp: 0 Date, 1 Time, 2 Timestamp, 3 Name, 4 Account name; +5 = modification instead of creation |
| 4–7 | — | index language id, u32 BE (21 = English; Unicode variants set high bits) |
| 8 | 0 | shadow field (external SQL source) |
| 8 | 3, 7 | index Minimal |
| 8 | 5 | index All |
| 8 | 6 | automatic indexing **off** |
| 9 | 0 | global storage |
| 9 | 1 | calculation result **not** stored |
| 10 | 0 | prohibit modification of auto-entered value |
| 10 | 1 | serial number, generated on commit |
| 10 | 2 | looked-up value |
| 11 | 0 | stamp of byte 3 enabled (needed for value 0) |
| 11 | 1 | serial number, generated on creation |
| 11 | 2 | constant data |
| 11 | 3 | calculated value |
| 11 | 4 | value from last visited record |
| 11 | 5 | always evaluate (do not stop when referenced fields are empty) |
| 11 | 6 | furigana |
| 11 | 7 | replace existing value |
| 14 | 0 | validate against a value list |
| 14 | 1 | maximum size |
| 14 | 2 | validate always (not only during data entry) |
| 14 | 4, 5, 6 | strict data type: numeric, four-digit year, time |
| 14 | 7 | custom error message (set with 15.7) |
| 15 | 0 | validation by calculation |
| 15 | 1 | `alwaysValidate` (validate even when not modified) |
| 15 | 2 | user **cannot** override |
| 15 | 3 | not empty |
| 15 | 4 | unique |
| 15 | 5 | existing value |
| 15 | 6 | range |
| 15 | 7 | custom message |
| 16–19 | — | serial number increment, u32 BE (1 in every field FileMaker creates) |
| 20–23 | — | maximum number of characters of 14.1, u32 BE |
| 24–25 | — | maximum repetitions, u16 BE |

Bytes 2 and 12–13 changed nothing visible in the XML. Index language: the
file's language (`[2]` key 13, §13) for new fields. Bytes 16–23 were read
from fields set in FileMaker's Field Options dialog (increment 3, maximum
12 characters).

**Summary fields** keep their definition in key 9 of the field node, e.g.
`01 00 02 01 00 00 FE 7F 02 01 05 BE BF EF 00 01 02 01 00 00 FF 1F 02 01 00 00 BF EF 00 01`:

| Byte | Meaning |
|-----:|---------|
| 0 | operation: 1 Total, 2 Average, 3 Count, 4 Minimum, 5 Maximum, 6 Standard Deviation, 7 Fraction of Total, 9 List (all checked in FileMaker's Table view; 8 shows a count and is not in the dialog, 10 and above show nothing) |
| 1 | options: 0x04 running total or count, 0x10 restart for each sorted group, 0x01 the dialog's other option: weighted average, fraction subtotaled (both with a second field), standard deviation by population (none); 0x04 on a standard deviation gives changing, meaningless values; 0x02, 0x08, 0x20, 0x40, 0x80 change nothing |
| 2–15 | the summarized field, as a 14-byte reference (below) |
| 16–29 | the second field: the sort field a running summary restarts on, the field a Fraction of Total is subtotaled by (both on the table's occurrence), the weight of a weighted Average (no occurrence); `02 01 00 00 00 00 02 01 00 00 00 00 00 01` when unset |

A **14-byte field reference** (also in lookups below): a 5-byte
occurrence slot (`04 03 D0 hi lo`, or `02 01 00` and two bytes of
padding), a byte (`7F` in second references and lookups, `00` for the
summarized field), a 4-byte field slot (`len 01 path-int`, padded), two
bytes, and the repetition (u16). The padding holds leftovers in older
files (`FE 7F`, `BE BF EF`); FileMaker 26 writes zeros. The XML names
the options RunningTotal, RunningCount, WeightedAverage,
FractionalSubtotal and `restartEachGroup`.

What FileMaker shows (Table view of `tools/oracle/summary_demo.sh`): a
running summary in a record is the value up to that record in the found
set's order, restarting at each group of the second field only when the
found set is sorted by it; in a sub-summary (`GetSummary`) it is the value
at the group's last record. A fraction of total is empty for a record
with no value, and a subtotaled one is empty unless the found set is
sorted by its field (then: divided by the group's total). A weighted
average is Σ(value × weight) / Σ weight.

**Lookups** (option bit 10.2) keep their definition in key 8:

| Byte | Meaning |
|-----:|---------|
| 0 | if no match: 0 do not copy, 1 copy the next lower value, 2 the next higher, 3 the constant of key 11 |
| 1 | 1 = copy an empty value too (the "Do not copy contents if empty" box unticked) |
| 2–15 | the field copied, as a 14-byte reference through the related occurrence |
| 16–20 | the occurrence the lookup starts from ("Starting with table"), `len 03 D0 hi lo` |
| 21 | `00` |

FileMaker fires a lookup only when the field catalog's dependencies hold
an empty node `[T].[3].[1].[14].[251].[F].[start TO].[related TO]` (251
spelled `FB`): without it the definition shows in Manage Database and in
the XML, but nothing is ever copied. A lookup has no `[FFF0]` entry.

Other validation keys: **7** "member of value list": `len 01 path-int`
of the value list, then `00 90 01 02 01 00 00 71 D1 FF` (FileMaker 26's
bytes, not decoded); **24** the custom message as the text of its
calculation (FileMaker 26 only offers a calculation, e.g. `"Bad value"`),
compiled in sub-node **`[32]`** (keys 4, 5, 6).

Other keys of a field node: **12** next serial value (text; FileMaker
writes `1` in every field), **11** lookup constant (text), **13**
auto-enter constant (text), **33**/**34** validation range low/high
(text), **251** and **252** counters, **217** source UUID, **218** tags;
sub-node **`[5]`** the calculation of a calculated field or the auto-enter
calculation, **`[25]`** the validation calculation (both keys 4, 5, 6 as
in §11, key 4 = the table's occurrence).

The field catalog header `[T].[3].[1]` also holds:

* `[6]`: a second name index by type: key = field type, data type × 4
  (+1 when global), the name's collation key without its tertiary part
  (§14); names that differ only there share one entry;
* `[14].[FFF0].[F]`: what field *F* depends on: key *G* = 01 for each field
  of the table its calculation uses (00 for the field a summary summarizes),
  key FF00 (32640) = 01 for any auto-enter, FF02 (32642) = 01 for a
  modification stamp; a stored calculation that uses no field of its
  table (a constant, custom functions, `Get`) has FF00 = 00 and FF01
  (32641) = 00, without which FileMaker never computes it; for a summary with a second field, that field
  = 02 (a weight), or the byte-string key `00 TOref len field` = 02 (a
  sort or subtotal field on the table's occurrence); `[14].[FFF1].[F]`:
  fields its validation calculation uses; `[14].[251].[F]`: lookups
  (above). FileMaker does not rebuild these.

## 6. Long values

A value longer than ~1000 bytes is moved from key *K* of a node into the
sub-path `[node].[K]`, as keys 1…n (key 0 = total length as u32 BE, or
formatting for text). Short values stay inline. Readers must accept both.

## 7. Relationship graph ✅

* Table-occurrence reference: `03 D0 hi lo`, id = 128 + hi:lo. FileMaker's
  XML shows `0x104000 | id`.
* `[3].[17].[5].[TO]` key 2: `02 01 00 | len 02 path-int(base table) |
  f32 left, top, width, height (BE) | u32 collapsed height | u32 | RGBA`.
  The node's path element is `D0 hi lo`. A table occurrence made with
  Manage Database › Tables has position and size 0 (FileMaker places it
  when the graph is shown) and colour `77 77 77 FF`.
* `[3].[17].[1].[14].[T]`: per base table, an evaluation plan of its
  auto-enter, calculated and summary fields (`[FF]` field → rank,
  `[FF00]` rank → kind and field, `[FF02]` modification stamps, `[FFFB]`
  summaries, `[FFFC]` fields to validate, `[G]` fields depending on G).
  It is a cache: FileMaker rebuilds it when it is missing.
* `[3].[251]`, with 251 spelled as one byte `FB` (not `80 7B`):
  `[1]` catalog header (key 0 = last id as a length-prefixed integer, 4
  = count, 216 UUID, 252 counter; no name index or display order), node
  `[5].[R]` per relationship (key 2, predicates in `[3]`, 216 UUID). A
  file that never had a relationship (a new file) has no `[3].[251]`.
* `[3].[251].[5].[R]` key 2: `04 TOref(left) 04 TOref(right) | right
  side's sort order | left side's sort order | 00 00 00 | flags`. A sort
  order is a u32 count, then per key `len TOref | len 01 field | 00 01 |
  02 01 00 | 02 01 00 | 00 01 | value list (00 = none) | order (0
  ascending, 1 descending) | 00000000`, the field being one of that
  side's occurrence. Flags: 0x01 delete the right side's records with
  the left one, 0x02 the left side's, 0x10 allow creating right records,
  0x20 left records, 0x40 right side sorted, 0x80 left side sorted.
* `[R].[3]` keys 1…n = join predicates `op | len 01 field | len 01 field`,
  op 0 `=`, 1 `≠`, 2 `<`, 3 `≤`, 4 `>`, 5 `≥`, 6 `×` (XML Equal,
  NotEqual, Less, LessOrEqual, Greater, GreaterOrEqual, CartesianProduct).
* `[3].[17].[5].[TO].[251]` (`FB`): the occurrence's links, one per
  relationship: 3-byte key `D0 hi lo` (the other occurrence) → the
  relationship id as a length-prefixed integer (op `12`: `12 D0 00 03
  01 03`).
* `[3].[17].[1].[8].[TO]`: routes. For every occurrence reachable from
  TO: `D0 hi lo` → the id of the first relationship on the way (the
  graph has no cycles, so the path is unique). The `[8]` node exists in
  new files.
* FileMaker refuses a second relationship between the same occurrences
  (it adds predicates to the first instead) and a cycle (another
  occurrence is needed); records can be created only through `=`
  predicates.

## 8. Scripts ✅

`[17].[5].[S]`: key 16 = name, key 240 = options (u64: 8 script, 9 folder,
2 separator), key 4 = step list made of 28-byte records. A list longer than
1000 bytes (36+ steps) moves to keys 1…n of `[S].[4]` (key 0 = length, §6).
The trailing zeros of the last record are trimmed.

### Step record

| Bytes | Meaning |
|------:|---------|
| 0–3 | `02 01 n 00`: an id slot (length, then the path integer's length and the path integer *n*), the step's parameter node `[S].[5].[n]`; from step 128 on `03 02 80 xx` |
| 4–5 | uninitialised (`BE BE` in the templates, anything in new files) |
| 6–19 | inline references (below); bytes between them are uninitialised |
| 20–21 | step id, i16 BE; **negative = disabled** (`FF A3` = disabled Beep) |
| 22–23 | u16: index (0-based) of the step closing the block for If, Else If, Else, Loop, Exit Loop If (End Loop: its Loop); destination kind for Go to Layout (1 original, 3 name by calculation, 4 number by calculation, 5 layout) and Go to Record/Request/Page (1 first, 2 last, 3 previous, 4 next, 5 by calculation) |
| 24–27 | u32 options = XML `<Options>` |

Inline references are length-prefixed: an object id is the length of its
path integer, then the path integer (`02 01 0B` = 11, `03 02 80 00` =
128), a table occurrence is `03 D0 hi lo` (§7). A
layout (Go to Layout, New Window) or script (Perform Script) sits at byte
6. A field (Set Field, Go to Field, Insert Text, Show Custom Dialog input 1)
is TO at 6, field at 12, repetition u16 at 18:
`04 03D00004 · 02 01 0B · · · 00 01` = Assignees::PrimaryKey.

Option bits (XML shows them as `<Boolean id=…>`): `0x4000` the step holds a
calculation; `0x80` "With dialog: Off"; `0x1000` select (Go to Field, Insert
Text); `0x2000` exit after last (Go to Record); `0x100`/`0x200` skip
validation / force commit (Commit), flush join results / external data
(Refresh Window); `0x2_0000` On (Set Error Capture, Allow User Abort);
`0x100_0000` pause (Enter Find/Browse Mode); `0x200_0000` restore (Enter
Find Mode, Perform Find, Sort Records); `0x40` Perform Script from list.

### Parameters `[S].[5].[n]`

Parameter *p* (XML `position`) is node `[128 + p]`: text in key 1, a byte
in key 3, a calculation in `.[5]` (keys 4 context `01 00`, 5 token stream,
6 compiled; §11). Calculations in scripts have no TO context, so field
references always show as `TO::Field`. Keys 216/217 of the node are the
step's UUID and source UUID.

| Step (id) | Storage |
|-----------|---------|
| # comment (89) | `[128]` k1 text |
| Set Variable (141) | `[128]` k1 name (`$x`, `$$g`), `[129].[5]` value, `[130].[5]` repetition |
| Set Field (76) | inline field, `[128].[5]` value; repetition calculation at position 10 (`[138].[5]`, not observed stored) |
| Insert Text (61) | `[128]` k1 text; inline field, or `[160]` k1 target variable |
| Insert Calculated Result (77), Insert Current Date/Time/User Name (13, 14, 60), Paste (48), Relookup Field Contents (40), Set Field By Name (147) | not observed: the script engine reads them like Set Field and Insert Text (inline field, `[160]` k1 variable, calculations at `[128]`, `[129]`…; Set Field By Name: target name, then value) |
| Replace Field Contents (91) | inline field; `[129]` k3 = with (2 serial numbers, 3 calculated result; current contents otherwise), `[128].[5]` calculation, `[130]` k3 increment, `[131]` k1 initial value (text), `[132]` k3 u8 update serial number in Entry Options, `[133]` k3 u8 use the field's Entry Options, `[134]` k3 u8 perform auto-enter (seen in FileMaker 26: clipboard XML `<With>`, `<SerialNumbers>`) |
| Perform Find/Replace (128) | `[128]` k3 operation (0 Find Next, 1 Replace & Find, 2 Replace, 3 Replace All), `[128].[5]` find text; `[129]` k3 direction (0 forward, 1 backward, 2 all), `[129].[5]` replace text; `[130]` k3 across (0 all records, 1 current record), `[131]` k3 within (0 all fields, 1 current field), `[132]` k3 u8 match case, `[133]` k3 u8 whole word (seen in FileMaker 26: XML `<FindReplaceOperation type direction AcrossOptions WithinOptions MatchCase MatchWholeWords>`) |
| If (68), Else If (125), Exit Loop If (72) | `[128].[5]` condition; Else If also `[128]` k3 = u32 index of the End If |
| Loop (71) | node key 129 = flush (1 always) |
| Perform Script (1) | inline script, `[128].[5]` parameter |
| Exit Script (103) | `[128].[5]` result |
| Go to Layout (6) | inline layout or `[128].[5]` name/number; node key 134 = animation |
| Go to Record/Request/Page (16) | bytes 22–23 kind, `[128].[5]` number; `[128]` k3 = `FFFFFFFF` |
| Go to Object (145) | `[128].[5]` object name, `[129].[5]` repetition |
| Show Custom Dialog (87) | `[128].[5]` title, `[129].[5]` message, `[130].[5]` label of input 1 (inputs 2–3 not observed); buttons 1–3: `[133…135].[5]` label, `[131…133]` k3 commit (1); `[128…130]` k1 = labels as plain text |
| Omit Multiple Records (26) | `[128].[5]` count |
| Sort Records (39) | node key 10 = sort specification |
| Enter Find Mode (22), Perform Find (28) | `[6].[6]` stored requests |

**Sort specification**: u32 count, then per key: TO, field (length-prefixed),
u16 repetition, two length-prefixed references (`01 00`, not decoded), u16,
value list (length-prefixed, empty if none: a custom order), order byte
(0 ascending, 1 descending), u32 (0).

**Find requests**: `[6].[6]` key 0 = count (`01 02`); request *r* is
`[6].[6].[r]`, with key 0 (empty, op `00 00`) when it omits, and one
byte-string key per criterion: `00 | TO ref | 01 field` → criterion text.
`[6].[5]` holds a value not decoded (`01`, `00000006`).

Strings in parameter key 1 follow the record-value rule (§13): inline
when one byte per UTF-16 unit and at most 1000 bytes, else in
`[128 + p].[1]` (key 0 = byte and unit counts, keys 1… = chunks). Path
elements above 127 are spelled `80 00` or `00 00` depending on the file
(both read 128): a writer keeps the spelling it read. Key 134 is written
as a 2-byte path integer (`0B 00 06`), keys 216/217 as one byte.

More option bits, from steps FileMaker 26 wrote: `0x1` an inline field
(also set for a variable target: Insert Text writes slots `01 00` and
`01 20` with repetition 1), `0x4` a text parameter (comment with text,
Set Variable, Show Custom Dialog), `0x8` an inline layout, `0x2` set by
FileMaker 26 on every Go to Layout (older templates: `0x8` alone).
Show Custom Dialog writes the commit flag `[131 + i]` k3 only for the
buttons it has. A new Perform Find writes `[6].[5]` key 0 = `00000006`.

### Writing (`fmp12::scripts_save`) ✅ (existing scripts)

Checked in FileMaker 26 (`tools/oracle/script_check.ps1`, example
`scriptedit`): edited Set Field calculations, appended Show Custom
Dialog, disabled steps, If/Else/End If, Set Variable (with repetition),
Go to Layout, Perform Script with a parameter and a calculation with a
multi-binding `Let`, a nested `Let`, `$v[2]`, `Case`, `Choose` and `List`
open without warning, read back in Save a Copy as XML exactly as written
and evaluate (the dialog shows `21|two|yes|two|x¶y`).

* Step list: records of 28 bytes; FileMaker 26 writes the last one whole
  (older files trim its trailing zeros: the writer keeps the length it
  read). **FileMaker finds a step's parameters by the step's position**:
  step *i* (0-based) uses node `[S].[5].[i + 1]` and bytes 0–3 say so;
  `[S].[5]` key 0 = number of steps. A step whose parameters sit under
  another number shows none. Bytes 4–5 are `00 00` in new steps.
* Parameter nodes are spelled `00 xx` (= 128 + xx, e.g. `00 00` for
  parameter 0) in every template and in what FileMaker 26 writes, also
  for button parameters and trigger nodes; FileMaker does not find a
  parameter spelled `80 xx`.
* Bytes 22–23 of control steps: If and Else If point at the next branch
  (Else If / Else) or the End If, Else at the End If, Loop and Exit Loop
  If at the End Loop, End Loop at its Loop, End If 0 (checked on every
  template script).
* Every node has a node marker; every step node has key 216 (UUID; new
  steps written by FileMaker 26 have no 217).
* Calculations: keys 4 (`01 00`), 5 (source tokens), 6 (prefix form, §11).
* A new script (as FileMaker 26 writes one): `[17].[1].[7].[S]` key 16
  name (key 240 only when not 8), a position in `[17].[1].[3]` (u32
  position → len-int id: the display order), a name index entry in
  `[17].[1].[1]` (collation key → len-int id), `[17].[1]` key 0 = last
  id and key 4 = count; `[17].[5].[S]` keys 16, 19 (`00000000`), 216,
  252 (`01 01`) and 31873–31875 (account and timestamp, copied from
  `[17].[1]`); `[17].[1]` key 252 (change counter) is bumped.
* **`[17].[1]` key 8** (16 bytes) is an **integrity digest of the script
  catalog**, checked when the file opens: when it does not match,
  FileMaker refuses the file with "the file's access privileges have been
  damaged or altered". It is **plain, unkeyed MD5**: for each member
  `[17].[1].[7].[S]` in id order (key order, not the display order of
  `[17].[1].[3]`; folders, their `--` closers and separators count like
  scripts), feed 8 bytes of "private flags" (big-endian, zero; no member
  in any file we have carries a key for them), then the name (key 16, or
  the chunks of `[S].[16]` for names that are not one byte per UTF-16
  unit) in UTF-16BE. No count, separator or terminator. A catalog with no
  scripts has no key 8. `fmp12::scripts_save::catalog_digest` computes it;
  `save_scripts` rewrites it whenever scripts are added, deleted or
  renamed. It depends on names and ids only, not on steps, UUIDs or the
  file: one script "Nouveau script" gives `786c087f…62da` in a blank file
  and in an emptied Tasks.

  *How it was confirmed (Nov 2026).* The algorithm was suggested by the
  public format notes of the OpenMaker project
  (github.com/OpenMaker-Dev/openmaker, `docs/re/scripts.md`); no code or
  text was taken from it, and we tested it on our own files only. It reproduces
  key 8 of all 66 installed starter templates (every language, including
  ja/ko/zh and de names stored in the long form) and of all 395 files in
  our oracle and sample sets that have a key 8 (75 distinct values); six
  of them list scripts in a display order that differs from id order, and
  only id order matches. The only misses were 13 files BaseCraft itself
  had written with a stale or experimental key 8. FileMaker 26 then opened, with
  no warning, a copy of Tasks with two scripts added, one renamed and one
  deleted (the new BC Calc ran from Task List's OnLayoutEnter, through
  "Sort by Status"), a copy of Tasks with Latin, CJK, German and Cyrillic
  names, and a `basecraft-cli new-file` file with scripts added, renamed
  and deleted (the new script ran from the Scripts menu); the XML exports
  and the Script Workspace list the new names (`tools/oracle/script_check.ps1`).

  An earlier reading of the binary took the `DBLibrary` CRC machinery for
  a digest seeded by the privilege set, and BaseCraft left the script list
  alone. That was wrong: for the scripts catalog there is no seed and no
  secret, and the same unkeyed MD5 reproduces every file we have. The
  accounts and privilege catalogs (`[23]`) carry their own MD5 digests,
  one per member (§15).
* **A file's first script**: FileMaker writes node markers for
  `[17].[1].[1]` (name index) and `[17].[1].[7]` (catalog members) when
  the first script is created; a file with entries under them but no
  marker is reported damaged by FileMaker Server's consistency check
  (error 805, `tools/oracle/fms_check.sh`). `save_scripts` adds them
  (test `first_script_of_a_blank_file_marks_its_nodes` on the blank asset).
* Scripts of 128 steps or more: seen in a 140-step script FileMaker 26
  built (140 Set Variable steps pasted into a copy of Tasks). Step node
  128 is spelled `80 00` (parameters under it `00 xx` as usual), the step
  record starts `03 02 80 00` (an id slot whose path integer has 2
  bytes), and `[S].[5]` key 0, the step count, is an id too: `02 80 0C`
  = 140 (`01 xx` below 128, which reads like a len-int). BaseCraft wrote
  `03 01 80 00` and `01 8C`; FileMaker then showed no parameters from
  step 129 on. `step_entries` now writes FileMaker's bytes exactly
  (test `scripts_of_128_steps_and_more_keep_their_parameters`; a
  140-step script BaseCraft wrote into the blank file reads back in
  FileMaker Pro 26's Save a Copy as XML with every name and value).
  Script, layout and other ids of 128 and more in catalogs
  (`[17].[1].[3]`, `[17].[1]` key 0) are still written as len-ints: not
  seen in a FileMaker file yet.
* Collation keys of the name index: 2-byte primary weights, case folded,
  then `00 00 00`; accented letters add secondary weights after `00 00`.
  Known weights: space `020A`, `-` `0223`, `'` `02EE`, a `120F` b `1225`
  c `123D` d `1250` e `126B` f `12A3` g `12B0` h `12D3` i `12EC` j `1305`
  k `131E` l `1330` m `135F` n `136D` o `138E` p `13B3` q `13C8` r `13DA`
  s `1410` t `1433` u `1453` v `147B` w `148D` y `149C` z `14AD`. Names
  with other characters get no index entry (ponytail).
* Folders: options 9 (bit 0) open a folder, a "--" entry with options 2
  or 10 (bit 1, XML `isFolder="Marker"`) closes it; the list is flat in
  `[17].[1].[3]` order.

### Script triggers (`fmp12::triggers`) ✅

Decoded by setting every trigger in FileMaker 26 (Layout Setup, Format ›
Set Script Triggers, File Options) on copies of Tasks and diffing; the
XML export of those files matches (`tests/triggers.rs`).

A layout's triggers are in `[4].[5].[L].[48]`, an object's in
`[4].[5].[L].[5].[obj].[48]`: key 4 = u16 number of triggers, then one
node `[48].[5].[128 + event]` per trigger (spelled `00 xx`). Key 1 is
`u16 event | len 01 script | 00 | u32 modes | 6 zero bytes` (older files
lack the `00` slot); the parameter is a calculation in
`[128 + event].[49]` (keys 4, 5, 6). Modes: 1 Browse, 2 Find, 4 Preview.
FileMaker also changes field 3 of the layout's protobuf `[4].[5].[L].[7]`
key 1 when a trigger is added (not decoded).

Event ids (XML `ScriptTrigger id`): objects 1 OnObjectEnter, 2
OnObjectExit, 3 OnObjectModify, 4 OnObjectSave, 5 OnObjectKeystroke, 6
OnObjectValidate, 7 OnPanelSwitch (by elimination), 8
OnObjectAVPlayerChange; layouts 101 OnLayoutEnter, 102
OnLayoutKeystroke, 103 OnRecordLoad, 104 OnRecordCommit, 105
OnRecordRevert, 106 OnModeEnter, 107 OnModeExit, 109 OnLayoutExit, 110
OnViewChange, 111 OnLayoutSizeChange, 112 OnGestureTap, 113
OnExternalCommandReceived; file 201 OnFirstWindowOpen, 202
OnLastWindowClose, 204 OnWindowOpen, 205 OnWindowClose, 206
OnFileAVPlayerChange, 209 OnWindowTransaction.

File triggers are script slots (`len 01 script`, `00` when empty) of the
file options record `[2]` key 24: u32, two bytes, OnFirstWindowOpen,
OnLastWindowClose, the layout shown on open, u16, u32, u16, two bytes,
OnWindowOpen, OnWindowClose, u32, OnFileAVPlayerChange, u16, u16,
OnWindowTransaction (absent in older files), `00`. Where a file
trigger's parameter goes was not seen (the UI driver found no parameter
box in that dialog).
### Button actions

A layout object's field 16 is the button. Its field 3 is the step as a
message {1 step id, 2 bytes 22–23, 3 options, 4 the 14 inline bytes};
step 0 means no action. A script button is a Perform Script step (id 1,
options `0x40`). The step's parameters are under
`[4].[5].[L].[5].[obj].[17]`, laid out like `[S].[5].[n]`.

### Step catalogue

`docs/reference/script_steps.txt` lists the 222 step ids FileMaker Pro 26
names in its XML (1–247). Built with the reverse oracle
(`tools/oracle/step_ids.py`): the step ids of a copy of Tasks were
overwritten with 0…365 and the copy exported as XML, six exports of 61
steps each.

Parameters were decoded from the templates and from a script of 57 steps
pasted into a copy of Tasks through the clipboard
(`tools/oracle/step_params.xml`, `Add-FmScriptSteps` in `fmui.ps1`), then
checked against the XML export (`tests/oracle.rs`,
`step_parameters_match_xml`).

### Parameters of the M5 steps

Decoded from 102 steps pasted into a copy of Tasks
(`tools/oracle/step_params_m5.xml`, plus a second paste of variants) and
read back with `fmdump <file> raw` and `tools/oracle/steps_dump.py`
(one line per step: id, bytes 22–23, options, inline bytes, parameter
keys). Positions are parameter nodes `[128 + p]`; a calculation is at
`[128 + p].[5]` key 5. FileMaker's import log names the XML elements it
did not know; those steps were left out below.

| Step | Bytes 22–23 (`arg`) | Options | Parameters |
|------|---------------------|---------|------------|
| Perform Script by name | 0 | `0x2004800` | [128] parameter, [130] name calculation |
| Install OnTimer Script | 0 | `0x4040` | inline script; [128] parameter, [129] interval |
| Set Error Logging | 0 | `0x2_0000` On, `0x1_0000` always, `0x4000` with a calculation | [128] custom debug information |
| Set Layout Object Animation, Allow User Abort | 0 | `0x2_0000` On | — |
| Go to Portal Row | 1 first, 2 last, 3 previous, 4 next, 5 by calculation | `0x1000` select, `0x2000` exit after last, `0x80` no dialog | [128] row calculation; [128] key 3 `ffffffff` |
| Go to Related Record | 1 current layout, 5 layout (inline) | `0x100` show only related records, `0x200` match all records of the found set, `0x2_0000` new window, `0x200_0000` use external table's layouts, `0x8` with a layout | [128] key 17 the related occurrence (`03 D0 hi lo`); new window: [128] key 3 style, [128] name, [129] height, [130] width, [131] top, [132] left |
| Cut, Copy, Paste, Clear | 0 | `0x1000` select entire contents, `0x200` Paste without style | inline field (none: the active field; Copy with none copies the record) |
| Set Selection | 0 | `0x4000` | inline field; [128] start, [129] end |
| Undo/Redo | 1 undo, 2 redo, 3 toggle | — | — |
| Set Next Serial Value | 0 | `0x4000` | inline field; [128] value |
| Insert from Index, Insert from Last Visited | 0 | `0x1000` select | inline field |
| Export Field Contents | 0 | `0x200` create folders | inline field; [128] key 130 path list |
| Truncate Table | 1 a table (inline `03 02 <path int>`), 0 none | `0x80` no dialog | — |
| Constrain / Extend Found Set | 0 | `0x200_0000` restore, `0x8000` | stored requests at `[6]`, as Perform Find |
| Perform Quick Find | 0 | `0x4000` | [128] text |
| New Window | 1 current layout, 5 layout (inline) | `0x4000` | [128] key 3 style; [128] name, [129] height, [130] width, [131] top, [132] left |
| Select / Close Window | 1 by name, 0 current | `0x8000_0000` current file only | [128] name |
| Move/Resize Window | 1 by name, 0 current | as Close Window | [128] name, [129] height, [130] width, [131] top, [132] left |
| Set Window Title | 1 by name, 0 current | `0x4000` | [128] window name, [129] new title |
| Adjust Window | 1 resize to fit, 2 maximize, 3 minimize, 4 restore, 5 hide | — | — |
| Arrange All Windows | 1 tile horizontally, 2 tile vertically, 3 cascade, 4 bring all to front | — | — |
| Scroll Window | 1 home, 2 end, 3 page up, 4 page down, 5 to selection | — | — |
| Show/Hide Toolbars, Menubar, Text Ruler | 1 show, 2 hide, 3 toggle | `0x8_0000` lock | — |
| View As | 1 form, 2 list, 3 table, 4 cycle | — | — |
| Set Zoom Level | 1–8: 25, 50, 75, 100, 150, 200, 300, 400 %; 9 zoom in, 10 zoom out | `0x8_0000` lock | — |
| Open File, Close File | 0 | `0x100` hidden | key 32 file reference |
| Save a Copy as | 1 copy, 2 compacted, 3 clone, 4 self-contained | `0x2000_0000` automatically open | [128] key 130 path list |
| Print, Print Setup, Import / Export Records | (Import: 1) | `0x80` no dialog | — |
| Save Records as PDF / Excel | (PDF: 1 records being browsed) | `0x80` no dialog, `0x200` create folders | [128] key 130 path list |
| Open URL | 0 | `0x80` no dialog | [128] URL |
| Send Mail (client) | 0 | `0x80` no dialog | [128] To, [129] CC, [130] BCC, [131] subject, [132] message |
| Get Folder Path | 0 | `0x100` allow folder creation | [128] key 1 the variable name; [131] dialog title, [132] default location |
| Speak, Dial Phone, Set Web Viewer | (Set Web Viewer: 5 go to URL) | `0x80` no dialog | [128] text / number / object name, [129] URL |

The path list ([128] key 130, `script::files::decode_path_list`): a u32
count, then per path a u8-prefixed kind (`file`; empty for a variable
such as `$path`) and a u16-prefixed path (`out.txt`): only the two
strings are XOR 0x5A, the counts are plain (FileMaker 26 writes
`00000001 04 <file> 0005 <a.txt>` for `file:a.txt`). FileMaker writes
key 130 with a one-byte key (`06 82 len`): spelled `0E 00 02` (the 2-byte
form of key 134) the export shows an empty path list.

Not decoded (FileMaker 26's import refused the XML names tried): Find
Matching Records' replace / constrain / extend choice, Sort Records by
Field's order.

### Data file, PDF and other steps

Decoded by writing candidate parameters with BaseCraft (example
`bare_steps`), reading them back in FileMaker Server's XML export
(`tools/oracle/fms_check.sh`, headless) and running them in FileMaker Pro
26 (`tools/oracle/datafiles`: a script that stores `Get ( LastError )` and
the results in `Tasks::Description`, read through UI Automation); the XML
element names were confirmed by pasting into FileMaker (its import log
names unknown elements: `<position>` for Set Data File Position, `<Count>`
for Read from Data File; `<UniversalPathList>`, `<Field>$var</Field>` for
targets).

| Step | Options | Bytes 22–23 | Parameters |
|------|---------|-------------|------------|
| Get File Exists (188), Get File Size (189), Open Data File (191) | `0x20` path list, `0x5` variable target (`0x1` field) | 0 | [128] key 130 path list; target: [160] key 1 variable (inline slots `01 00`, `01 20`, repetition 1) or the inline field |
| Create Data File (190), Delete File (197) | `0x20`, `0x200` create folders | 0 | [128] key 130 |
| Rename File (199) | `0x4020` | 0 | [128] key 130 the file; [128].[5] the new name |
| Write to Data File (192) | `0x4000`, `0x200` append line feed, target as above (the data source) | 1 UTF-16, 2 UTF-8, 3 bytes (0: as 1) | [128].[5] file id; data source in [160] or the inline field |
| Read from Data File (193) | `0x4000`, `0x8000` with a count, target | 1 UTF-16, 2 UTF-8, 3 bytes (0: as 3) | [128].[5] file id, [129].[5] count |
| Get Data File Position (194) | `0x4000`, target | 0 | [128].[5] file id |
| Set Data File Position (195) | `0xC000` | 0 | [128].[5] file id, [129].[5] position |
| Close Data File (196) | `0x4000` | 0 | [128].[5] file id |
| Go to List of Records (228) | `0x4000` (needed), `0x8` layout, `0x2` | 1 current layout, 5 layout (inline) | [135].[5] (parameter 7) the record ids |
| Configure Persistent Data (238) | `0x4`, `0x4000`, `0x8000` | 0 | [128] key 1 name, [129].[5] value, [130].[5] instance id |
| Create PDF (243) | `0x200_0000` restore | 0 | PDF options (not decoded) |
| Open / Append / Close PDF (246, 244, 245) | `0x20`, `0x200` create folders (Close) | 0 | [128] key 130; root key 139 (u32) the source / destination: 0 a file, 1 a field or variable |
| Print PDF (242) | `0x80` no dialog, `0x20` | 0 | [128] key 130; root key 132 (u32) the source |

A parameter at position 1 counts only with option `0x8000`: without it
FileMaker 26 shows the value in the step but runs as if it were empty
(Set Data File Position goes to 0, Read reads the whole file).

Behaviour of FileMaker 26 (Windows), reproduced by `script::datafiles`:

* File ids are 1, 2, … in the session. Open Data File of an open file:
  300; of a missing file: 100. Get File Exists of a missing file or a
  folder: 0 (no error); Get File Size: 100. A path list in a variable
  with several lines: Open takes the first file that exists.
* Create Data File empties an existing file. Create folders worked with
  an absolute path; with `file:sub/x.txt` FileMaker returned 100 (BaseCraft
  creates the folders either way).
* Write writes at the position and overwrites (the file does not
  shrink); text is UTF-16 little-endian without a byte order mark unless
  UTF-8 is chosen (bytes also write UTF-16 for text); the line feed is
  one `0A` (two bytes in UTF-16); `¶` is written as `0D`.
* Read with a count starts at the position: bytes for UTF-8 and bytes,
  characters for UTF-16; past the end it returns what is left with error
  10 (empty at the end). Without a count it returns the whole file
  whatever the position and leaves the position at the end. Invalid UTF-8
  reads as U+FFFD. Bytes give a container named after the data file.
* Set Data File Position accepts positions past the end (error 0);
  negative: 14, position unchanged. Close / Read / Write of a closed or
  unknown id: 100. Rename and Delete of a missing file: 100.
* PDF: Cancel PDF with nothing open 0; Close PDF with nothing open 5;
  Append PDF with nothing open 829; Create PDF with one open 833; Append
  of a missing file 100; Close PDF after Create PDF saves an empty PDF
  (422 bytes); Print PDF of a missing file 100.
* Go to List of Records: the records in the list's order, the first one
  current; ids that match no record: 101 (the others are shown); none:
  401 and an empty found set; a repeated id counts once.
* Configure Persistent Data without a name: 5. Configure Local
  Notification and Configure AI Account with no parameters: 5. Insert
  Object: 4 (the step is marked obsolete in the Script Workspace); Insert
  PDF and Insert Audio/Video with no field: 3.

Error codes FileMaker 26 returned for these steps (a script of the steps,
each followed by `Get ( LastError )`, run in a copy of Tasks on Windows;
`tools/oracle/step_errors_m5.xml`): Go to Object, Set Web Viewer on a
missing object 116; Select / Close Window by a missing name 112; Perform
Script by a missing name 104; Set Next Serial Value on a field that is not
a serial 5; Find Matching Records with no field and no active field 401;
Go to Related Record to a layout of another table 414; Truncate Table with
no table 106 (nothing deleted); Speak, Dial Phone, Perform Script on
Server, Configure NFC Reading, Region Monitor, Machine Learning Model,
AVPlayer Play 3; Set Allowed Orientations 4; Enable Touch Keyboard,
Set Multi-User, Copy All Records, Omit Multiple, Show Omitted Only, Sort
Records without keys, View As, Set Zoom Level, Show/Hide Toolbars 0.
`Get ( WindowName )` of a file's first window is the file name without
extension; after New Window [Name: "W2"] it is `W2`; Set Zoom Level
[Zoom In] from 150 % gives 200; View As [Cycle] from Table gives Form. Go to Record by calculation past the
end, 0, or Previous on the first record: 101. Perform Quick Find with no
match: 401 and an empty found set. Go to Portal Row with no related
record: 0 (nothing happens); with an active field outside any portal: 3.
Go to Related Record with no related record: 101 (layout unchanged).
Execute SQL without a data source: 5. Modify Last Find enters Find mode
(`Get ( WindowMode )` 1); Set Window Title [Current; "T9"] makes
`Get ( WindowName )` `T9`.
## 9. Value lists ✅

`[33].[5].[V]`: key 16 = name, key 3 = custom values (CR-separated),
key 2 = definition, 216 UUID:

| Bytes | Meaning |
|------:|---------|
| 0 | source: 0 custom values, 1 values from a field |
| 1–2 | u16 options: 0x01 only values related from an occurrence, 0x02 sort by the second field, 0x04 show only the second field (each checked alone in FileMaker's XML) |
| … | first field: `len TOref` (`02 01 00` when none), `len 01 field`, u16 `00 01` |
| … | second field, the same (`02 01 00 02 01 00 00 01` when none) |
| … | the occurrence related values start from (`02 01 00` when none), then two `02 01 00` |
| last 2 | `01 7A` |

The catalog `[33].[1]`: key 0 last id (length-prefixed), 4 count, 32 =
01 (XML `SortOrder`), name index `[1]`, display order `[3]`, `[6]` an
index keyed by the u16 source and the name key without its tertiary part
(like the field type index), `[7]` the set of ids (one empty key per
id).

## 10. Layouts ✅

Layout data is **Protocol Buffers** (schema-less reader in `pb.rs`):

* `[4].[5].[L]` key 2 = header (2 TO in XML form, 3 menu set, 5 style, 7 local CSS, 8 width, 11 print columns, 12 margins; see "Layout Setup" below), key 11 = theme (`01 n`), keys 240/241 = flags and options, key 33 = the layout's own named styles (the XML's `<LayoutCustomStyle>`, styles saved with the layout instead of the theme) in the theme's compiled form: repeated field 2 = `{1 name, 2 rules, 3 object type}` (§12); an object names one in its style field 2 like a theme style;
* `[4].[5].[L].[3]` key 0 = number of parts (length-prefixed); `[4].[5].[L].[3].[P]`
  (P = 1…n, top to bottom) key 2 = part (1 kind, 2 = 0, 3 = 0, 4 top,
  5 height, 6 break field, 10 style, 11 alternate-row style), key 7 =
  the part's objects (below);
* key 7 (or long value `[7]`) = object list, **front to back** (the first object is on top): repeated field 2 = object
  (1 kind, 2 id, 3 options, 4 bounds {1 top, 2 left, 3 bottom, 4 right},
  5 style {1 local rules, 2 style name, 3 theme object type, 4 display name} (§12),
  6 children (repeated), 7 text options, 8 field {2 {1 TO, 2 field, 3 repetition}},
  13 tab control, 14 tab panel, 15 portal, 16 button);
* `[4].[5].[L].[5].[obj]` key 1 = static text (also a button's or tab
  panel's label text), `.[3]` = button label calculation, `.[2]` = tab
  label calculation.

Part kinds (field 1): 1 Header, 2 Leading grand summary, 3 Leading
sub-summary, 4 Body, 5 Trailing sub-summary, 6 Trailing grand summary,
7 Footer, 8 Title header, 9 Title footer, 12 Top navigation, 13 Bottom
navigation; parts are stored in that order. 1, 3, 4, 7 and 12 are seen in
the templates; 2, 6 and 7 were written by BaseCraft and shown by
FileMaker under their names (Layout mode labels, XML `Part type`). The
part's style type (§12) follows the kind: 31 title header, 32 header,
33 leading grand summary, 34 body (35 alternate rows), 101 leading
sub-summary, 201 trailing sub-summary (assumed), 37 trailing grand
summary, 38 footer, 39 title footer, 40 top navigation, 41 bottom
navigation. The break field of a sub-summary (field 6) is
`08 len 00 <TO ref 03 D0 hi lo> len <path-int field>`, e.g.
`08 07 00 03 D0 00 02 01 08` = field 8 of occurrence 130.

Merge fields in layout text are usually `<<TO::Field>>` by name. Files from
older versions store them **by reference**: `<<`, the characters`01 02 0E`(`01 01 0E` seen too), then the same field reference spelled in
hex digits, `>>`: `<<\x01\x02\x0E0003D000020108>>` = field 8 of
occurrence 130 (`00 03 D0 00 02 01 08`). Checked against the XML export of
such a file (`<<TO::Field>>` there) and FileMaker's display.

### Layout objects in detail

* Object list field 1 (repeated varint) = ids of the **top-level** objects,
  ascending in the templates; children of portals, tab panels and button
  bars are not listed. Field 2 = the objects, front to back.
* **Containers list their direct children** in object field 6 (repeated
  varint, ascending): a portal its fields, a tab control its panels, a
  panel its objects, a button bar its segments, a group its members.
  Children's bounds are relative to the container. Decoding nesting from
  field 6 gives the XML's nesting for all 633 objects of the oracle files
  (the geometry alone got 97 %).
* The list is stored inline (key 7 of `[4].[5].[L]`, empty = `00 07`) up
  to 1000 bytes; above that in `[4].[5].[L].[7]`: key 0 = total length
  (u32 BE), keys 1…n = 1000-byte chunks (largest inline list seen: 834
  bytes; smallest split one: 1,812).
* Object kinds (XML `kind`): 1 field, 2 text, 3 graphic, 4 line (style
  type 50), 5 rectangle (51), 6 opens in FileMaker (rounded rectangle,
  🔸), 7 oval (53), 8 group (no style), 9 portal (83), 10 button (80),
  11 tab control (81), 12 tab panel (82), 13 external object (web
  viewer 86, chart 85), 14 popover button (80), 15 popover (89), 16 slide
  control (91), 17 slide panel (92), 18 button bar (78; its segments are
  buttons of style type 79). Kinds 13–18 were made with FileMaker's Insert
  menu and read back.
  Kinds 3, 7 and 8 were checked by writing them and reading FileMaker's
  XML type names. A graphic object (kind 3) names its picture in field
  10.1: an id in the file-wide picture store `[31].[5].[id]`, laid out
  like a table's container store (§13; `FNAM`, `JPEG`/`PNGf`/`PDF `
  streams, `SIZE`, plus an undecoded 8-byte `FORK`; the templates' store
  holds `GLPH` and `SVG ` icon streams). BaseCraft draws PNG, JPEG and
  BMP (`BMPf`, with or without the 14-byte `BM` header: 🔸 no `BMPf`
  stream seen yet) in Browse, Preview and PDF; GIF, SVG and PDF pictures
  are not drawn. Files from the
  FileMaker 12 era make buttons out of any object, often a group (kind 8):
  the action is then field 12, whose field 1 is the step message that
  newer buttons keep in field 16.3 (checked on a 2016 file whose three
  group buttons each run a Perform Script). A field's control style is field 8.5 (0 edit box,
  1 drop-down list, 2 pop-up menu, 3 checkbox set, 4 radio button set,
  6 drop-down calendar) and its style type follows it (61, 62, 63, 64,
  65, 66; 67 container); radio sets were checked in FileMaker. A concealed
  edit box is an edit box (0) with field-object flag `0x200000` (below); 5
  is not it: written on a template copy's edit boxes, FileMaker 26 still
  showed their text.
* **Options** (field 3, XML `<Options>` except bit 0x20000, which the XML
  drops; checked on 5,544 template objects and by setting each option in
  FileMaker's Layout mode): autosizing anchors in the top bits, `1 << 28`
  left, `1 << 29` top, `1 << 30` right, `1 << 31` bottom (left + top =
  FileMaker's default); `0x2` locked; `0x10` slide up (based on all
  objects above); `0x20` slide left; `0x200` hide when printing;
  `0x8000` slide up based on the objects directly above only, `0x40` also
  resize the enclosing part (each set in FileMaker's Inspector, Sliding &
  visibility); `0x4` the object has a "Hide object when" calculation,
  `0x100` the hide condition applies in Find mode (Inspector › Data ›
  Behavior). `0x1` = the object has conditional formats (`.[138]`) and
  `0x4000` = it has a tooltip (`.[3]`): both hold on every template object
  and on the objects FileMaker wrote in the experiments. `0x8` is set by
  FileMaker on new popover buttons and button bar segments (and on some
  template buttons and containers); toggling every Inspector option never
  changed it, its meaning is not known. `0x3000` (lines): not decoded.
* Field 8 of a field: `{1 options (0x1200E0 on edit boxes), 2 reference,
  3: 1, 4: 0, 5 control style, 6 value list, 7: 1, 8: 1}`.
  Field 8.1 is the clipboard XML's `FieldObj flags` and the Save-as-XML
  `<Field><Options>` (`fmp12::layouts::field_flags`). Decoded by pasting
  field objects with one flag changed and reading FileMaker's Inspector ›
  Data, and by unchecking a box there and diffing the saved file:
  `0x400` "Include arrow icon to show and hide list" (drop-down list; the
  arrow disappears without it) and "Include icon to show and hide calendar"
  (drop-down calendar: clearing it on a template copy's calendars removed
  their icon in FileMaker 26's Browse mode); `0x800` auto-complete: "Auto-complete
  using previously entered values" on an edit box, "Auto-complete using
  value list" on a drop-down list; `0x200000` concealed edit box (the
  text drawn as bullets; found by setting the undecoded bits of a template
  copy's edit box in halves and typing into it in FileMaker 26; no template
  object has it); `0x8000` set = "Include field for
  Quick Find" **off** (Inspector › Data › Behavior; the only byte that
  changed, `0x1200E0` → `0x1280E0`; FileMaker's default is on, i.e. clear;
  the templates' container fields `0x128000`/`0x128014` carry it). The
  clipboard XML's `quickFind` attribute is not this bit alone: FileMaker
  writes `quickFind="0"` for container fields without `0x8000`, and
  ignores the attribute when pasting. Other bits (`0xE0`, `0x20000`,
  `0x100000`, `0x1`/`0x8` on pop-up menus and drop-downs, `0x14` on
  containers) are not decoded.
* Portal (field 15): `{1: 0, 2 TO (XML form), 3 options (273), 5 first
  row, 6 rows (double), 7: 1}`.
* Tab control (field 13): `{1: 264, 2: 0, 3 tab height (45), 5 front
  panel}`. FileMaker does not draw a tab control whose front panel is not
  one of its own panels. Tab panel (field 14): `{1: 1, 2 tab offset,
  3 tab width, 4: 0}`; its label is node key 1 and the calculation `.[2]`.
* `[4].[5].[L].[5]` key 0 = last object id (length-prefixed integer).
* `[4].[5].[L].[5].[obj]`: key 1 text, key 7 = position in the tab order
  (`00 n`, 1-based), key 13 = data formatting (below; `00 00` without),
  key 16 = object name (empty: `00 10`), keys 216/217 = UUIDs.
  Calculations are sub-nodes `{4: 01 00, 5: tokens, 6: compiled form}`
  (a text literal is `13 len text 00`, §11): `.[2]` button, popover
  button or segment label (a popover's title; a tab panel's label),
  `.[3]` tooltip (of any object), `.[6]` placeholder text of a field,
  `.[141]` "Hide object when", `.[128].[5]` a web viewer's address (a
  chart's title, see Charts); `.[138]` conditional formats (below);
  `.[17]` button step parameters. A static button label is node key 1.
* **Text with merge fields** keeps its text in `.[1]` key 1 instead of
  node key 1 (all 80+ merge-field texts of the templates; Save as XML
  shows `<StyledText><Data>DURATION <<Duration>></Data>`). Key 0 = three
  big-endian u32: the byte length of key 1, its character count, and 4.
  Each `<<…>>` holds a stored reference, not the name: U+0001, then
  U+0001 for a field of the layout's table or U+0002 for a related one,
  then the length of the rest as a character (U+000E = 14), then
  `0003D` + the occurrence id − 128 in five hex digits + `01` + the field
  id in two hex digits: `<<\u{1}\u{1}\u{e}0003D000020106>>` = occurrence
  130 (Meetings), field 6 (Duration); `<<\u{1}\u{2}\u{e}0003D000040107>>`
  = Action Items::Number of Action Items. Checked on Meeting Details
  against the XML (`fmp12::layouts::merge_ref`, engine test
  `stored_merge_fields_resolve`). 🔸 Field ids above 255 and the meaning of
  `0003D`/`01` are unseen.
* 🔸 **Style runs** (several fonts, sizes or colours in one text object)
  are not decoded: none of the Claris templates or add-on modules has one
  (no `.[1].[255]`, no `CharacterStyle` in their Save as XML), so there is
  no ground truth yet. An FMUpgradeTool patch (FileMaker Server 26, VM)
  adding a text object with `<CharacterStyleVector>` (the clipboard
  grammar) or guessed `<CharacterStyleList>`/`<FontStyle>` children keeps
  the text and silently drops the styles; FMEngine.dll names the Save as
  XML elements `CharacterStyleList`, `CharacterStyle`, `FontReference`,
  `FontStyle`, `ParagraphStyleList`, `ParagraphStyle` and attributes
  `face`, `size`, `underline`, `postscriptName`, `textAlign`, `position`,
  `offset`, but not their nesting. Needs a file with a styled text object
  made in FileMaker Pro.
* Popover button (field 17): `{1: 0, 2: 2, 3 popover id, 5 icon size 12,
  6: 0}`; its popover (field 18): `{1 options, 3 position, 4 popover
  button id}`. The popover is a top-level object that lists its objects
  in field 6; it is not in the tab order. Slide control (field 19) `{1
  options}`, slide panel (field 20) `{1: 0}`, the panels are the
  control's children (field 6) and the front one is the first.
  🔸 slide options bits are not decoded (`slideFlagSet` of the clipboard
  XML; BaseCraft assumes 1 = no swipe, 2 = no dots). Button bar (field
  21): `{1 options, 2 active segment id}`. Web viewer (field 11): `{1:
  0x57454256 'WEBV', 2: 0, 3 options}`; a new one has options 32781
  (`0x800D`), the Contacts template's map `0x8003`. 🔸 Read as Web Viewer
  Setup's checkboxes (`fmp12::layouts::web`): `0x1` allow interaction, `0x2`
  display in Find mode, `0x4` progress bar, `0x8` status messages, `0x8000`
  encode URL, `0x10` (assumed) allow JavaScript to perform scripts; inferred
  from the dialog's defaults and those two values, not toggled one by one.
* **Conditional formats** `[obj].[138]`: key 4 = count (u16), then
  `.[128 + i]` per condition, in order: key 1 = first value as typed,
  key 6 = second value (between), key 13 = `{1 options, 2 style {1 local
  rules, 3: 0}, 3 operator}`, `.[5]` = the formula FileMaker evaluates.
  Operators, in the order of the Conditional Formatting dialog: 0
  formula, 1 between, 2 not between, 3 equal, 4 not equal, 5 greater, 6
  less, 7 greater or equal, 8 less or equal, 9 containing, 10 not
  containing, 11 beginning with, 12 ending with, 13 empty, 14 before
  today, 15 today, 16 after today, 17–20 more / less than x days
  after / before. For a value test FileMaker writes the formula itself
  on `Self` (`Self = "hello"` for "equal to hello", tokens `FA 00 44 13
  …`): a value test pasted without it never applies. The style holds the
  `.self` normal-state properties the dialog sets (text and fill colour,
  weight, style, underline…).
* **Data formatting** (node key 13, `fmp12::formats`): a 210-byte record
  (213 with the container format), big endian, strings UTF-16 BE padded
  with zeros. Offsets were mapped by pasting field objects whose
  clipboard XML (`NumFormat`, `DateFormat`, `TimeFormat` attributes)
  took one value at a time, and meanings by reading how FileMaker Pro 26
  shows -1234.5678, 7 March 2026 and 14:05:09 with each one
  (`print::format` tests).

  | Bytes | Meaning |
  |------:|---------|
  | 2–3 | number options: `0x1` general, `0x2` thousands separator, `0x4` notation (currency, or percent with `0x10`), `0x8` fixed decimals, `0x20` Boolean, `0x100` as entered, `0x200` set with currency (no visible effect), `0x400` symbol before the minus sign, `0x800` leading symbol (else trailing), `0x1000` negative colour, `0x2000` hide zero, `0x4000` system separators (the locale's, with thousands) |
  | 4 | numeral style |
  | 5 | negative style: 0 `-1234`, 1 `1234-`, 2 `(1234)`, 3 `<1234>`, 4 `1234 CR`, 5 `▲1234`, 6–7 no sign |
  | 6–17 | currency symbol |
  | 18–19, 20–21 | thousands and decimal separators |
  | 22–24 | negative colour (RGB) |
  | 26–27 | decimal digits |
  | 28–43, 44–59 | Boolean texts (non-zero, zero) |
  | 60 | date format: 0 as entered, 1 custom, 2 numeric with 2-digit year and separator (`7/3/26`, the locale's order), 7 the same with 4 digits, 3 long month (`March 7, 2026`), 4 abbreviated month, 5 abbreviated weekday and month, 6 full, 8 system short (as entered), 9–11 system long |
  | 61, 62, 63 | date numeral style; month and day style: 0 none, 1 leading zero, 2 leading space |
  | 64–65 | date separator (formats 2 and 7) |
  | 66–69 | custom date elements: 1 day, 2 abbreviated weekday, 3 weekday, 4 month number, 5 abbreviated month, 6 month name, 7 2-digit year, 8 4-digit year, 9 quarter (`Q1`, `T1` in French), 10 quarter name, 11–17 Japanese eras and names |
  | 70–149 | custom date texts: before the first element, between, after (5 × 16 bytes) |
  | 150–151 | time options: `0x1` hours, `0x2` minutes, `0x4` seconds, `0x8` the system's format (shown as entered; FileMaker's default `0x8F`), `0x80` formatted. Custom times are 12-hour with the AM/PM texts; minutes without hours count from midnight; seconds take the number format's fixed decimals |
  | 152, 153, 154 | time numeral style; hour style and minutes/seconds style: 0 none, 1 leading zero, 2 leading space |
  | 156–157 | time separator |
  | 158–173, 174–189, 190–205 | AM, PM and 24-hour suffix texts |
  | 210–211 | container (graphic) format (🔍 values 0–7, FileMaker's default 5; scaling not decoded) |

  Timestamps take the date format only with a custom time format.
* `[4].[5].[L].[10]`: a spatial index of the fields, buttons, tab controls
  and button bars: byte-string key = top, left (u16, absolute), id − 1
  (u16); value = `00 len 01 id …` (`len` = length of `01 id`), then
  `00 00 00 00 01` (other tails seen). Portal children are not listed.
* `[4].[5].[L].[21]`: tab order, key 128 + n → the n-th object, the same
  object reference.
* `[4].[5].[L].[3].[P]` key 7: the top-level objects whose top edge lies
  in the part (`top ≤ y < top + height`), ascending ids: u32 count, then
  `len 01 path-int id` each. The rule reproduces all 726 template parts.
* Buttons (object field 16): `{1 options, 2, 3: step {1 id, 2 arg,
  3 options, 4 14-byte inline block}, 4 icon type (none = text only,
  1 SVG), 5 icon size, 6}`. The inline block is `len 01 id` then one byte
  `BE` or `6F` that does not depend on the target.
* `[4].[5].[L].[13]` holds a second copy of parts, objects and indexes
  (meaning not settled; left as is).
* `[4].[1].[7].[L]` repeats the layout's name (key 16) and a message whose
  field 2 is its table occurrence (field 3 the menu set); keys 240 and
  241 repeat the layout's (below), key 48 = number of layout script
  triggers (u32); `[4].[1].[1]` indexes layout names by collation key (§14),
  `[4].[1].[3]` is the display order.

### Layout Setup

Decoded by changing one setting at a time in FileMaker Pro 26's Layout
Setup (« Format de modèle ») on copies of Contacts and diffing; a file
BaseCraft wrote with every setting changed opens in FileMaker with the
same values in the dialog (and passes FileMaker Server's consistency
check; its XML `<Options>`, `hidden`, `<MenuSet>` and `<ScriptTriggers>`
agree). `fmp12::layouts::setup`.

* **Key 241** of `[4].[5].[L]` (u64 BE, the same value in `[4].[1].[7].[L]`
  key 241; it is the XML's `<Options>`), templates `0x10F3080D`:

  | Bit | Meaning |
  |----:|---------|
  | `0x1` | "Delineate fields on current row only" |
  | `0x2`, `0x4`, `0x8` | Form, List, Table view **not** available |
  | `0x10` | "Save record changes automatically" **off** |
  | `0x20` | "Show field frames when record is active" |
  | `0x40` | "Use fixed page margins" |
  | `0x80` | "Facing pages" |
  | `0x100` | "Print in n columns" |
  | `0x200` | default view List; `0x4200` Table; neither = Form |
  | `0x400` | columns "Down first" (else "Across first") |
  | `0x8000` | "Enable Quick Find" **off** |
  | `0x1000_0000` | "Show current record indicator in List view" **off** |

  ponytail: `0x4000` was only seen with the Table default view; with it
  FileMaker also wrote header fields 13 = 22.0 and 14 = 25.0 (not
  decoded, BaseCraft leaves them). The other set bits (`0x800`,
  `0xF30000`) never changed.
* **Key 240** (u64 BE, also in the catalog entry): `0x1` folder, `0x2`
  separator, `0x8` "Include in layout menus" **off** (XML
  `hidden="True"`); FileMaker writes the layout's key only once it is not
  0. 🔍 `0x10` seen on template folders.
* Header (key 2) field 3 = **menu set**: 0 `[File Default]`, 1
  `[Standard FileMaker Menus]` (XML `CustomMenuSetReference` id); also
  field 3 of the catalog's key 2. ponytail: custom menu set ids assumed
  to be the same numbering, custom menu sets are not decoded.
* Header field 11 = **number of columns** (2 by default), field 12 =
  **fixed margins** `{1 top, 2 left/inside, 3 bottom, 4 right/outside}`
  in points (36 in new files; FileMaker shows the printer's margins while
  "fixed" is off and rewrote field 12 with them when columns were set).
  Field 10 changed with the columns and margins (190.33, 259.0): a width
  FileMaker computes from the page (not decoded, left as is). Fields 6,
  9, 13–17 did not change with any setting.
* **Script triggers** (§8): `[4].[5].[L].[48]` key 4 = count, nodes
  `[48].[5].[128 + event]`; adding two triggers also set the catalog's
  key 48 to 2 (XML `<ScriptTriggers membercount>`). FileMaker's list
  order: OnRecordLoad, OnRecordCommit, OnRecordRevert,
  OnLayoutKeystroke, OnLayoutEnter, OnLayoutExit, OnLayoutSizeChange,
  OnModeEnter, OnModeExit, OnViewChange, OnGestureTap,
  OnExternalCommandReceived; Preview is greyed out for OnRecordLoad.
* Not decoded: Table View Properties (grid, parts, column headers, row
  height; the XML `<GridStyle>` and `<TableView>`), "Reset Quick Find".

### Charts (`fmp12::charts`) ✅ read, 🔸 edited

Decoded from charts made in FileMaker Pro 26 (Insert › Chart, Chart Setup,
and layout objects pasted as `Mac-XML2` clipboard snippets, then diffed;
fixture `crates/fmp12/tests/fixtures/charts.fmp12`, 14 charts).

* A chart is object kind **13** (an external object, like a web viewer):
  object field 11 = `{1: 0x43485254 'CHRT', 2: 0 (typeIndex), 3: 19
  (externalFlagSet)}`; options (field 3) `0x30000000`. In the clipboard XML
  it is `<Object type="ExternalObject"><ExternalObj typeID="CHRT">`.
* Calculations in `[4].[5].[L].[5].[obj].[K].[5]` (keys 4, 5, 6 as for
  button labels): K = 128 chart title, 129 X axis title, 130 Y axis title,
  131 X data, 133 + 2*i data of Y series *i*, 134 + 2*i its name. A bubble
  chart's size (XML `WSeriesList`) follows the last series: key 137 with
  two series. 🔸 one-series bubbles not seen.
* Settings record `[4].[5].[L].[5].[obj].[139].[138]`: key 0 = length
  (u32 BE), keys 1… = 1000-byte chunks of a record of big-endian u32 words
  (1,110 bytes for one series, 1,185 for two). `.[139]` holds `00 00`.

| Byte | Meaning |
|-----:|---------|
| 0–3 | 2 (version?) |
| 4–7 | show data points on chart (values on the marks) |
| 8–11 | type: 0 Column, 1 Bar, 2 Line, 3 Area, 4 Pie, 5 Positive/negative, 6 Stacked column, 7 Stacked bar, 8 Bubble, 9 Scatter (XML `Column`, `Bar`, `Line`, `Area`, `Pie`, …, `Stacked Column`, `Stacked Bar`, `Bubble`, `Scatter`) |
| 20–23 | colour scheme: 3 Earth (default, XML `Earth`), 0 Bright; FileMaker 26 lists Earth, Jewel Tones, Pastel, Desert Spring, Gray, Brown, Red, Orange, Purple, Blue… (other codes not mapped) |
| 56–59 | show legend (XML `Legend style="Normal"` / `"None"`) |
| 120 | data source: 2 current found set, 1 current record (delimited data), 3 related records |
| 121… | length-prefixed: `02 01 00`, or for related records the table occurrence `04 03 D0 hi lo` |

  After the source the record has variable-length parts (fonts: `00 0D 3F B7
  …` with size and RGBA colour, the series colours list `00 00 00 02 38 79
  A3 FF D8 92 41 FF`, gridline colours, axis number formats) that are not
  decoded.
* Writing (`save_layouts`, `LayoutObject::chart`): the calculations are
  rewritten as web viewer addresses are (`.[K]` and `.[K].[5]` marked), the
  record with words 4, 8, 20, 56 and the source (byte 120 and its
  length-prefixed reference) replaced, the rest kept; key 0 = new length.
  FileMaker Server's consistency check passes on edited charts. The record
  is 75 bytes longer per series: the series count is not changed.
* Rendering (`print::chart`, shared by Browse, Preview and PDF), measured
  on FileMaker's Browse mode: title 19 pt on top, legend (swatch + series
  name) under it, value labels on both sides of the plot (top and bottom
  for bar charts), a single series coloured per category (Earth:
  `909A3F BD9D7D 4E7E99 CAA465 8E8E8E 7A7841 DAC582 4D7D98 646567 60ABB3
  6F7349 847566`), several series one colour each; positive/negative
  charts show the first series over a blue (positive) and pink (negative)
  background; delimited data are return-separated lists of the current
  record.

### Writing (`fmp12::save::save_layouts`, `schema_edit`) ✅

FileMaker writes minimal protobuf, so decode → encode gives the same
bytes for every layout message of the 66 templates (`tests/layouts.rs`).
The writer keeps the raw messages and patches only the fields that
changed: an object's options (3), bounds (4), style (5), children (6),
field reference and control (8), tab (14), portal (15), button step
(16.3); a part's kind, top, height, break field and style; the header's
table occurrence (2) and width (8). New objects are built on an existing
object of the same kind and style type (from the same layout if
possible), which keeps the fields we do not understand, or from scratch
with the template values above; their node copies key 13 of the
prototype and gets fresh UUIDs; new fields are added to the end of the
tab order. A new tab control's front panel is its first panel. Deleted
objects lose their node and their index and tab-order entries (the tab
order is renumbered); moved ones get their index key shifted by the
move. The part object lists are recomputed. When parts are added,
removed or reordered, `[L].[3]` is rewritten (`{1 kind, 2: 0, 3: 0,
4 top, 5 height, [6 break], 10 {3 type}, 11 {3 type}}` for a new part).
A rename goes through `schema_edit::Editor::rename_layout` (both name
keys and the name index). New, duplicated and deleted layouts are
`Editor::new_layout` (a copy of the file's first plain layout without
objects, with empty part lists), `duplicate_layout` (every entry of the
layout and its catalog entry, fresh UUIDs) and `delete_layout`.

Text and shape styles go into the object's local style (§12) on the
`.self` element, as FileMaker's Inspector does.

Sub-nodes of an object node are spelled as FileMaker writes them, and
every level of a sub-node gets its node marker: `.[138]` and `.[141]` as
one byte (`8A`, `8D`), indexed slots (`128 + i`: conditions, a web
viewer's `.[128]`) as `00 i`. With the two-byte spelling `80 0D`
FileMaker reads no calculation; without the marker of `.[128]` its
consistency check reports a damaged block (FMDeveloperTool, error 805).

The "Objects Demo" layout (`BASECRAFT_OBJECTS_DEMO=<out.fmp12> cargo test
-p basecraft objects_demo_file`, from the Assets template) was checked in
FileMaker Pro 26 with `layout_check.ps1` and FileMaker Server's
consistency check (`fms_check.sh`, no problems): it opens without a
warning and FileMaker shows the price as `($1,234.50)` in red, the date
and the timestamp in their formats, the name red and bold by its value
condition, the description (its hide condition false), the popover with
its title when its button is clicked, the second slide after a click on
its dot, the button bar with its active segment, and the web viewer
loading its computed address; the XML export lists the conditions
(`Self = "Laptop"`, `IsEmpty ( Assets::Price )`), the hide calculation,
the placeholder, the tooltip, the formats and the tab order.

Checked in FileMaker Pro 26 (`tools/oracle/layout_check.ps1`, the file
built by `BASECRAFT_LAYOUT_DEMO=<out> cargo test -p basecraft
layout_demo_file` from the Contacts template): it opens without a
warning; the layout pop-up lists the new and the duplicated (renamed)
layout and not the deleted one; Browse and Layout mode show the new
parts, the fields with their labels, a pop-up menu, a portal on a related
occurrence with its two fields (`Addresses [1..3+]`), a tab control with
two tabs and a field on the first, a button running a script with its
label, underlined text, a rounded and locked rectangle, an oval and a
line grouped, and a field in bold italic red centred text; Save a Copy as
XML lists them with their kinds and options.

## 11. Calculations ✅

Stored next to their owner as keys 4 (context: a TO ref `03 D0 hi lo`, or
`01 00` in scripts and layouts), 5 (token stream that keeps the source text,
whitespace included) and 6 (compiled form). Key 6 is what FileMaker
evaluates: without it an auto-enter calculation gives `?`, and FileMaker
does not rebuild it from key 5.

The token table was recovered by overwriting token bytes in copies of real
files and reading FileMaker's XML rendering. Full table:
`crates/fmp12/src/calc_tables.rs` (operators, 162 one-byte functions, 91
two-byte functions behind prefixes F8–FE, 140 `Get` flags after `9C`).

| Token | Meaning |
|-------|---------|
| `03` `04` `05` `06` `07` | `;` `(` `)` `[` `]` |
| `0B len name 00 xx` | Let variable name |
| `0C 13 len text 00` | whitespace or comment, emitted verbatim |
| `10 02 n l` + groups + u16 flags | number: u16 `n` integer groups, u16 `l` decimals (16 or more), then u32 groups of nine digits: `n` for the integer part (right-aligned), ⌈(l+2)/9⌉ for the decimals |
| `13 len text 00` | text literal; `len` counts UTF-16 units, not bytes |
| `16 flags 04 TOref len 01 field u16 rep` | field reference (shown as `TO::Field` outside the context TO) |
| `1E`–`23` | not, and, or, xor, True, False |
| `25`–`29`, `2C` | + − × ÷ ^, unary minus |
| `41`–`49`, `50` | comparisons, `&` |
| `9B` / `9C xx` | `Get` / its flag |
| `F8`–`FE xx` | two-byte functions (`F8 00` = While, `FA 00` = Self, …) |
| `0D 13 len text 00` | `/* comment */` (text between the delimiters) |
| `0E 13 len text 00` | `// comment`, its line break included |
| `24` | `¶` outside quotes |
| `1A len name` | `$variable` / `$$global`; `06 … 07` after it = repetition |
| `1B 14 len 01 id` | custom function call (§16) |
| `1D 00 00 15` + 4 bytes + u16 | library function: `FMDR` design functions (01 DatabaseNames, 02 FieldBounds, 03 FieldIDs, 04 FieldNames, 05 FieldRepetitions, 06 FieldStyle, 07 FieldType, 08 GetNextSerialValue, 09 LayoutIDs, 0A LayoutNames, 0B TableIDs, 0C TableNames, 0D RelationInfo, 0E ScriptIDs, 0F ScriptNames, 10 ValueListIDs, 11 ValueListNames, 12 ValueListItems, 13 WindowNames, 14 FieldComment, 15 LayoutObjectNames, 16 BaseTableIDs, 17 BaseTableNames, 18 BaseTableComment, 19 FieldAnnotation, 1A FieldDisplayNames), `FMMB` 01 Location, 02 LocationValues, 03 RangeBeacons |

A `len` of 128 UTF-16 units or more takes two bytes, the first with its top
bit set (`13 81 14` = 276 units). Unary minus is `26` like subtraction in
key 5. A number literal keeps its value, not its spelling (`1,50` and
`1.5` give the same bytes); its last two bytes are `00 20`, or `00 40`
when it was written with an exponent. Each u32 group holds nine decimal
digits (0–999 999 999). The integer part takes as many groups as it needs
(`10 02 00 02 00 10 00000001 00000000 …` = 1 000 000 000;
123456789012345678901234567890 takes 4 groups). The decimals keep 16
digits, or as many as the literal has, plus 2 guard digits, rounded up to
whole groups: 16 decimals take 2 groups, 17 take 3
(`0.1234567890123456789012345`: `l` = 25, 3 groups). A negative literal is
unary minus (`26`/`2C`) and its absolute value. From Set Variable steps
pasted into FileMaker Pro 26 (`fixtures/compiled.tsv`). A calculation FileMaker cannot parse is stored as one
`0D` comment holding the whole text, with key 6 = `00`.

### Compiled form (key 6)

Prefix notation without punctuation or trivia: each operator or function
token comes before its operands, which are compiled the same way.

* Binary operators: token, left, right (`1 & 2 + 3` → `50 1 25 2 3`);
  precedence from loose to tight: `or`/`xor`, `and`, comparisons, `&`,
  `+ -`, `* /`, `^`, then prefix operators; all left-associative
  (`2^3^2` = `(2^3)^2`). Unary minus `2C`, unary plus `68`, `not` `1E`.
* Number literals: as in key 5 without the last two bytes. Text, field
  references, `$variables`, `Get` (`9B flag`) and constants as in key 5.
* Functions with a fixed number of parameters: token, then the arguments
  in order. `If` with two arguments ends with `00`.
* Functions with optional or repeated parameters write a u16 argument
  count and the arguments **in reverse order**: Factorial, Substitute,
  Average, Count, Max, Min, StDev, StDevP, Sum, List, Variance, VarianceP,
  TextFont, TextColorRemove, TextFontRemove, TextSizeRemove, Lookup,
  LookupNext, every `F9 xx` function and every library function
  (`1D <count> 15 FMDR idx`). A `[ a ; b ]` list argument is `19`, u16
  count, items in reverse order. SetPrecision and SetRecursion reverse
  their two arguments without a count.
* `Choose`: u16 number of results, arguments in order. `Case`: u16 number
  of test/result pairs, in order; a default result is given the test `1`.
  `Evaluate`: u16 count of the expression and trigger fields, `00 00`, then
  them in order (a list of fields is spread). `GetField`: `DD 00 00` +
  argument.
* A field passed to an aggregate function, GetNthRecord, GetRepetition,
  Last, Extend, GetSummary, Lookup or LookupNext is `17` instead of `16`.
  `Field[n]` compiles as GetRepetition: `89 17… n`; `$var[n]` as `E4 n $var`.
* Custom function call (§16): `1B`, u16 argument count, `14 len 01 id`,
  the arguments in reverse order; `1C` instead of `1B` when the call gives
  the calculation's result (tail position).
* `Let`: `DF`, u16 number of bindings, the bindings, the body. Variables
  are numbered by binding position, counting the bindings of the
  enclosing Let/While (a binding that reassigns a name of the same Let
  still uses a number); a reference is `18 u16 slot` (key 5 keeps
  `0B len name 00 00`). A new name compiles to its value alone; a name
  already bound in the same Let to `E3 value 1 18 slot`; a `$variable` to
  `E3 value repetition $var`. `While`: `F8 00`, initial bindings (u16
  count + bindings), condition, loop bindings (count + bindings), result.

`calc::compile` reproduces FileMaker's keys 5 and 6 for the 1,890
calculations of `crates/fmp12/tests/fixtures/compiled.tsv` (Set Variable
steps pasted with `tools/oracle/compile_snippet.py`) and for all 5,214
calculations of the templates (fields, scripts, layouts, comments
included). The same parser derives key 6 from a stored key 5
(`calc::compiled`, used by the script writer, which keeps only key 5 in
memory). A calculation that holds only comments has key 6 = `00`. Exception: FileMaker's SCSU uses dynamic
windows and Unicode mode for non-Latin text where BaseCraft quotes each
character; both decode alike.

## 12. Themes and styles ✅ (partial)

Decoder: `crates/fmp12/src/theme.rs`. Property ids were mapped by comparing
the compiled styles of the template themes with FileMaker's installed theme
CSS (local reference only, never copied here).

### Storage

* `[6].[5].[n]`: theme *n*. Key 10 = display name ("Universal Touch"),
  13 = group, 16 = id (`com.filemaker.theme.universal_touch`),
  22 (or long value `[22]`, §6) = compiled styles (protobuf),
  `[23]` = manifest metadata as protobuf of string pairs (chart palette,
  layout-builder constants, named-style display names).
* `[4].[5].[L]` key 11 = `01 n`: the layout uses theme *n*.
* `[25].[5].[id]`: font catalog, ids from 128. Key 16 = family
  ("arial,sans-serif", "Helvetica Neue"), key 3 = face name ("HelveticaNeue-Bold").

### Compiled styles

```
top: repeated 1 = default style { 1: object type, 2: rules }
     repeated 2 = named style   { 1: name, 2: rules, 3: object type }
rules: repeated 1 = rule { 1: element, 2: state, 3: { repeated 1: property } }
property: { 1: property id, one value field }
```

The same `{1: rules, 2: name, 3: object type, 4: display name}` message is
the style of a layout object (object field 5), of a part (part field 10;
field 11 = alternate-row style, type 35 for the body) and of the layout
background (header field 5, type 20, local rules in header field 7).
Field 1 holds the object's local overrides. Effective style = theme default
for the type → named style → local rules; in each, the `:normal` rule
applies first, then the requested state.

Value fields:

| Field | Value |
|------:|-------|
| 3 | enum / integer (varint; −1 as 64-bit two's complement) |
| 5 | colour `{1 r, 2 g, 3 b, 4 a}`, f32 in 0..1 |
| 6 | length `{1 f64 value, 2 unit, 3 always 0}` |
| 12, 18 | border-image / background-image; `{1: 0}` = none |
| 13 | pair of lengths `{1, 2}` (corner radius horizontal, vertical) |
| 14 | 🔍 icon-related, element 4 only |
| 19 | box-shadow `{1 enabled (0 = none), 2 inset, 3 colour, 4 x, 5 y, 6 blur, 7 spread}` |

Units: 0 none (zero values), 1 `%`, 6 `pt`, 7 `em`, 9 `line`.

### Property ids

Confidence: ✅ matched against CSS values across several themes, or
written into a local style and read back from FileMaker's XML
(`<LocalCSS>` prints the CSS FileMaker understood),
🔸 consistent with the CSS but few samples.

| Id | CSS property | Values | |
|---:|--------------|--------|-|
| 1 | background-image | 18 | ✅ |
| 7 | background-color | colour | ✅ |
| 20–23 | border-{top,right,bottom,left}-color | colour | ✅ |
| 24–27 | border-{top,right,bottom,left}-style | 1 none, 2 hidden, 3 dotted, 4 dashed, 5 solid, 6 double | ✅ |
| 28–31 | border-{top,right,bottom,left}-width | length | ✅ |
| 32–35 | border-{top-right,bottom-right,bottom-left,top-left}-radius | pair | ✅ |
| 36 | border-image-source | 12 | ✅ |
| 64 | font-family | font id − 128 (`[25]`) | ✅ |
| 65 | font-weight | 1 normal, 2 bold, 3 `100`, 4 `200` | ✅ |
| 66 | font-stretch | 2 wider, 3 narrower | ✅ |
| 67 | font-style | 1 normal, 2 italic | ✅ |
| 68 | font-variant | 2 small-caps | ✅ |
| 69 | font-size | length | ✅ |
| 70 | color | colour | ✅ |
| 71 | direction | 2 rtl | ✅ |
| 72 | line-height | length (`line`) | ✅ |
| 73 | block-progression | 2 rl, 3 lr | ✅ |
| 74 | text-align | 1 left, 2 center, 3 right, 4 justify | ✅ |
| 75 | text-transform | 1 none, 3 uppercase | 🔸 |
| 100 | display | 2 block | ✅ |
| 101–104 | padding-{top,right,bottom,left} | length | ✅ |
| 105–108 | margin-{top,right,bottom,left} | length | ✅ |
| 109, 110 | width, height | length | ✅ |
| 117–120 | top, right, bottom, left | length | ✅ |
| 121 | position | 3 absolute | 🔸 |
| 122 | box-shadow | 19 | ✅ |
| 123 | box-sizing | 1 border-box | 🔸 |
| 124 | vertical-align | 2 super | ✅ |
| 504 | -fm-strikethrough | 0 off, other values on | ✅ |
| 505 | -fm-underline | 0 none, 1 underline, 2 word-underline, 3 double-underline | ✅ |
| 506 | -fm-glyph-variant | 2 superscript | ✅ |
| 509 | -fm-character-direction | 2 rtl | ✅ |
| 512, 516, 612, 614, 615, 617 | -fm-use-default-appearance, -fm-override-with-classic, -fm-tategaki, -fm-borders-between-reps, -fm-borders-baseline, -fm-texty-field | bool | ✅ |
| 602 | -fm-fill-effect | integer | ✅ |
| 625, 626 | -fm-grid-width, -fm-grid-height | integer | ✅ |
| 513 | -fm-icon | 2 down-arrow, 3 calendar, 5 radio, 6 modern-check | 🔸 |
| 514 | -fm-icon-color | colour | ✅ |
| 515 | -fm-tab-spacing | integer | 🔸 |
| 610 | -fm-portal-alt-background | bool | 🔸 |
| 611 | -fm-text-vertical-align | 0 top, 1 center, 2 bottom | ✅ |
| 616 | -fm-body-alt-background | bool | 🔸 |
| 619 | -fm-use-body-current-row-style | bool | 🔸 |
| 620 | -fm-use-portal-current-row-style | bool | 🔸 |
| 621 | -fm-icon-padding | length | ✅ |

Unknown: 2 (field 14, icons), 510/511 (pairs), 613 (in every local style, 0),
618 (a shadow on `.inner_border`).

### Object types

| Id | CSS type | Id | CSS type |
|---:|----------|---:|----------|
| 20 | layout_background | 60 | field (shared: `.baseline`, `:droptarget`) |
| 30 | any part (named part styles) | 61 | edit_box |
| 32 | header (part kind 1) | 62 | drop_down |
| 34 | body (part kind 4) | 63 | pop_up |
| 35 | body_alt | 64 | checkbox_set |
| 38 | footer (part kind 7) | 65 | radio_set |
| 40 | top_nav_part (part kind 12) | 66 | calendar (drop-down calendar) |
| 41 | bottom_nav_part 🔸 | 67 | container |
| 31, 33, 37, 39 | title header/footer, grand summaries 🔍 | 68 | text_area |
| 100–102, 200–202 | sub-summaries (part kind 3 → 101) 🔍 | 70 | text_box |
| 50 | line | 78 | button_bar |
| 51 | rectangle | 79 | button_bar_segment |
| 52 | rounded rectangle | 80 | button |
| 53 | oval | 81 | tab_control |
| 54 | any shape (named shape styles) | 82 | tab_panel |
| 83 | portal | 85 | chart |
| 86 | web_viewer | 89 | popover |
| 90 | scrollbar | 91 | dot_control (slide control) |

### Elements and states

| Element | CSS | Element | CSS |
|--:|-----|--:|-----|
| 1 | `.self` | 13 | `.row_alt` |
| 2 | `.inner_border` | 20 | `.scrollbar_top_button` |
| 3 | `.text` | 21 | `.scrollbar_bottom_button` |
| 4 | `.icon` | 22 | `.scrollbar_track` |
| 5 | `.baseline` | 23 | `.scrollbar_thumb` |
| 6 | `.repeat_border` | 26 | `.dot` |
| 8 | `.contents` | 27 | `.dot_container` |
| 9 | `.tab` | 28 | `.row_active` |
| 10 | `.tab_inner_border` | 29 | `.button_bar_divider` |
| 12 | `.row` | | |

States: 1 `:normal`, 2 `:hover`, 3 `:pressed`, 4 `:focus`, 6 `:disabled`,
7 `:checked` (also `:active`; the later CSS rule wins), 15 `:droptarget`,
16 `:activefocus`, 18 `:placeholder`.

## 13. Records ✅

Decoded from records typed into copies of the Inventory and Meetings
templates with FileMaker Pro 26, diffing the file after each change
(`fmdump <file> entries`). Writer: `crates/fmp12/src/save.rs`.

### Values

All values are **text**, numbers, dates, times and timestamps included, in
the file's own formats (the locale below), not the user's:

| Field type | Typed (French UI) | Stored, English file | Stored, French file |
|---|---|---|---|
| Date | `01/02/2026` | `2/1/2026` | `01/02/2026` |
| Time | `9:05:30` | `9:05:30` | `09:05:30` |
| Time | `14:30` | `14:30:00` | `14:30:00` |
| Timestamp (auto-enter) | — | `11/27/2026 2:13:33 PM` | `27/11/2026 14:35:26` |
| Number | `12.75`, `-3.5`, `2024` | `12.75`, `-3.5`, `2024` | |

Text is SCSU, XOR 0x5A (§3); CR separates lines. A value is stored
**inline** at `[T].[5].[R]` with key = field id when its SCSU form has one
byte per UTF-16 unit (ASCII, Latin-1) and at most 1000 bytes (ops 06/07).
Otherwise (an em dash, `✓`, CJK…, or more than 1000 bytes) it moves to
`[T].[5].[R].[F]`:

* key 0 = SCSU byte count, UTF-16 count (u32 BE each), plus a third u32,
  always 4, when the value is over 1000 bytes;
* keys 1…n = the bytes in 1000-byte chunks (a chunk may end inside an SCSU
  sequence, so concatenate before decoding).

FileMaker's SCSU quotes a character of a static window with SQn (`05 14` =
U+2014). BaseCraft encodes ASCII/Latin-1 as single bytes, static-window
characters with SQn and anything else with SQU; re-encoding all the records
FileMaker wrote gives its exact bytes (`tests/records.rs`).

Other record keys: **252** = number of modifications (length-prefixed
integer, `01 01`, `01 02`, written from the second editing session on).
Calculation results of stored calculation fields are stored like any value
(`Units In | Out` = `25`); FileMaker does not recompute them when it opens a
file, so a writer must store them.

**Repeating fields** (from records set by Set Field in FileMaker Pro 26):
repetition 1 is stored like any value, at key = field id. Repetition
*n* ≥ 2 adds a byte *n* to the key: `07 02` = field 7, repetition 2 (a
2-byte key whose top bit is clear, ops 09–0F); `80 03 03` = field 131,
repetition 3 (3-byte key, ops 10–17). Empty repetitions are not stored.
Long values move to the sub-path spelled the same way
(`[T].[5].[R].[0C 02]`). A reader that gets the 2-byte key as an integer
loses the top bit (`07 02` and `87 02` both read as 1922), so BaseCraft
reads it as a repetition when the field repeats that far. Stored
calculations with repetitions store each one.

### Global fields

A global field has one value per table, whatever the record (and with no
record at all): `[T].[8]` (a node, marker `80`) holds it at key = field id,
stored exactly like a record's value (inline, or in `[T].[8].[F]` when long;
repetitions spelled the same). Found in a FileMaker-saved file whose globals
FileMaker showed (a portal filter value, a selected id); a record never stores
a global's value. BaseCraft writes the node back when the values change (the
file then opens in FileMaker 26).

### Containers

A container field's value is **text** in the record:

* a file stored in the file: the id of an entry of the table's container
  store, in upper-case hex, then U+E000 (`01` + `E000` = store id 1; SCSU
  makes it a long value);
* a file stored by reference: its path, `imagewin:/C:/photos/blue.jpg`,
  `filewin:/…`, `filemac:/…` (`GetAsText` gives the same text).

The store `[T].[31]` (`fmp12::containers`) keeps each distinct content
once:

* `[T].[31].[5].[id]`: one node per stored file. Its **streams** are
  long-key items (op `16`/`17`, 4-byte key) named by a tag: `FNAM`
  (u32 1, the kind as length byte + XOR-ed text, `image` or `file`, u16
  length + XOR-ed file name), the data (`PNGf`, `JPEG`, `GIFf`, `FILE` for
  anything else), and for pictures `SIZE` (u16 width, u16 height) and
  `DPI_` (u16 ×2, only when the picture says its resolution). A stream
  over 1000 bytes moves to `[id].[tag]`: key 0 = u32 length, keys 1…n =
  1000-byte chunks. Key 2 = directory: u16 stream count, then tag + u32
  length per stream, in tag order. Key 4 = MD5 over each stream's tag and
  bytes in tag order (FileMaker's dedup key). Key 18 = u32 number of
  record values using it. Key 216 = UUID.
* `[T].[31].[1]`: key 0 = last id (length-prefixed integer), key 4 =
  number of stored files, key 216 = UUID, key 252 = change counter;
  `.[7]` = the set of ids (`00 id` items), `.[8].[MD5]` = id of each MD5.

`GetContainerAttribute ( f ; "all" )` reports Internal Size = Σ (stream
length + 8); for a reference, the length of its path.
FileMaker gives a picture inserted twice the same id (and counts its uses).
BaseCraft's writer (`save_containers`) adds new files, recounts key 18
from the record values and drops files no record uses; files already in
the store keep their bytes.

### External storage ✅ open · ⏳ secure (encrypted)

Decoded from container fields set to "Store container data externally"
in FileMaker Pro 26's Field Options (copies of Tasks, files set by
`Base64Decode` in pasted scripts), then checked by opening files
BaseCraft wrote.

* **Field options** (key 2): byte 10 bit `0x10` = stored externally,
  `0x20` = open storage (else secure); byte 8 bit `0x04` = secure
  storage "with fewer folders". Key 38 = u32 id of the base directory;
  `[39]` keys 4–6 = the open-storage folder calculation, by default the
  text `"Table/Field/"`.
* **Base directories** `[38]` (Manage › Containers): `[38].[5].[id]`
  with a 4-byte path element (`00000000` = id 0), key 16 = path relative
  to the database's folder (`Tasks/` in the templates, `Files/<name>/`
  in new files), key 240 = 1. The thumbnail setting is not stored there.
* **Store entry** `[T].[31].[5].[id]`: the record value is the store id
  as for embedded data; the entry holds `FNAM` and `SIZE` as usual, plus
  * `REMO`: byte 0 = flags (0 open, 1 secure, 9 secure with fewer
    folders), 7 zero bytes, then for open storage the folder
    (`Tasks/ExtOpen/`, plain UTF-8, not XOR-ed) and `00`;
  * the data stream, its tag the embedded one with its case swapped
    (`pngF`, `jpeg`, `gifF`, `file` for other files): u64 little-endian
    file size, 16 bytes, then a name and `00`. Open storage: the file's
    MD5 and its file name. Secure storage: 16 opaque bytes and
    `_<table id>` (`_130`);
  * `ENCR` (secure only): 32 bytes.

  Key 4 (the dedup MD5, §13) is computed as if the file were embedded:
  over `FNAM`, the embedded tag (`PNGf`) with the file's bytes, `REMO`
  and `SIZE`, in tag order.
* **Open storage files** lie at `<database folder>/<base directory>/
  <REMO folder>/<file name>`: `Tasks/Tasks/ExtOpen/red.png`. A second,
  different file of the same name in the folder becomes `red_1.png`
  (the store keeps `red.png` in `FNAM`). `GetAsText` of the field gives
  `remote:red.png¶size:4,3¶PNGf:Tasks/ExtOpen/red.png`;
  `GetContainerAttribute` reports `Storage Type: External (Open)` and
  `External Files: 1`.
* **Secure storage files** lie at `<base directory>/Secure/52/97/
  ABAB0061/B4723A60/0D3C544B/E855_130` (32 hex digits split 2/2/8/8/8/4,
  then `_<table id>`), or with fewer folders `Secure/1/6C1ECDD82C7A2542E8F42C33308F/F52D_130`
  (a one-digit bucket, 28 and 4 hex digits). The files are **encrypted**:
  their length is the original rounded up to the next multiple of 16
  (73 → 80, 15 → 16, 97 → 112: a 16-byte block cipher with padding), the
  same picture stored twice gives different bytes, and the 32 hex digits
  are not the MD5 of the content, of the stored 16 bytes, or of their hex
  text (an encrypted id). The key is FileMaker's own; BaseCraft does not
  derive it and stops there: secure-storage files are not read, and new
  files in a secure field are stored embedded, which FileMaker reads
  (`Storage Type: Embedded`).
* **Thumbnails**: Manage › Containers › Thumbnails says FileMaker makes
  them for large pictures of *hosted* files. With "permanent" chosen and
  an 800 × 600 picture shown in a smaller field, a local file got no
  thumbnail stream and no thumbnail folder: there is nothing to read in
  local files.

BaseCraft (`containers::Stored::new_open`, `engine::Database::
read_external_containers` / `write_external_containers`) reads open
storage files next to the file it opens and writes new ones next to the
file it saves, in the default folder `Table/Field/` of the first base
directory. Checked in FileMaker Pro 26 (Tasks copy with Assignees::Photo
set to open storage, records added by `basecraft-cli new … Photo=@file`):
the file opens without warning and `GetContainerAttribute` gives each
file's name, MD5, size and picture size, `red_1.png` included; a file
BaseCraft put in the secure Attachments::Attachment field reads back as
embedded. FileMaker Server's consistency check (`fms_check.sh`) flags
that file, but it flags a plain BaseCraft rewrite of the same Tasks copy
too, with no change at all (`fmdump … rewrite`): a writer issue, not
the container entries. Bisected to layout 3 (FileMaker had resized its
window): without that layout's 373-byte key-2 item the rewrite passes.
The item only moved the sector boundaries: the packer filled a sector to
its last byte and left out the pops that must close it (§1). Fixed in
`write::pack`.

### Locale (`[2]` key 13)

u32 language (21 English, 24 French, 25 German, 31 Japanese…), 6 bytes, list
separator (1 byte: `,` or `;`), then UTF-16 BE characters: two quote marks,
thousands separator, decimal separator, date separator, time separator; then
`02`, the **date order** (0 M/D/Y, 1 D/M/Y, 2 Y/M/D), 4 format bytes, and
the AM and PM strings (length byte + XOR-ed text; `" AM"`/`" PM"` in
English, empty in French). Verified on the 11 languages of Meetings.

### Record ids and counters

* Record ids start at 1 and are **never reused**: `[T]` key 0 is the last id
  ever used, a length-prefixed integer (`01 28` = 40, `02 01 2C` = 300). It
  does not decrease when records are deleted. FileMaker gives the next new
  record that id + 1.
* `[T].[13]` = the set of live record ids, in the **id-set** form below.
* `[3].[16].[5].[T]` key 251 (u32) grows when the table's records change,
  about once per transaction or session (1→2→3→4, then +39 for 38 new
  records). BaseCraft adds 1 per save.
* `[TO].[7]` (TO element `D0 hi lo`): key 0 `01 02`; `.[1]` key 3 = last
  record id again, an empty node `.[1].[80 0B 80 02 + u32 counter 251]`,
  key FF empty; `.[2]` key 3 `01 00`. FileMaker refreshes it itself: a file
  whose cache is stale opens with the right records and FileMaker rewrites
  it on the next change. BaseCraft leaves it alone.

**Id sets** (also the postings of an index): one id below 256 is a Kv with
a 1-byte key and an empty value (`00 id`). Two or more ids are a bitmap in
key 0, u32 BE words, word *i* bit *j* (least significant first) = id
32*i* + *j*: `{1, 2}` = `03 00 00000006`, ids 1–40 = `05 00 FFFFFFFE
000001FF`. Sets with ids beyond what fits in one value (2040) were not
observed.

### Field indexes (`[T].[1].[F]`)

* `[F]` key 16 = u64 number of entries, u64 number of distinct keys.
  `[F].[0].[key]` = **value index**: one entry per line of the value, the
  key is `01` followed by 2-byte primary collation weights (case and accent
  blind: `é` = `e` = `126B`, `a` = `120F`, `0`–`9` = `1205`–`120E`,
  space `020A`, `-` `0223`, `—` `022C`), so `""` (empty value) = `01`. The
  item is the id set of the records. Keys stop after 109 characters; a
  longer value also gets an entry `FF 80`.
* `[F]` key 17 and `[F].[1].[key]` = **word index**, same layout, built
  when a find needs it (the field then shows "All" indexing).

FileMaker **trusts** a stored index: with a stale one, a related record we
added did not show in a portal (the match field's index did not list it).
A *missing* index on a field marked indexed is read as empty (the portal
showed nothing). But a field marked unindexed with automatic indexing on
(option byte 8, bits 3/5/7 clear, bit 6 clear) gets its index **rebuilt
on demand**, and FileMaker marks it indexed again (Minimal for the portal,
All after a find). So `save_records` removes `[T].[1].[F]` of every field
whose values changed (all fields when a record is added or deleted) and
clears its index bits. Fields with automatic indexing off keep their stale
index (not handled). Writing exact index keys would need FileMaker's
collation table for every index language.

A table that never had records has no records node `[T].[5]`. When
BaseCraft writes the first records it writes the node marker too: without
it, FileMaker adding a record of its own starts the node afresh and the
records BaseCraft wrote disappear (seen in FileMaker Pro 26).

## 14. Schema writing (`fmp12::schema_edit`) ✅

Decoded by doing each change in FileMaker's Manage Database on copies of a
new file and diffing the entries (`fmdump <file> entries`), then checked by
opening files BaseCraft wrote.

### Name keys

The name indexes `[cat].[1].[1]` (tables, occurrences, layouts, fields,
scripts, themes…) are keyed by a collation key of the name: the primary
weights (u16 per collation element: `a` = `120F`, `b` = `1225`, digits
`1205`–`120E`, space `020A`), `00 00`, the secondary weights (one byte per
element, e.g. `13` acute, `16` grave, `37` cedilla; left out when all are
`01`), `00`, then the tertiary weights (left out when all are `01`; set by
fractions, superscripts, ligatures and capital Greek or Cyrillic letters;
Latin case is ignored). `é` is two elements: `126B`/`01` and an accent
with no primary weight and secondary `13`. A second object with the same
name (layouts named `-`) gets its id appended to the key. The weights of
423 characters (ASCII, Latin-1, Latin Extended-A, common punctuation,
Greek and Cyrillic letters) were read from fields named "x" + character
(`crates/fmp12/src/collate.rs`); they reproduce the 4,097 name-index keys
of the templates. Names with other characters get no index entry.
FileMaker opens a file whose name indexes lack entries; it does not
rebuild them.

The Japanese, Korean and Chinese templates keep even short names in the
long form `[node].[16]` (key 0 = length, keys 1…n = text), not in key 16.
A name written inline must replace that subtree: a node with both forms
fails FileMaker Server's consistency check (error 805, "bad blocks").

### A new table

FileMaker's Manage Database › Tables › Create writes:

* the table: `[3].[16]` catalog entries (§4) and node `[3].[16].[5].[T]`
  (16 name, 216 UUID, 251 counter, 252 = 1, metadata 31873–31875 —
  optional);
* a table occurrence of the same name (`[3].[17]`, §7);
* a layout of the same name on that occurrence: `[4].[1]` catalog entries,
  `[4].[1].[7].[L]` (key 2 = `{2: 0x104000 | TO, 3: 0}`, 16 name, 48, 240,
  241) and `[4].[5].[L]`, a copy of the new file's layout with no objects
  (header key 2 field 2 = the occurrence, parts, theme key 11, empty
  object list `00 07`, empty `[8]`, `[10]`, `[21]`, `[1].[D0 hi lo]`);
* the field catalog `[T].[3].[1]` and the fields;
* the graph's evaluation plan `[3].[17].[1].[14].[T]` (rebuilt by
  FileMaker when missing, so BaseCraft leaves it out and drops it whenever
  a table's fields change).

A file must hold its data formats (`[2]` key 134, written the first time
FileMaker closes a file) before it can get a second table: without them
FileMaker reports "the file's access privileges have been damaged".

### Fields

Adding a field writes its node (§5), the catalog entries, the `[6]` type
index, the `[14]` dependencies and, for an indexed field of an empty
table, empty index statistics `[T].[1].[F]` key 16 (and 17 for "All"):
16 zero bytes. In a table with records an index is left for FileMaker to
build (§13). Renaming updates key 16 and both name indexes; deleting
removes the node, its index entries, its values in every record and its
dependencies, and compacts the display order. Renaming a table also
renames the occurrences and layouts that had its name; deleting a table
removes its data and, as when "also delete occurrences" is ticked, its
occurrences and their relationships.

### New files

`schema_edit::new_file` starts from `crates/fmp12/assets/blank.fmp12`, a
file FileMaker created (File › Create › Blank), opened and closed once,
then emptied of its default fields with BaseCraft's own code and given
BaseCraft's plain theme (`examples/mkblank.rs`). A theme whose compiled
styles hold only a few default rules (background, borders, text colour
and size) is enough for FileMaker; one with no rules opens too, but
draws layouts grey.

### Occurrences, relationships and value lists

Decoded from a file where FileMaker's Edit Relationship, Add Table
Occurrence and Manage Value Lists dialogs added occurrences, four
relationships (every operator, both sides sorted, create and delete on
either side) and four value lists, and from fields given lookups and the
other options below in its Field Options dialog (`tools/oracle`,
`Find-FmWindow`, `Click-FmId`: those dialogs expose automation ids).

* **Occurrence**: as for a new table (§7 key 2). FileMaker places a new
  one 30 points right and below the last; BaseCraft puts it to the right
  of the others, 131 × 111 points (occurrences of size 0 count as a row,
  the way FileMaker lays them out). Renaming writes key 16 and the name
  index; deleting removes its node, its relationships and its record
  cache `[TO]`. The last occurrence of a table stays (its calculations
  are evaluated from it).
* **Relationship**: node `[3].[251].[5].[R]` and catalog (§7), then both
  occurrences' links `[251]` and the route table `[3].[17].[1].[8]` are
  rewritten from all relationships, and the evaluation plan
  `[3].[17].[1].[14]` is dropped (FileMaker rebuilds it).
* **Value list**: node `[33].[5].[V]`, catalog entries `[1]`, `[3]`,
  `[6]`, `[7]`, keys 0, 4, 252 (§9).
* **Field options**: lookups (key 8, constant key 11, the `[14].[251]`
  dependency node), "member of value list" (key 7), maximum characters
  (bytes 20–23), custom messages (key 24 and `[32]`), the serial
  increment (bytes 16–19), summary options (key 9 byte 1 and the second
  reference), §5.

Checked in FileMaker Pro 26 (`tools/oracle/graph_demo.sh`,
`template_demo.sh`, `Test-FmGraph`): files built by `basecraft-cli`
open without a warning; Manage Database shows the graph; Save a Copy as
XML shows every relationship option and operator, the lookups, the value
lists and the other field options as written; records FileMaker creates
see their related records through calculations (`Count`, `List`, a `×`
relationship), lookups copy the related value (also the no-match
constant and the next lower value), `ValueListItems` returns the custom
and field values, serials step by the increment.

## 15. Accounts and privileges (`fmp12::security`) ✅

`[23].[1]` accounts, `[23].[2]` privilege sets, `[23].[3]` extended
privileges, `[23].[4]` file access. Each catalog has the usual layout
(§4): `[23].[c].[1]` key 0 last id, key 4 count, `[3]` display order,
`[1]` name index, `[7]` member list; members at `[23].[c].[5].[id]` with
keys 16 (name; long names in `[16]`), 216 (UUID) and 31873–31875 (copied
from the catalog head when a member is added).

### Accounts `[23].[1].[5].[A]`

| Key | Meaning |
|----:|---------|
| 2 | authentication: absent/0 FileMaker, 1 external server, OAuth providers above (`k >> 3`: 2 Amazon, 3 Google, 4 Azure AD, 5 Claris ID, 6 Apple, 7 AD FS) |
| 3 | description |
| 4 | **digest** (below) |
| 6 | password data: `01`, 10-byte salt, 20-byte hash, then `hash[1] & 0x1F` check bytes, byte `31 + i` = byte `11 + i` XOR byte `1 + i` |
| 10 | status: when the password was set, a date (`0x800000 \| year << 10 \| month << 6 \| day`, 3 bytes) then the seconds as a number (`80`, count of integer digits, digits + 1 as nibbles, six fraction digits, trailing zeros dropped), after a length byte. The microseconds carry flags: `1 << 19` inactive, `1 << 18` must change password. No key: 1/1/1024 |
| 11 | privilege set id (length-prefixed integer) |

Account 1 is [Guest]: its name is stored `[` U+E002 `]` (U+E002 is
shown as "Guest") and its password is that text. The catalog head
`[23].[1].[1]` keys 16 and 6 hold File Options › "Log in using" (account
name and password as text); key 6 alone, empty, when the option is off.

**Password hash.** The password's UTF-16 code units, decomposed (é →
e + U+0301; a leading combining mark gets a space first), each XOR-ed
with two bytes of a fixed 30-byte pepper (`FileMaker, Inc.\0` and 14
more bytes, `security::PEPPER`), then PBKDF2-HMAC-SHA1 with the 10-byte
salt, one iteration, 20 bytes. Verified on hashes FileMaker 26 itself
wrote when its own script steps (Add Account, Reset Account Password,
Change Password) set test passwords (`test123`, `Test-é123`, `test456`,
`test789`; `tools/oracle/account_steps.xml`, vectors in
`tests/security.rs`), on the empty password of every starter
template's Admin, and the other way round: FileMaker 26 opens files
whose passwords BaseCraft hashed. BaseCraft checks passwords and writes
new ones (new random salt); it has no way to read one back or to open a
file without one.

*Sources.* The hash construction (pepper, decomposition, one PBKDF2
iteration) and the shape of the digests were suggested by the public
notes of the OpenMaker project (github.com/OpenMaker-Dev/openmaker,
`docs/re/local-files.md`, `network-protocol.md` §4.3–4.4); no code was
taken from it. Every detail here was then checked on our own files and
against FileMaker 26 as described.

### Privilege sets `[23].[2].[5].[P]`

Sets 1–3 are built in: [Full Access], [Data Entry Only], [Read-Only
Access] (names stored in the file's language; `Get (
AccountPrivilegeSetName )` gives the English ones, as FileMaker 26 does
in a French file).

| Key / node | Meaning |
|-----------|---------|
| 2 | records of tables without their own entry: byte 0 = create, view, edit, delete as 2-bit levels from the high bits (0 no, 1 yes, 2 limited), byte 1 = access of fields (1 no access, 2 view only, 3 modifiable) |
| 3 | description |
| 4 | **digest** |
| 10 | other privileges, u16 XOR `0x6000`: `0x1` create value lists, `0x2` no idle disconnect, `0x4` change password, `0x8` manage extended privileges, `0x10` manage accounts, `0x20` manage database, `0x80` manage menus, `0x100` print, `0x200` export, `0x400` override validation, `0x800` create scripts, `0x1000` create layouts, `0x2000`/`0x4000` menus (both: all, `0x2000`: editing only, none: minimum), `0x8000` open quickly |
| 12 | change password every N days |
| 13 | minimum password length |
| `[5].[T]` | custom record privileges of base table T: key 2 as above; `[5]` field id → access byte; `[6]`–`[9]` the calculation limiting create/view/edit/delete (keys 4 context `03 D0 hi lo`, 5 tokens, 6 compiled, §11) |
| `[9]` | layouts, scripts, value lists: key = catalog byte (`04` layouts, `11` scripts, `21` value lists), alone for the default of every item or followed by the item's id; value = one byte `0xD0 \| records << 2 \| access` (layouts: records through the layout in bits 2–3) |

`[23].[2].[1].[8]` indexes the sets by digest (a node named by the
16-byte digest holding one empty key per set id; a copied set shares its
original's node, opened once: a second node marker is a damaged block
for FMDeveloperTool).

### Extended privileges `[23].[3].[5].[E]`

Key 16 keyword (`fmapp`, `fmwebdirect`, `fmrest`, `fmodata`,
`fmreauthenticate10`…), 3 description (absent for the built-in ones,
which FileMaker shows in its own language), 2 `01 xx`, 4 digest, `[5]`
one empty key per privilege set that has it.

**Key 2 switches the set list on.** FileMaker reads `[5]` only when key 2
is `01 01`; with `01 02` or no key 2 the privilege has no set, whatever
`[5]` holds. The second byte enters the digest (2 when the key is
absent). Evidence: `fmreauthenticate10` in every starter template
(`01 01`, sets 1–3); FMDeveloperTool's Save as XML of a BaseCraft file
listed no `PrivilegeSetReference` for an `fmrest` whose `[5]` held set 4
until key 2 `01 01` was added (and none again with `01 02`); FileMaker
Server writes `01 01` itself (below). BaseCraft writes `01 01` with the
list and drops key 2 when no set is left.

**FileMaker Server adds `fmapp` when hosting.** A file in which no set
has a network extended privilege is opened by FileMaker Server 26 only
after it adds `fmapp` to [Full Access] (Event.log 655: « peut désormais
être partagée après l'ajout du privilège étendu [fmapp] à l'ensemble de
privilèges [Accès intégral] »). It writes key 2 `01 01`, `[5]` set 1 and
the digest BaseCraft computes (`c1f11bd3…` for set 1), plus keys 252
(`01 01`) and 31873–31875 of its own. A file where a set already has
`fmrest` is opened unchanged. FileMaker Server also refuses a file whose
accounts have no password (Event.log 1017). The Data API logs in only
accounts whose set has `fmrest`: error 9 for Admin ([Full Access]
without it), 802 for every account while no set of the file has it.

### Digests (key 4)

Each member carries a plain MD5 of its content; FileMaker refuses the
file ("access privileges damaged") when one does not match. Like the
script list's key 8 (§8) they hold no secret: BaseCraft recomputes all
of them for every account, set and extended privilege of the 66 starter
templates, and for the accounts FileMaker 26 added by script
(`Security::check`).

* **Account**: MD5 of [the UUID `[23].[4].[1]` key 16 and `[2]` key
  254, when the file version `[2]` key 5 is above 10], the account key
  as a path (`len + 1, len, bytes`), the key 2 byte, the name in
  UTF-16BE (as stored, `[` U+E002 `]` for Guest), the first 31 bytes of
  key 6, the description in UTF-16BE, then keys 10 and 11, each after a
  length byte.
* **Privilege set**: MD5 of the record levels of key 2 (one byte each;
  a limited level without a calculation counts as no; `0 0 0 0 1` when
  key 2 is absent) and its field byte; key 10 as stored plus `[2]` key
  254 (u32, wrapping); the description in UTF-16BE; keys 12 and 13
  (u16); for each table its key as a path, its levels and field byte as
  above, for each limited action the compiled calculation, the context
  length and the context, then each field entry (key as a path, byte);
  then each `[9]` entry (key as a path, byte). The name is not part of
  it.
* **Extended privilege**: MD5 of keyword and description (UTF-16BE), the
  key 2 byte, then each set key as a path.

### Writing (`security::save`)

A login is required: `Security::login` checks the account name (not
case-sensitive) and the password and returns a `Session`; `save` takes
one. A full access session may change anything; otherwise only what
FileMaker's script steps allow (a set with "manage accounts": accounts
outside [Full Access]; any account: its own password). One active
[Full Access] account must remain. Changed members are rewritten with
their digest, the name index (§14 collation keys), display order, count
and last id; untouched members keep their bytes. "Log in using" is
turned off when its account's password or name changes.

Checked in FileMaker Pro 26 (`tools/oracle/security_check.ps1`,
example `secdemo`): a copy of Tasks where BaseCraft added a set
"Clerks" and four accounts opens without warning. alice (Clerks,
"test123") logs in and the no-access layout shows « <Accès refusé> »;
a wrong password and the inactive bob get FileMaker's alert; carol
(must change) gets FileMaker's « Modifier mot de passe » dialog; Admin
logs in with the password BaseCraft set. Save a Copy as XML shows Clerks
as written: Tasks create and view, edit limited by `Tasks::Status ≠
"Done"`, no delete, one field view only and one no access, the other
tables view only, "Task Details" no access and the other layouts view
only, the first script no access and the others executable, value lists
view only, print yes, export no, editing menus only; the accounts with
their sets, carol's "change password on next login".

### Account script steps

As FileMaker 26 stores them (steps it wrote from a pasted snippet, and
copies with bytes changed in place, read back through Save a Copy as
XML):

| Step (id) | Storage |
|-----------|---------|
| Add Account (134) | `[128].[5]` name, `[129].[5]` password, privilege set inline at byte 6 (`02 01 id`), bytes 22–23 = 1 user must change password; options `0x4002` |
| Delete Account (135) | `[128].[5]` name |
| Reset Account Password (136) | `[128].[5]` name, `[129].[5]` password, bytes 22–23 must change |
| Enable Account (137) | `[128].[5]` name, bytes 22–23 = 1 deactivate (XML `Deactivate`), 0 activate |
| Change Password (83) | `[128].[5]` old password, `[129].[5]` new password, option `0x80` without dialog |
| Re-Login (138) | `[128].[5]` account name, `[129].[5]` password, option `0x80` without dialog |

Errors FileMaker 26 gave running them (and so does the engine): 212 for
a missing account (Delete, Reset, Enable) and for a refused Re-Login (the
account stays logged in), 12 for Add Account with a name in use, 9 for
Add Account by an account that may not manage accounts. At open, a wrong
password and an inactive account get the same alert: « Le nom de compte
et le mot de passe fournis ne permettent pas d'accéder à ce fichier. »

## 16. Custom functions (`fmp12::custom_functions`) ✅

Decoded from custom functions pasted into FileMaker Pro 26's Manage
Custom Functions (clipboard format `Mac-XMFN`, the classic
`<CustomFunction functionArity visible parameters name><Calculation>`
snippet; a `<Group name>` makes a folder) and from Set Variable steps
calling them, then checked on files BaseCraft wrote.

Catalog `[16]`, laid out like the script catalog (§8):

* `[16].[1]`: header (§4): key 0 last id, key 4 count, key 32 = `01`,
  key 216 UUID, key 252 change counter, 31873–31875; `[1]` name index
  (collation keys, §14), `[3]` display order, `[7].[id]` one member per
  entry: key 16 name, key 240 = u64 1 for a folder, 2 for the end of a
  folder (named `--`), absent for a function. No integrity digest (no
  key 8, unlike scripts). A file without custom functions has no `[16]`.
* `[16].[5].[id]`: key 16 name; key 32 = `02` + u16 number of
  parameters; key 33 = the parameter names joined by `;` (absent without
  parameters); key 36 = availability, `01` all accounts, `00` accounts
  with full access only (`access="FullAccessOnly"` in the XML); `[35]`
  keys 5 and 6 = the calculation (§11, no key 4 context: fields are
  written `TO::Field`); 216 UUID, 252 counter. Folders have only key 16,
  216, 240 and 252.

In a calculation:

* key 5: `1B 14 len 01 id` (the id as a path element, as for a field
  reference), then the usual `(` `;` `)` tokens; a function without
  parameters is written without parentheses (`Hello`);
* key 6: `1B`, u16 argument count, `14 len 01 id`, then the arguments in
  reverse order. A call that gives the calculation's result is `1C`:
  the whole calculation, the results of `If`, `Case` and `Choose`, the
  body of `Let`, the result of `While`, and parentheses pass the result
  position on; operands, function arguments, `If`/`Case` tests and Let
  values do not (`Hello & 1` → `50 1B…`; `Sq ( Sq ( 2 ) )` → `1C…1B…`).
* Inside a custom function, a parameter is written like a Let variable:
  key 5 `0B len name 00 00`, key 6 `18 u16 slot`, slots 0… in parameter
  order; Let variables inside count on from there.

Evaluation (FileMaker Pro 26 through `Evaluate`, `tools/oracle/
calc_oracle.ps1`): parameters are evaluated in the caller; `Get`
functions see the caller's context; too few or too many arguments are
errors 1201 and 1202. One calculation may make **49,999** custom
function calls, nested (`Deep ( 49998 )`, 49,999 frames) or one after
the other (`Tail ( 20000 ; 0 ) & Tail ( 29998 ; 0 )`, 50,000 calls,
fails): the 50,000th call makes the result `?`, which `EvaluationError`
and `IsEmpty` do not catch. An `Evaluate` counts its own calls
(`Evaluate ( "Tail ( 30000 ; 0 )" ) + Evaluate ( "Tail ( 30000 ; 0 )" )`
= 60000). There is no separate 10,000-deep limit in FileMaker 26. The
`calc` crate grows its stack (`stacker`) to nest that deep; on wasm,
calls nest at most 2,000 deep.

Writing (`schema_edit::Editor::add_custom_function`, `set_…`,
`delete_…`) creates the catalog when the file has none, keeps folders,
adds new functions at the end, and compiles the calculation with the
function itself known (recursion). Renaming keeps the id, so callers
keep calling it. Checked in FileMaker Pro 26 and FileMaker Server's
consistency check on a Tasks copy where `basecraft-cli functions`
added seven functions (one renamed, one deleted) and a calculation
field calling six of them: it opens without warning, Manage Custom
Functions lists them (parameters, availability) and Save as XML shows
their calculations; FileMaker's Evaluate of 15 expressions, the stored
calculation field (`120/5/Hello/100/49/Admin/Hello`) and one depending
on a field (`25/120`) give BaseCraft's results.
