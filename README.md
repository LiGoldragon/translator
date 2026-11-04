# translator

Fresh version 1 reusable translator library. Binary Signal framing and typed
rkyv transport are always available. Enable the `text` feature for Datom text
decoding and encoding; Nexus crates can depend on the binary API without a
Datom dependency.

All external endpoints and CLI vocabulary belong to component translators and
are supplied as typed configuration.
# Translator

The optional `unix` feature supplies policy-free Unix transport mechanics:
absolute progress deadlines, idle-before-first-byte framing, complete-submit
witnesses, kernel peer credentials and pidfd liveness, validated admission,
and classified accept retry. Component socket names, lanes, query routing and
domain configuration remain in each Nexus.
