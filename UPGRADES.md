# Upgrades

## 0.3.0 — Signal 10.3.0, Datom 0.33.0, Protos 0.33.0, ethos-zero 19.2.0

This is the first upgrade of the text format from 0.2.0, so it takes a minor
version. The 0.2.1 revision (`5b80`) carried the same source; 0.3.0 changes the
version only, and consumers select 0.3.0.

The dependencies on signal, datom-codec and protos are repinned to Signal 10.3.0
(`7afb`), datom-codec 0.33.0 (`eabe`) and protos 0.33.0 (`3c7a`), and the flake
generator to ethos-zero 19.2.0 (`5a5d`). The committed generated Rust is
regenerated: the `i64` identity wrappers additionally derive `Copy`, `PartialOrd`
and `Ord`, and the `String` wrappers additionally derive `PartialOrd` and `Ord`.

`decode` takes the datom-codec 0.33.0 `Budget`, built over a protos 0.33.0
`ReaderBudget`, which is constructed by `ReaderBudget::try_from(limit)` because
its field is private. A consumer repins datom-codec and protos with translator.
