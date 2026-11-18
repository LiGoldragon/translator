# Upgrades

## translator-client 0.2.0 and translator-serving 0.2.0 — every exchange ends, a CLI takes its datom alone, one Nexus bootstrap

Both take the nexus crates at 0.2.0 (`fb3441f952ecba19dd3e9f288a39e7275f8ae700`),
where every exchange ends with `Delivery::End` and the lane `Kind` gains
`Ended`. `translator` stays 0.5.3 here, and a consumer keeps 0.5.2
(`29601a3`). The flake moves onto build-base `bab0819`, which registers the
nexus crates at 0.2.0.

**Every exchange ends.** `NexusCalling::ask` and `NexusCalling::follow`
become one `NexusCalling::call(socket, patience, capacity, answered)`, which
hands each answer on until the Nexus ends the exchange
(`nexus_querying::ExchangeFollowing`). The translator writes each text
answer back and then `Ended`, or `Failed` with the refusal or failure that
ended it; a Nexus that closes before the end is a failure. `Answers` and
`PublicTranslating::answers` are gone: the translator no longer knows which
answer is a frame. `translator-client` reads until `Ended` or `Failed`, and a
connection closed before either is a broken lane, not the end.

**A CLI takes its datom alone.** `TextInvocation::from_arguments::<Nexus>(lane,
args, &configuration)` reads `'<datom>'` alone and places the lane in the
place the configuration names; `TextCommand::run::<Nexus>(lane, args)` is a
CLI's whole `main`, an `ExitCode`. The configuration is
`ConfigurationFile::of_process::<Nexus>()`,
`<configuration home>/nexus/<nexus-name>.datom`, holding
`ClientConfiguration.{ PlaceName }` (`crates/client/ethos/client.ethos`), `{
<place-name> }`; `ConfigurationRefusal.[ Unlocated Missing Unreadable Invalid
]`, printed as its datom naming the file, exits 2 (`RefusalReporting::EXIT`),
as a usage refusal does, and nothing is sent. The invocation is compiled in
under `datom`, which now turns on `shared-translator/datom`.

**One Nexus bootstrap.** `PairLaunching` is gone: `PairRunning::launch`
takes no type parameter and writes the Nexus `nexus_serving::NexusBootstrap`
(re-exported, with `InstanceId`), its place, admission, the translator's
identity and a fresh `InstanceId` (`FreshInstance`, sixteen bytes of the
kernel's randomness as thirty-two hexadecimal digits). The translator's
`TranslatorBootstrap` carries the text limits its own text budget starts at.

**The translator's settings in forwarded readouts.**
`MetaTranslating::meta_answered` takes `&self`, so the translator merges its
own settings into a forwarded readout; `MetaTranslating::announced(&self,
answered)` names the frame its own answer is carried into every open
subscription as. `Settling` gains `Subscribed(Forwarded)`: forwarded as a
subscription of the Nexus's, followed while the Nexus keeps it open, each
answer of it written under one lock with the translator's announcements
(`MetaSubscriptions`, a new field of `Connection`). `TranslatorError` gains
`Refused(Refusal)`, whose text is the refusal's.

To adopt: repin both crates and the nexus crates to 0.2.0 together; drop
`answers`; give `meta_answered` `&self` and write `announced`; settle a
subscription as `Settling::Subscribed`; drop `PairLaunching` and read the
Nexus's bootstrap as `NexusBootstrap`; give `Connection` its
`subscriptions`; make a CLI's `main` `TextInvocation::run`, turning on
`translator-client/datom`, and write each CLI's place into its configuration.

## translator-client 0.1.1 and translator-serving 0.1.1 — the socket path is the exchange's

`TranslatorPlacing::at` takes the translator's socket path from
`nexus_exchange::SocketPaths` (`translator` for the public text lane,
`meta_translator` for the meta) and no longer computes it from the socket
names and the place's runtime directory itself. The path is the same, and
nothing a consumer calls changes. `translator-serving` changes no code: it
names `SocketPaths` through `nexus-serving`'s re-export as before.

Both take the nexus crates at 0.1.1 (`63a2d477ca194913c847d15f227637d84fbe9aeb`),
so a consumer locks one revision of them. `translator` stays 0.5.3 here, and
a consumer keeps 0.5.2 (`29601a3`). The flake moves onto build-base
`88fec4b`, which registers the nexus crates at 0.1.1.

To adopt: repin `translator-client` and `translator-serving` to this
repository's revision and the nexus crates to `63a2d47`, together.

## 0.5.3, translator-client 0.1.0 and translator-serving 0.1.0 — what every translator does alike

The repository becomes a workspace. `translator` moves to 0.5.3 with no change
to its code, its features or its dependencies; its manifest gains only the
`[workspace]` table. The version moves because the new members take
`translator` 0.5.2 from git: the workspace then holds this crate and that
revision of it, and two packages of one name and one version give crate2nix
package identities that carry the checkout's absolute path, so the committed
`Cargo.nix` would differ from the one the build base's check generates. A
consumer keeps 0.5.2; there is nothing to do for 0.5.3.

Two new members hold what every translator process did by hand:

- `translator-client` 0.1.0: `TextClient` with `TextLane`, `TranslatorPlacing`
  (a text lane's socket in a place, named after the Nexus), `TextRequesting`
  (`request`, `request_stream`), `ClientError`, and `TextInvocation` with
  `Invoking` and `AnswerPrinting` for a CLI's whole invocation. Feature
  `datom` derives Datom on `TextLane`; nothing else needs it.
- `translator-serving` 0.1.0: the traits a translator implements, `Translator`,
  `PublicTranslating`, `MetaTranslating` (with `Settling`),
  `EmissionAnswering` and `PairLaunching`; the serving, `TranslatorServing`,
  `Connection` with `ConnectionServing`, `NexusCalling` and
  `EmissionRestoring`; the text budget, `ValidTextBudget` with `Budgeting` and
  `TextBudgeting`; the bootstrap, `TranslatorBootstrap` with
  `BootstrapReading`; and the pair launcher, `Pairing` with `PairReading` and
  `PairRunning`. Its kinds are declared in `crates/serving/ethos/serving.ethos`.
  Reading text is per consumer: its `text` feature, which a translator turns
  on, compiles in the lanes, the traits and the pair launcher and turns on
  this crate's `text`; without it the crate holds its kinds, its errors and
  its calls and links no Datom. Its `datom` feature derives Datom on its
  kinds.

Both take this crate as `translator` at 29601a3 from git, and the nexus
crates at 8976179, the revisions a consumer already locks. A client-facing
failure names no component: an exchange error a translator answers with reads
`Nexus exchange error`, and a CLI's reads `translator client error`.

The flake moves onto build-base 6514496 and ethos-zero 30.0.1 (`2999292`),
which generates `identity.rs` and `placement.rs` unchanged.

How to adopt: depend on `translator-client` from the CLI and on
`translator-serving` from the translator process, at this repository's
revision, and implement the five traits for the translator's own type.

## 0.5.2 — signal 11.2.0

Only the signal pin moves: signal 11.2.0 (4f5cbb9), which adds `signal::Following` and changes nothing this crate uses. A consumer repins signal with it, so `links = "signal"` finds one package in its graph.

## 0.5.1 — signal 11.1.0

Only the signal pin moves: signal 11.1.0 (b145ddb), which adds the subscription Frame and Stream and changes nothing this crate uses. A consumer repins signal with it, so `links = "signal"` finds one package in its graph.

## 0.5.0 — signal 11.0.0

Only the signal pin moves: signal 11.0.0 (6d4eb74), which carries no transport feature and no tokio. A consumer repins signal with it, so `links = "signal"` finds one package in its graph.

## 0.4.2 — the datom feature carries signal/datom

The `datom` feature now also turns on signal's `datom` feature, the edge the build base's feature set asks of a repository that builds signal with it. The pins stay: signal 10.4.1 (d77169d), datom-codec 09a0526, protos c93b598. No other change.

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
