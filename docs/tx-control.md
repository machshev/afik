# Shared PTT lifecycle

`TXCTRL-054` adds `radio-platform::tx_control`, a target-neutral, no_std,
heap-free controller. `radio-sim::tx_control::TxSimulator` composes that same
controller with the existing logical BK4819 model. Neither firmware's keypad,
serial controls, RF adapter, or main loop invokes this controller.

## Startup and entry

Construction requires a non-zero, never-reused application instance, the sampled
PTT level, and explicit integer milliseconds. Startup always requests PA off,
unconditional TX off, then receive restoration, in that order. It does not
assume that reboot established safe outputs. Held-at-boot PTT, or a press during
startup, requires release before entry. A denied press also requires release;
changing permissions or evidence while held cannot retry it automatically.

A fresh press in `Ready` goes through `TxPolicy::authorise` with the complete
current `TxRequest`. The controller retains an exact bounded snapshot and arms
an absolute deadline measured from that press, including all preparation time.
Entry requests PA off, non-energizing preparation, chip/modulation TX enable,
then PA enable. Each operation must be acknowledged before the next is issued.
Preparation carries exact approved settings and power; it represents no
register encoding or calibrated hardware power result.

An adapter obtains a borrowed `Effect`, executes it once synchronously, drops
it, and reports `Acknowledgement::applied` or `failed` for its opaque `EffectId`.
The effect borrows the controller, policy, and current request. Preparation and
both enable operations carry fresh complete-policy authority. Copying an action
name or acknowledgement identity does not provide TX authority. Target TX and
PA interfaces must accept and recheck that borrowed token at execution.

Every input checks current facts against the accepted request while entering or
transmitting, and repeats central authorization. Acquiring an entry effect also
checks the entire fresh request and policy. Changed identity, channel, power,
evidence, operating facts, or denial starts shutdown. An effect-acquisition
rejection requires delivering invalidation before proceeding. Rust prevents an
effect from surviving controller mutation; a compile-fail test verifies this.

## Time, identities, and cancellation

The core owns no clock or scheduler. Callers deliver monotonic `u64`
milliseconds with every input and before acquiring an effect. They must deliver
a tick at the exposed deadline, and bound synchronous adapter latency. At or
beyond the deadline, even an entry acknowledgement starts shutdown before
another enable operation can be obtained. Duplicate presses never extend it.
Backwards time faults the controller and starts cleanup. Absolute-deadline
overflow denies entry.

Effect identities contain the application instance, a checked cycle counter,
and a private phase. Duplicate, old-phase, old-cycle, and other-instance
acknowledgements cannot advance a current operation. Cycle exhaustion denies
new entry without wrapping; an already accepted final cycle can still shut
down. Adapters must invalidate queued work when replacing the controller and
must not reuse its instance. There is no detached or asynchronous effect queue.

Release, timeout, explicit configuration invalidation, changed approved facts,
and target/adapter faults cancel entry or transmission. Timeout/invalidation
cannot restart while PTT remains held. A release followed by a press during
cleanup also requires another release before entry, rather than starting TX
when receive restoration eventually completes.

## Shutdown and failure

Shutdown requests PA off, then unconditional chip/modulation TX off, then the
latest authoritative receive setup. It does not call the driver's known-TX-only
stop method: the logical adapter uses neutral-mode recovery, including from
unknown/faulted states. A target receive adapter owns its full receive setup,
tones, thresholds, audio preference, and health checks; the simulator models
only current selection/frequency and known selected state.

Any entry failure, including uncertain partial completion, starts that same
shutdown. PA-off failure still requests TX off. Receive restoration is issued
only after both disables succeed. If either disable fails, the controller
latches `Faulted` without restoring receive or enabling TX again. A failed
receive restoration requests a second PA-off/TX-off pair with distinct phase
identities, then faults without retrying receive. A fault remains latched even
if cleanup successfully restores receive. A replacement controller must use a
fresh instance and complete safe startup; no automatic recovery is provided.

Applied acknowledgements are trusted adapter reports, not measurements. Failed
shutdown cannot prove the PA or chip is physically off. A blocked executor,
lost input, unbounded adapter operation, fabricated current snapshot, or reused
instance is outside this pure software guarantee (`RISK-038`, `RISK-039`).

## Simulator evidence

Virtual time advances only explicitly. `step` delivers a tick, executes exactly
one effect, and reports its exact completion. PA state and prepared power are
logical values, not GPIO or RF. The adapter performs the existing token-gated
TX command plan, and shutdown recovers neutral mode before restoring receive.

Tests cover identical timed and fault scripts, boot/press/release interlocks,
complete-request denial with no added RF operations, fresh authority at each
entry boundary, stale acknowledgements, exact deadline cancellation, changed
selection restoration, and faults before or after every ordinary effect.
Register failures cover each TX-entry and receive-restore write; tests also
exercise faults in the second cleanup pair. A rejected PA-off operation leaves
the logical PA enabled while chip shutdown is attempted, explicitly avoiding a
fabricated successful shutdown observation.

No hardware/register/calibration fact is added by these tests. `RISK-007` remains
the separate physical TX evidence gate.
