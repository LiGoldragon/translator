# translator

Fresh version 1 reusable translator library. Binary Signal framing and typed
rkyv transport are always available. Enable the `text` feature for Datom text
decoding and encoding; Nexus crates can depend on the binary API without a
Datom dependency.

All external endpoints and CLI vocabulary belong to component translators and
are supplied as typed configuration.

## Nexus places

`resolve_runtime(place_name)` and `resolve_place(place_name)` are pure path
resolution. Runtime sockets resolve below `$XDG_RUNTIME_DIR/nexus/<place-name>`;
state resolves below an absolute `$XDG_STATE_HOME/nexus/<place-name>`, or
`$HOME/.local/state/nexus/<place-name>` when the state variable is unset,
empty or relative.

At Nexus start, call `Preparable::prepare_for_start` once on the resolved
`Place`. It creates both place directories. When only the legacy state at
`metaflow/<place-name>` exists it moves that directory to `nexus/<place-name>`;
when both state directories exist it returns `PlacePreparationError::StateConflict`
with both paths. Legacy runtime directories are ephemeral and remain untouched.
# Translator

The optional `unix` feature supplies policy-free Unix transport mechanics:
absolute progress deadlines, idle-before-first-byte framing, complete-submit
witnesses, kernel peer credentials and pidfd liveness, validated admission,
and classified accept retry. Component socket names, lanes, query routing and
domain configuration remain in each Nexus.

## The text lanes and the pair launcher every translator shares

Two members of this workspace hold what every translator process does alike,
so a translator writes only its own bodies. Each takes this crate from git at
the revision the nexus crates take, so a consumer locks one revision of it.

`translator-client` is the CLI's end. `TextClient::placed::<Nexus>(name,
lane, limit)` resolves the place and the translator socket of a text lane
(`TextLane::PublicText` or `TextLane::MetaText`) named after the Nexus;
`request` returns the first answer and `request_stream` every answer until the
translator closes. `TextInvocation::from_arguments::<Nexus>(lane, args)` reads
a CLI's `<place-name> '<datom>'`, and `print_answers` prints each answer. It
links no Datom and no runtime.

`translator-serving` is the translator process's end; its `text` feature,
which the translator turns on, compiles in the lanes, the traits and the pair
launcher. `TranslatorBootstrap`
is the frame the pair launcher writes on its standard input
(`BootstrapReading`); `TranslatorServing::serve` binds both translator sockets,
pins the paired Nexus, and serves each connection on a thread: the caller from
SO_PEERCRED, a foreign login refused, ordinary text read within the
translator's own budget once it is ready and carried to the Nexus as its call
with the Caller, every answer of a followed query written back as text, meta
text read within the bootstrap budget and settled between the translator and
the Nexus, and an emission from the paired Nexus answered. `Pairing` is the
pair launcher's argument list (`PairReading`), and `PairRunning::launch`
prepares the place, starts both processes, hands each a bootstrap naming the
other and stops the survivor. A translator implements `Translator`,
`PublicTranslating`, `MetaTranslating`, `EmissionAnswering` and
`PairLaunching` for its own type.
