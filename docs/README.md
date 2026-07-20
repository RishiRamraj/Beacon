# Beacon documentation

| Document | What it covers |
|---|---|
| [design.md](design.md) | The full architecture: frame loop, plugin model, event arbitration, audio and speech, phasing, and accepted risks. |
| [decisions/](decisions/README.md) | Architecture decision records, one per substantive choice, with the evidence and the rejected alternatives. |

## How these fit together

`design.md` is the narrative: how Beacon works and why it is shaped this way.
It is kept current as the design evolves.

The ADRs are the record of individual decisions, including ones that are settled
and should not be relitigated without new evidence. When a decision changes, the
ADR gains a superseding entry rather than being edited into agreement with the
present; the point is that the reasoning survives, including the parts that
turned out to be wrong.

## Keeping them current

Any change that alters a decision in `design.md` should update the matching ADR
in the same commit. Any new substantive decision gets a new ADR. The design doc
may be edited freely; ADRs are append-mostly.
