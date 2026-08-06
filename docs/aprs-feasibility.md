# APRS receive feasibility

This is the physical receive-chain verdict for `APRS-011`. Protocol parsing and
repeater discovery are feasible at a complete-frame boundary. Physical APRS
reception on a UV-K5-family target is **deferred for missing modem, board-path,
and target-peripheral evidence**.

## Layer verdicts

| Layer | Verdict | Evidence boundary |
| --- | --- | --- |
| RF tuning and FM receive | Deferred physically | The logical BK4819 model is not a target adapter; silicon revision, initialization, RF path, crystal, filters, and physical performance remain unknown. |
| FM/baseband access | Deferred | Beken documents internal demodulation but no accepted evidence establishes a suitable board signal routed to the MCU. |
| On-chip FSK modem | Deferred | A modem exists, but the low-confidence V3 modes and fixed framing do not establish Bell-202-style APRS compatibility or transparent AX.25 bits. |
| MCU audio demodulation | Deferred | DP32G030 ADC/timer behavior, board routing, sampling budget, buffering, and interrupt/DMA behavior are not accepted target facts. |
| NRZI, flags and bit unstuffing | Designable, not in this package | AX.25 defines the bit rules, but a physical/sample input, recovery policy, bounds, and receive corpus are missing. |
| Complete de-stuffed frame and FCS | Implement | This is hardware-independent, bounded by the APRS reference, and independently testable. |
| AX.25 UI/APRS Object/Item parsing | Implement | Primary specifications give exact address, UI/PID, information, lifecycle, position, and field syntax. |
| Repeater discovery table | Implement as untrusted receive-only data | Local key/freshness/capacity rules can be explicit and deterministic without tuning, storage mutation, or TX authority. |

Passing parser tests will prove only behavior on supplied bytes. It will not
increase confidence in the blocked physical layers.

## Why the BK4819 modem is not enough evidence

Beken publicly advertises FSK modulation/demodulation. The only accessible
register-level description is machine-translated, user-uploaded, and not matched
to the fitted silicon. It names FFSK 1200/1800 and 1200/2400 receive modes and a
configured preamble, sync word, length, CRC, FIFO, and interrupt flow.

Typical terrestrial 1200-baud APRS uses 1200/2200 Hz AFSK, NRZI, HDLC flags,
zero-bit stuffing, a variable AX.25 UI frame, and the AX.25 FCS. The available
description neither names the 1200/2200 pair nor documents a transparent raw-bit
mode that would let software perform HDLC recovery. Similar numbers or an FSK
label are not proof of compatibility.

The alternative—sampling discriminator/audio and demodulating in the MCU—also
remains only a hypothesis. A Cortex-M0 and an ADC name do not establish the
physical pin, analog bandwidth/level, attainable sample clock, DMA/interrupt
service, memory budget, error rate, or power cost on this board.

## Required receive-only experiments

No experiment may transmit from the DUT. Establish calibration backup and
recovery before changing any chip state.

1. Photograph and identify the board and fitted BK4819/DP32G030 markings. Trace
   the RF IC audio/baseband, speaker path, MCU pins, test points, filters, bias,
   and level limits from schematics plus continuity measurements.
2. Obtain original revision-matched BK4819 documentation. With a calibrated,
   shielded/coupled generator carrying known APRS recordings, test modem modes
   one field at a time. Establish tone response, baud/clock behavior, raw versus
   framed output, preamble/sync interaction, length, FIFO ordering, interrupts,
   overflow, CRC disable, cancellation, and cleanup.
3. Capture any accessible discriminator/audio path with an oscilloscope/audio
   analyzer across known tone levels, deviations, receive bandwidths, squelch,
   weak signals, adjacent carriers, collision, clipping, and silence. Verify
   DC bias and absolute MCU pin limits before connection.
4. Only if a safe MCU input path exists, establish ADC clock/rate, timer trigger,
   interrupt or DMA path, bounded buffering, worst-case CPU/RAM/flash use, power,
   and coexistence with UI/radio service. These require sourced DP32G030
   peripheral facts before target code or Renode modeling.
5. Build a provenance-tagged corpus containing generated and recorded clean,
   weak, noisy, frequency-offset, over/under-deviated, collided, truncated,
   overlong, bad-FCS, stuffed-bit, abort, and back-to-back frames. Keep raw
   samples, expected recovered bits/bytes, equipment settings, and decoder
   version together.
6. Measure acquisition, bit/frame error rate, false frames, FCS rejection,
   overflow/recovery, cancellation, stale interrupts, and cleanup over the
   corpus and live receive-only generator setup. Repeat after reset and across
   supply/temperature cases that are safe and relevant.
7. Prove through bus traces and independent RF monitoring that every experiment
   remains in receive, that faults cannot write the inferred TX mode, and that
   failed cleanup latches the radio unavailable until explicit recovery.

## Future lower-layer test gates

A later work package may add lower layers only after the experiments above. Its
deterministic tests must include exact tone/symbol vectors; fractional clock and
frequency error; both NRZI polarities; flag acquisition; zero insertion/removal;
abort and non-octet frames; maximum-length and back-to-back frames; buffer
overflow; FCS residue; stale DMA/interrupt events; cancellation at every phase;
and identical sample/time scripts producing identical frames and faults.

## Safety boundary

Received APRS data is unauthenticated. A valid FCS detects accidental corruption
only; it does not authenticate the origin or make a repeater advertisement true.
Discovery results therefore remain reviewable receive-only observations. They
cannot construct `ActiveChannel`, select a trusted `TxClass`, mint
`TxAuthorisation`, tune automatically, or mutate configuration. Any future save
is a separate confirmed transaction constrained to `TxClass::Never`.
