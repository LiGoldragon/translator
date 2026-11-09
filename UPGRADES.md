# Upgrades

## 0.4.1 — datom-codec 09a0526, protos c93b598, signal 10.4.1

Only the pins move: datom-codec 0.33.1 (09a0526), protos 0.33.0 (c93b598) and signal 10.4.1 (d77169d), with the build-dependency ethos-zero 22.0.0 (1976e45). The generator input follows this repository's build-base. A consumer repins signal with them, so `links = "signal"` finds one package in its graph.

## 0.4.0 — datom-codec 0bbeb11, protos 4024614, signal 10.4.0

Only the pins move: datom-codec 0.33.1 (0bbeb11), protos 0.33.0 (4024614) and
signal 10.4.0 (7e6dfe5). A consumer repins signal with them, so `links = "signal"`
resolves to one package, and runs its tests. The generated identity and placement
contracts are checked with ethos-zero daa22ee.

## 0.3.2 — one operation on a prepared place

`ScopedPreparation::with_prepared_place` runs a caller's operation on a prepared place.
It is a separate trait, so `Preparable` and its implementors are unchanged, and
`prepare_for_start` is now exactly `with_prepared_place` over an empty operation.

- **Standard place**: the operation runs while the same exclusion `prepare_for_start`
  takes, the lock `nexus-place-locks/<name>` of the same two-plan preparation, is still
  held. The guard is released when the call returns, on success, on an operation error
  and on unwind, before the caller starts anything of its own lifetime. Cooperating
  callers that do their own filename work inside the operation therefore serialize with
  each other and with the legacy move.
- **Explicit place**: the operation runs after directories are created, with no lock.
  Concurrent operations on one explicit destination are not serialized, and
  `PreparedPlace::mode` says so: `MigrationExclusive` for a standard place, `CreateOnly`
  for an explicit one.
- `PreparedPlace` lends the runtime and state paths and the mode for the call, is built only
  by this module and owns neither the place nor the lock. The operation returns an owned
  result; a preparation failure is `Held::Preparation`, an operation failure is
  `Held::Operation`, and a failed preparation never runs the operation.

The exclusion is not ownership of the place for its lifetime, it does not stop other
writers, and an alias the process cannot observe is outside it. No text, archive or
wire layout changes.

The placement code and its tests depend on the Unix target, not on a Cargo feature, and the flake runs `scoped` and `preparation` under the default feature set as well as the whole suite under all features.

How to deploy: repin translator 0.3.2 in the consumer; no new directory appears.

## 0.3.1 — shared exclusion for place preparation

`Preparable::prepare_for_start` keeps its signature and its public error type. It now runs one of two plans, told apart by the place itself.

- **Standard place**: the state path is `<root>/nexus/<name>`, an absolute path of normal components whose last component is the place name. On Unix the root is opened once by pathname, following symlinks as before, and verified: a directory owned by the effective user with no group or other write bit. A missing root is created beneath its deepest existing ancestor, which is verified the same way or is a sticky shared directory such as a temporary parent whose child the process then owns, one component at a time, at mode 0700. Every later step is relative to a retained descriptor with no-follow: `nexus`, the lock directory `nexus-place-locks` and the lock file `<name>` (both 0700 and 0600, close-on-exec, one link, never unlinked), the legacy and current leaves. Existing directories and leaves are verified by descriptor: real directories, owned by the user, not writable by group or others; `0755` directories the old preparation created under a common umask stay valid. A creation that finds the entry already present is treated as present and verified. The preparer takes an exclusive advisory lock (`File::lock`) on `nexus-place-locks/<name>` and holds it through the existence checks, the move and the directory creations; it is released when the guard drops, before the function returns, and by process death. Two cooperating preparers of one destination therefore serialize: one moves a sole legacy store, the other finds it moved. When both the legacy and the current store exist it returns `StateConflict` and changes nothing. A move that succeeds followed by a failure creating the runtime leaves the destination state intact; a retry moves nothing and completes. The runtime directory of a standard place is created the same anchored way.
- **Explicit place**: any other state path. It only creates the runtime and state directories, never looks for or moves legacy state, and takes no lock. It refuses with `Io` of kind `InvalidInput` when its state path, or the first existing directory on the way to it, is the same directory (device and inode) as the standard destination or legacy store of the same place name, or the `nexus` or `metaflow` namespace directory, under the environment state root: an actual identity conflict between the two plans.

Guarantee and limits. The exclusion serializes cooperating preparers of one configured destination. It is not ownership of the place for its lifetime, it does not stop other writers, and an alias the process cannot observe, for example a bind mount or an explicit path that reaches a standard tree without either being resolvable from the environment, is outside it. A configuration that assigns one directory to both plans is unsupported. On a target without Unix descriptors a standard place keeps the previous unlocked preparation. New refusals on Unix, as `Io` errors: a symlink, a non-directory, a foreign owner or a group- or other-writable directory where a place directory is expected, and an explicit place that is an actual identity conflict. The crate now declares `rust-version = "1.89"` for `File::lock`, and `rustix` with the `fs` feature is a Unix-target dependency. No text, archive or wire layout changes.

How to deploy: repin translator 0.3.1 in the consumer. The lock directory `nexus-place-locks` appears under the state root of a standard place, once, and stays.

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
