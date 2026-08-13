# The `.febin` format

Version 1. All integers little-endian. All section offsets absolute from the
start of the file and 4-byte aligned. Every offset/count pair is bounds-checked
before use, so a corrupt file can only ever produce a `FormatError`.

The format is designed to be *read in place*: `ProcedureDatabase` borrows the
byte slice and never copies or allocates, which is what lets an aircraft embed
one with `include_bytes!` and pay nothing at load beyond validation.

## Layout

```
+------------------+  0
| header (80)      |
+------------------+  80
| procedure records|  32 bytes each, sorted by identifier
+------------------+  (aligned to 4)
| symbol records   |  12 bytes each
+------------------+  (aligned to 4)
| control records  |  16 bytes each
+------------------+  (aligned to 4)
| position table   |  4 bytes each (u32 string id)
+------------------+  (aligned to 4)
| string index     |  4 bytes × (string_count + 1)
+------------------+  (aligned to 4)
| string blob      |  UTF-8, not NUL-terminated
+------------------+  (aligned to 4)
| code             |  bytecode for every procedure
+------------------+  total_size
```

## Header

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | magic, `FEBC` |
| 4 | 2 | `format_version` (1) |
| 6 | 2 | `header_size` (80) |
| 8 | 4 | `total_size` |
| 12 | 4 | `content_hash` (FNV-1a 32 over everything after the header) |
| 16 | 4 | `flags` |
| 20 | 4 | `procedure_count` |
| 24 | 4 | `procedure_offset` |
| 28 | 4 | `symbol_count` |
| 32 | 4 | `symbol_offset` |
| 36 | 4 | `control_count` |
| 40 | 4 | `control_offset` |
| 44 | 4 | `position_count` |
| 48 | 4 | `position_offset` |
| 52 | 4 | `string_count` |
| 56 | 4 | `string_index_offset` |
| 60 | 4 | `string_blob_offset` |
| 64 | 4 | `string_blob_len` |
| 68 | 4 | `code_offset` |
| 72 | 4 | `code_len` |
| 76 | 4 | reserved, zero |

Flags: bit 0 (`CONTENT_HASH`) means `content_hash` is populated.

`header_size` is stored rather than assumed so a later version can append
fields; a reader locates sections through the offset table, never by arithmetic
on a hard-coded header length.

`total_size` may be smaller than the slice handed to `from_bytes`. A host is
free to append its own data after a database, or to embed one inside a larger
archive; the reader trims to `total_size` and reports it from `size_bytes()`.

## Procedure record (32 bytes)

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | `id` string id |
| 4 | 4 | `name` string id |
| 8 | 4 | `description` string id, or `0xFFFFFFFF` |
| 12 | 4 | body offset into the code section |
| 16 | 4 | body length |
| 20 | 4 | trigger offset into the code section |
| 24 | 4 | trigger length (0 = no trigger) |
| 28 | 1 | category (0 normal, 1 abnormal, 2 emergency, 3 reference) |
| 29 | 1 | priority |
| 30 | 2 | revision |

Records are sorted by identifier and identifiers are unique, so
`get_procedure` is a binary search with no allocation — cheap enough to call
from a tick, though a host should cache the index.

The record deliberately does **not** store the procedure's required stack
depth. The verifier derives it, and a stored value would be one more field a
malformed file could lie about.

## Symbol record (12 bytes)

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | name string id |
| 4 | 4 | host tag |
| 8 | 1 | type (0 bool, 1 f32) |
| 9 | 3 | padding, zero |

## Control record (16 bytes)

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | name string id |
| 4 | 4 | host tag |
| 8 | 1 | kind (0 switch, 1 valve, 2 selector, 3 analog, 4 checklist) |
| 9 | 1 | position count |
| 10 | 2 | index of the first position in the position table |
| 12 | 4 | padding, zero |

An unrecognised kind reads back as `ControlKind::Unknown` and is passed through
to the host rather than being guessed at.

## Strings

`string_index` holds `string_count + 1` offsets into the blob; string *i*
occupies `index[i]..index[i+1]`. The final entry equals `string_blob_len`, so
the index is self-checking: it must be monotonic and end exactly at the blob's
end.

The whole blob is validated as UTF-8 once at load, and each individual slice is
checked to be a character-boundary pair. Accessors then hand out `&str` with no
`unsafe` and no per-tick re-validation.

Strings are interned: the same text appearing in twenty procedures is stored
once.

## Determinism

The same sources plus the same registry produce byte-identical output, on any
machine, in any directory, in any file order. That is a shipping requirement —
a reproducible build lets you prove the `.febin` in a release is the one built
from the tagged sources.

It is achieved by:

* sorting procedures by identifier **before** lowering, so every id assigned
  afterwards depends only on that fixed traversal;
* interning strings, symbols and controls in first-use order over that
  traversal;
* using `BTreeMap` throughout the compiler — never a `HashMap`, whose iteration
  order is seeded per process;
* writing nothing derived from time, paths, addresses or the host. Source unit
  *names* appear in diagnostics but never in the output.

## Validation

`ProcedureDatabase::from_bytes` checks, in order:

1. slice at least `HEADER_SIZE`, magic, version, `header_size`, `total_size`;
2. every count is addressable (`≤ u16::MAX` where the bytecode indexes it);
3. every section offset is aligned and its extent inside the file;
4. the content hash, if flagged;
5. the string index is monotonic and the blob is UTF-8;
6. every symbol, control and position record resolves;
7. every procedure record: strings resolve, identifiers are non-empty, sorted
   and unique, code extents are inside the code section;
8. every procedure's bytecode, in full — see [`verification.md`](verification.md).

After it returns `Ok`, execution cannot encounter a malformed instruction.
There is no lazy validation anywhere: the aircraft pays the cost once, at load,
rather than discovering a problem mid-approach.
