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

The format version has not changed since 12, so FileMaker 26 files still say `Pro 12.0`.

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
sector (`… 40 40`).

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
§8) and the accounts and privilege catalogs (`[23]`).
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
| `[17]` | ScriptCatalog; `[17].[5].[S].[5].[n]` = Step | 🔍 |
| `[23].[1]` | AccountsCatalog | 🔍 |
| `[23].[2]` | PrivilegeSetsCatalog | 🔍 |
| `[23].[3]` | ExtendedPrivilegesCatalog | 🔍 |
| `[23].[4]` | FileAccessCatalog | 🔍 |
| `[33]` | ValueListCatalog (§9) | ✅ read, written (§14) |
| `[38]` | BaseDirectoryCatalog | 🔍 |
| `[65]` | CustomMenuCatalog | 🔍 |
| `[T].[3].[5].[F]` | FieldCatalog of table T | ✅ names |
| `[T].[3].[1]` | field catalog header: name index `[1]` (collation keys, §14), order `[3]`, type index `[6]`, dependencies `[14]` (§5) | ✅ |
| `[T]` key 0 | last record id ever used (§13) | ✅ |
| `[T].[1].[F]` | index of field F's values (§13) | ✅ |
| `[T].[5].[R]` | record R: key F = value of field F (§13) | ✅ |
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
  modification stamp; for a summary with a second field, that field
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
| 0–3 | `02 01 n 00`: length-prefixed `01` + path integer *n*, the step's parameter node `[S].[5].[n]` |
| 4–5 | uninitialised (`BE BE` in the templates, anything in new files) |
| 6–19 | inline references (below); bytes between them are uninitialised |
| 20–21 | step id, i16 BE; **negative = disabled** (`FF A3` = disabled Beep) |
| 22–23 | u16: index (0-based) of the step closing the block for If, Else If, Else, Loop, Exit Loop If (End Loop: its Loop); destination kind for Go to Layout (1 original, 3 name by calculation, 4 number by calculation, 5 layout) and Go to Record/Request/Page (1 first, 2 last, 3 previous, 4 next, 5 by calculation) |
| 24–27 | u32 options = XML `<Options>` |

Inline references are length-prefixed: an object id is `01` + path
integer (`02 01 0B` = 11), a table occurrence is `03 D0 hi lo` (§7). A
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
* **`[17].[1]` key 8** (16 bytes) is **not an undecoded hash of the script
  names — it is part of FileMaker's access-privileges protection, and
  BaseCraft deliberately does not forge it** (see the box below). When it
  does not match, FileMaker refuses the file with "the file's access
  privileges have been damaged or altered". It depends on the script
  names (renaming one breaks it), not on script ids (one script "Nouveau
  script" gives `786c087f…62da` with id 1 or id 10), UUIDs, steps or the
  file (the same in a blank file and in an emptied Tasks). A catalog with
  no scripts has no key 8 and opens.

  > **What key 8 is (reverse-engineered with Ghidra, Nov 2026).** The
  > scripts catalog is `Draco::DBLibrary`, which carries the
  > `M_DBCrcIndex` mix-in — the same secure-CRC tamper machinery as the
  > privileges and accounts catalogs (`DBPrivilegesCatalog`,
  > `DBUserAccountCatalog`, `DBAuthFileCatalog`, all of which also have a
  > `HasBeenTampered()` and store a `KSummary` CRC under key 8). Key 8 is
  > that `KSummary`: an MD5 (`Support.dll` `Draco::unistringMD5` and
  > `CRCContext` both hash the UTF-16 **big-endian** code units, standard
  > MD5 init/finalize) over the script members, **seeded by
  > `DBFileUser::InitSecureCRC`**. `InitSecureCRC` folds in the file's
  > *privilege-set identity* (the default `[Full Access]` set's UUID and a
  > file security word), and the sibling `DBAccess::UpdateCRC` /
  > `DBUserAccount::ComputeCRC` fold in the access-control levels and, for
  > protected files, account **password hashes**. In other words key 8
  > cryptographically binds the script list to the access-privilege set:
  > its purpose is exactly to stop a tool from adding, deleting or
  > renaming scripts (which may run with Full Access) outside FileMaker's
  > security system. For the default no-password `[Full Access]` file the
  > seed is a constant (hence blank == emptied Tasks), so the value is in
  > principle reproducible; for files with custom accounts/passwords or
  > encryption the seed is keyed by account secrets we cannot and must not
  > reproduce. Writing a correct key 8 is therefore a forgery of a
  > FileMaker security check, so BaseCraft does not do it: it edits the
  > steps of existing scripts (key 8 unchanged) and leaves adding,
  > deleting and renaming scripts to FileMaker, warning on save when the
  > script list changed. The `unistringMD5`-over-the-plain-names guesses
  > (any encoding, separator, order, case folding, collation keys,
  > length/count prefixes) all miss because the hashed stream is the
  > privilege-seeded member stream, not the bare names.
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

## 10. Layouts ✅ (partial)

Layout data is **Protocol Buffers** (schema-less reader in `pb.rs`):

* `[4].[5].[L]` key 2 = header (2 TO in XML form, 5 style, 7 local CSS, 8 width), key 11 = theme (`01 n`);
* `[4].[5].[L].[3].[P]` key 2 = part (1 kind, 4 top, 5 height, 6 break field, 10 style);
* key 7 (or long value `[7]`) = object list, **front to back** (the first object is on top): repeated field 2 = object
  (1 kind, 2 id, 3 options, 4 bounds {1 top, 2 left, 3 bottom, 4 right},
  5 style {1 local rules, 2 style name, 3 theme object type, 4 display name} (§12), 7 text options,
  8 field {2 {1 TO, 2 field, 3 repetition}}, 16 button);
* `[4].[5].[L].[5].[obj]` key 1 = static text, `.[3]` = button label calculation.

Part kinds (field 1): 1 Header, 3 Leading sub-summary, 4 Body, 7 Footer,
12 Top navigation are seen in the templates (a new layout has 1, 4, 7).
2 Leading grand summary, 5 Trailing sub-summary, 6 Trailing grand summary,
8 Title header, 9 Title footer and 13 Bottom navigation are assumed from
FileMaker's part order and not verified. The break field of a sub-summary
(field 6) is `08 len 00 <TO ref 03 D0 hi lo> len <path-int field>`, e.g.
`08 07 00 03 D0 00 02 01 08` = field 8 of occurrence 130.

### Layout objects in detail

* Object list field 1 (repeated varint) = ids of the **top-level** objects,
  ascending in the templates; children of portals, tab panels and button
  bars are not listed. Field 2 = the objects, front to back.
* The list is stored inline (key 7 of `[4].[5].[L]`, empty = `00 07`) up
  to 1000 bytes; above that in `[4].[5].[L].[7]`: key 0 = total length
  (u32 BE), keys 1…n = 1000-byte chunks (largest inline list seen: 834
  bytes; smallest split one: 1,812).
* Object kinds: 1 field, 2 text, 4 line (style type 50), 5 rectangle (51),
  9 portal, 10 button, 11 tab control, 12 tab panel, 13 web viewer,
  18 button bar. A field's control style is field 8.5 (0 edit box,
  1 drop-down, 2 pop-up, 3 checkbox set, 6 calendar) and its style type
  follows it (61, 62, 63, 64, 66; 67 container).
* `[4].[5].[L].[5]` key 0 = last object id (length-prefixed integer).
* `[4].[5].[L].[5].[obj]`: key 1 text, key 7 (`00 02` fields, `00 04`
  buttons), key 13 (field formatting, `00 00` elsewhere), key 16 = object
  name (empty: `00 10`), keys 216/217 = UUIDs; `.[3]` button label
  `{4: 01 00, 5: tokens, 6: tokens}` (a text literal is `13 len text 00`,
  §11); `.[6]` placeholder; `.[17]` button step parameters.
* `[4].[5].[L].[10]`: a spatial index of the fields, buttons, tab controls
  and button bars: byte-string key = top, left (u16, absolute), id − 1
  (u16); value = `00 len 01 id …` (`len` = length of `01 id`), then
  `00 00 00 00 01` (other tails seen). Portal children are not listed.
* `[4].[5].[L].[21]`: tab order, key n → the same object reference.
* Buttons (object field 16): `{1, 2, 3: step {1 id, 2 arg, 3 options,
  4 14-byte inline block}, 4, 5, 6}`. The inline block is `len 01 id` then
  one byte `BE` or `6F` that does not depend on the target.
* `[4].[1].[7].[L]` repeats the layout's name (key 16) and a message whose
  field 2 is its table occurrence; `[4].[1].[1]` indexes layout names by
  collation key.

### Charts (`fmp12::charts`) ✅ read

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
* Rendering (`print::chart`, shared by Browse, Preview and PDF), measured
  on FileMaker's Browse mode: title 19 pt on top, legend (swatch + series
  name) under it, value labels on both sides of the plot (top and bottom
  for bar charts), a single series coloured per category (Earth:
  `909A3F BD9D7D 4E7E99 CAA465 8E8E8E 7A7841 DAC582 4D7D98 646567 60ABB3
  6F7349 847566`), several series one colour each; positive/negative
  charts show the first series over a blue (positive) and pink (negative)
  background; delimited data are return-separated lists of the current
  record.

### Writing (`fmp12::save::save_layouts`) 🔍

FileMaker writes minimal protobuf, so decode → encode gives the same
bytes for every layout message of the 66 templates (`tests/layouts.rs`).
The writer keeps the raw messages and patches only the fields that
changed: an object's bounds (field 4), style (5), field reference and
control (8), button step (16.3); a part's top, height and style (4, 5,
10); the header's table occurrence (2) and width (8). New objects are
built on an existing object of the same kind and style type (from the
same layout if possible), which keeps the fields we do not understand;
their node copies keys 7 and 13 of the prototype and gets fresh UUIDs.
Deleted objects lose their node and their index and tab-order entries;
moved ones get their index key shifted by the move. A rename rewrites
both name keys but not the name index (collation keys are not
implemented). Not yet verified in FileMaker.

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
| `10` + 19 bytes | number: `02 00 01 00 10`, u32 integer, u32 ×1e-9, u32 ×1e-18, u16 flags |
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
| `1D 00 00 15` + 4 bytes + u16 | library function: `FMDR` design functions (01 DatabaseNames, 02 FieldBounds, 03 FieldIDs, 04 FieldNames, 05 FieldRepetitions, 06 FieldStyle, 07 FieldType, 08 GetNextSerialValue, 09 LayoutIDs, 0A LayoutNames, 0B TableIDs, 0C TableNames, 0D RelationInfo, 0E ScriptIDs, 0F ScriptNames, 10 ValueListIDs, 11 ValueListNames, 12 ValueListItems, 13 WindowNames, 14 FieldComment, 15 LayoutObjectNames, 16 BaseTableIDs, 17 BaseTableNames, 18 BaseTableComment, 19 FieldAnnotation, 1A FieldDisplayNames), `FMMB` 01 Location, 02 LocationValues, 03 RangeBeacons |

A `len` of 128 UTF-16 units or more takes two bytes, the first with its top
bit set (`13 81 14` = 276 units). Unary minus is `26` like subtraction in
key 5. A number literal keeps its value, not its spelling (`1,50` and
`1.5` give the same bytes); its last two bytes are `00 20`, or `00 40`
when it was written with an exponent. The integer part is below 10⁹ in
every literal FileMaker accepted from a pasted script; larger literals
are not decoded. A calculation FileMaker cannot parse is stored as one
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

Confidence: ✅ matched against CSS values across several themes,
🔸 consistent with the CSS but few samples.

| Id | CSS property | Values | |
|---:|--------------|--------|-|
| 1 | background-image | 18 | ✅ |
| 7 | background-color | colour | ✅ |
| 20–23 | border-{top,right,bottom,left}-color | colour | ✅ |
| 24–27 | border-{top,right,bottom,left}-style | 1 none, 5 solid | ✅ |
| 28–31 | border-{top,right,bottom,left}-width | length | ✅ |
| 32–35 | border-{top-left,top-right,bottom-right,bottom-left}-radius | pair | ✅ (corner order 🔸) |
| 36 | border-image-source | 12 | ✅ |
| 64 | font-family | font id − 128 (`[25]`) | ✅ |
| 65 | font-weight | 2 bold | ✅ |
| 67 | font-style | 1 normal | 🔸 |
| 69 | font-size | length | ✅ |
| 70 | color | colour | ✅ |
| 72 | line-height | length (`line`) | ✅ |
| 74 | text-align | 1 left, 2 center, 3 right | ✅ |
| 75 | text-transform | 1 none, 3 uppercase | 🔸 |
| 101–104 | padding-{top,right,bottom,left} | length | ✅ |
| 105–108 | margin-{top,right,bottom,left} | length | ✅ |
| 109, 110 | width, height | length | ✅ |
| 117–120 | top, right, bottom, left | length | ✅ |
| 121 | position | 3 absolute | 🔸 |
| 122 | box-shadow | 19 | ✅ |
| 123 | box-sizing | 1 border-box | 🔸 |
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

**Repeating fields**: none of the templates has one; not yet observed.

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
