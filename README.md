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

`translator-client` is the CLI's end. A CLI takes its one datom alone:
`TextInvocation::run::<Nexus>(lane, args)` reads the place from the CLI's
configuration, the one datom `$XDG_CONFIG_HOME/nexus/<nexus-name>.datom`
(`~/.config/nexus/<nexus-name>.datom` when the variable is unset), `{
<place-name> }` (`ConfigurationFile`, `ConfigurationReading`), resolves it
under `$XDG_RUNTIME_DIR/nexus/` to the translator socket of the text lane
(`TextLane::PublicText` or `TextLane::MetaText`) through
`nexus_exchange::SocketPaths`, and prints each answer until the translator
ends them. A configuration missing, unreadable or invalid is refused naming
the file, `Missing.<path>`, `Unreadable.{ <path> <reason> }` or `Invalid.{
<path> <reason> }`, with exit 2, and nothing is sent. `request_stream`
delivers every answer until `Ended`, and `Failed` is the error carrying its
text; `request` returns the first answer. The invocation reads Datom, under
the `datom` feature; it links no runtime.

`translator-serving` is the translator process's end; its `text` feature,
which the translator turns on, compiles in the lanes, the traits and the pair
launcher. `TranslatorBootstrap` is the frame the pair launcher writes on its
standard input (`BootstrapReading`), its text limits the budget the
translator starts with, which the launcher takes from its configuration;
`TranslatorServing::serve` binds both translator sockets, pins the paired
Nexus, and serves each connection on a thread: the caller from SO_PEERCRED,
a foreign login refused, ordinary text read within the translator's own
budget once it is ready and carried to the Nexus as its call with the
Caller, every answer of its exchange written back as text until the Nexus
ends it, then `Ended`, or `Failed` with what ended it. Every exchange ends,
a one-answer exchange after its answer, so the translator needs no
knowledge of which answer is a frame. Meta text is read within the
bootstrap budget and settled (`Settling`): answered by the translator, its
change announced to every open subscription (`MetaTranslating::announced`);
forwarded to the Nexus, the translator's own settings merged into its
answer (`MetaTranslating::meta_answered`, which takes the translator); or
forwarded as a subscription, which the translator's own changes are carried
into while the Nexus keeps it open (`MetaSubscriptions`). An emission from
the paired Nexus is answered. `Pairing` is the pair launcher's argument
list (`PairReading`), and `PairRunning::launch` prepares the place, starts
both processes, hands the Nexus its `nexus_serving::NexusBootstrap` with a
fresh `InstanceId` (`FreshInstance`) and the translator its own, each
naming the other, and stops the survivor. A translator implements
`Translator`, `PublicTranslating`, `MetaTranslating` and
`EmissionAnswering` for its own type.
