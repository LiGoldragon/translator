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
