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
| 14 | 2 | free bytes in the payload (4076 − used) |
| 16 | 4 | write sequence number (FileMaker's consistency check resets the maximum) |
| 20 | 4076 | payload (byte code, §2) |

Sector *k* starts at offset `k × 4096`. For sector 1 (the B-tree root), the
"next" field holds the number of body sectors, so
`(next + 1) × 4096 == file size`.

The sectors form a B-tree. All data lives in the leaves (level 0), which are
chained through prev/next. The chain starts at the leaf whose prev is 0.
Index sectors (level ≥ 1) map separator keys to child sectors. Readers can
ignore them, but **FileMaker validates them**: a file whose root index was
rebuilt naïvely ("path of first chunk + key → child") is rejected with "file
access permissions damaged". An entry is a run of path pushes, some with the
pop-after flag (`E0 xx`, `F8 len …`), then a Kv whose 4-byte value is the child
sector id (`03 03 00 00 00 4E` = key 3 → sector 78). `1B 00 01 id` is used
for the leftmost child and for children whose first chunk has a long key.
Read in emission order, the pushes give the child's first path. How the
pop-after flags interact across consecutive entries is still open
(`tools/oracle/idxdec.py`). Until that is settled, OpenMaker only edits files
in place.

Every leaf payload starts from an empty path and pushes the full path of its
first chunk. A chunk never spans two sectors.

**In-place edits work.** Copies of real files with single bytes changed
(same length) open in FileMaker without complaint: there is no checksum.
We use this to reverse-engineer field semantics (`tools/oracle`).

## 2. Payload byte code ✅

Each payload is a stream of opcodes. Each sector rebuilds its *path* (a stack
of byte strings) from an empty stack. Chunks are:
*Push* / *Pop* (path), *Data* (anonymous), *Kv* (integer key → value),
and *KvLong* (bytes key → value). Opcodes 07/0F, which fmptools reads as "segments", are plain Kv chunks with a 2-byte length. Values over ~64 KB are stored in a sub-path as keys 1…n.

If `op & 0xC0 == 0xC0`, the chunk is decoded as `op & 0x3F` and followed by a Pop.

| Op | Chunk | Encoding after the op byte |
|----|-------|----------------------------|
| 00 | Data / end | 1 byte. `00 00` or end of buffer ends the payload |
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
| 80, 88 | no-op | — |

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

* `[cat].[1]`: catalog header (key 216 = catalog UUID);
* `[cat].[5].[id]`: one object per id. Common keys: **16** name,
  **216** UUID, **217** source UUID, **31873** user name,
  **31874** account name, **31875** timestamp (modification metadata);
* `[cat].[5].[id].[5].[n]`: child objects (script steps, layout objects).

| Path | XML catalog | Status |
|------|-------------|--------|
| `[2]` | file-level settings | 🔍 |
| `[3].[16]` | BaseTableCatalog | ✅ names |
| `[3].[17]` | TableOccurrenceCatalog | 🔍 |
| `[3].[251]` | RelationshipCatalog | 🔍 |
| `[4]` | LayoutCatalog; `[4].[5].[L].[5].[n]` = LayoutObject | 🔍 |
| `[6]` | ThemeCatalog (§12) | ✅ styles |
| `[25]` | font catalog (§12) | ✅ names |
| `[17]` | ScriptCatalog; `[17].[5].[S].[5].[n]` = Step | 🔍 |
| `[23].[1]` | AccountsCatalog | 🔍 |
| `[23].[2]` | PrivilegeSetsCatalog | 🔍 |
| `[23].[3]` | ExtendedPrivilegesCatalog | 🔍 |
| `[23].[4]` | FileAccessCatalog | 🔍 |
| `[33]` | ValueListCatalog | 🔍 |
| `[38]` | BaseDirectoryCatalog | 🔍 |
| `[65]` | CustomMenuCatalog | 🔍 |
| `[T].[3].[5].[F]` | FieldCatalog of table T | ✅ names |
| `[T].[3].[1]` | field index (collation keys) | 🔍 |
| `[T].[5].[R]` | record R: key F = value of field F | ✅ |
| `[T].[5].[R].[F]` | long value of field F, keys/segments 1…n (key 0 = style runs) | ✅ |
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
| 14 | 7 | custom error message |
| 15 | 0 | validation by calculation |
| 15 | 1 | `alwaysValidate` (validate even when not modified) |
| 15 | 2 | user **cannot** override |
| 15 | 3 | not empty |
| 15 | 4 | unique |
| 15 | 5 | existing value |
| 15 | 6 | range |
| 15 | 7 | custom message |
| 24–25 | — | maximum repetitions, u16 BE |

Bytes 2, 12–13 and 16–23 changed nothing visible in the XML.

## 6. Long values

A value longer than ~1000 bytes is moved from key *K* of a node into the
sub-path `[node].[K]`, as keys 1…n (key 0 = total length as u32 BE, or
formatting for text). Short values stay inline. Readers must accept both.

## 7. Relationship graph ✅

* Table-occurrence reference: `03 D0 hi lo`, id = 128 + hi:lo. FileMaker's
  XML shows `0x104000 | id`.
* `[3].[17].[5].[TO]` key 2: `02 01 00 | len 02 path-int(base table) |
  f32 left, top, width, height (BE) | u32 collapsed height | u32 | RGBA`.
* `[3].[251].[5].[R]` key 2: `04 TOref(left) 04 TOref(right) | sort spec… |
  flags`. The last byte holds the flags: 0x10 cascade-create right, 0x20 cascade-create left.
  `[R].[3]` keys 1…n = join predicates `op | len 01 field | len 01 field`
  (op 0 = `=`).

## 8. Scripts ✅

`[17].[5].[S]`: key 16 = name, key 240 = options (u64: 8 script, 9 folder,
2 separator), key 4 = step list made of 28-byte records:
`02 01 index 00 | BE BE | … | u16 step id @20 | u16 | u32 options @24`.
The trailing zeros of the last record are trimmed.
Step parameters live in `[S].[5].[n]`, e.g. a calculation in `.[128].[5]`.

## 9. Value lists ✅

`[33].[5].[V]`: key 16 = name, key 3 = custom values (CR-separated),
key 2 = definition: byte 0 = source (0 custom, 1 field), then for a field:
`flags | 04 TOref | len 01 field | …`.

## 10. Layouts ✅ (partial)

Layout data is **Protocol Buffers** (schema-less reader in `pb.rs`):

* `[4].[5].[L]` key 2 = header (2 TO in XML form, 5 style, 7 local CSS, 8 width), key 11 = theme (`01 n`);
* `[4].[5].[L].[3].[P]` key 2 = part (1 kind, 4 top, 5 height, 6 break field, 10 style);
* key 7 (or long value `[7]`) = object list: repeated field 2 = object
  (1 kind, 2 id, 3 options, 4 bounds {1 top, 2 left, 3 bottom, 4 right},
  5 style {1 local rules, 2 style name, 3 theme object type, 4 display name} (§12), 7 text options,
  8 field {2 {1 TO, 2 field, 3 repetition}}, 16 button);
* `[4].[5].[L].[5].[obj]` key 1 = static text, `.[3]` = button label calculation.

## 11. Calculations ✅

Stored next to their owner as keys 4 (context: a TO ref `03 D0 hi lo`, or
`01 00` in scripts and layouts), 5 (token stream that keeps the source text,
whitespace included) and 6 (compiled form, not needed for display).

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
