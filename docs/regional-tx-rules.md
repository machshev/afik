# Regional transmit rules

`TXRULE-049` is an offline, hardware-independent rule snapshot. It does not
enable a PA, alter the K1 image, or claim that any transmission is lawful.
`BENCH-046` remains deferred until the exact K1 and serial bench are available.

The rule engine is `no_std`, heap-free, and fixed-capacity. It represents all
six permissioned `TxClass` values plus `Never`, but a class permission is only
one input. Location, current licence or authorization, operator status, and
equipment eligibility are facts the radio cannot discover and therefore
remain affirmative attestations. Marine, aeronautical, business, and
experimental use additionally requires a complete individualized grant with
an exact region, class, frequency range, FM bandwidth, and conducted-power
ceiling. Missing or mismatched data denies transmission.

The built-in version-1 snapshot was checked on 2026-08-12 against:

- Ofcom, “Information for amateur radio licensees”, including OFW611 and
  IR 2028: <https://www.ofcom.org.uk/spectrum/radio-equipment/amateur-radio-info>
- Ofcom, “Business Radio Guidance 2025”, including its PMR446 summary:
  <https://www.ofcom.org.uk/siteassets/resources/documents/spectrum/business-radio-licences/business-radio-guidance-document.pdf>
- Ofcom licence information for individualized spectrum products:
  <https://www.ofcom.org.uk/licences>
- 47 CFR 97.301, authorized amateur frequency bands:
  <https://www.ecfr.gov/current/title-47/chapter-I/subchapter-D/part-97/subpart-D/section-97.301>
- 47 CFR 97.305, authorized emission types:
  <https://www.ecfr.gov/current/title-47/chapter-I/subchapter-D/part-97/subpart-D/section-97.305>
- 47 CFR 97.313, transmitter power standards:
  <https://www.ecfr.gov/current/title-47/chapter-I/subchapter-D/part-97/subpart-D/section-97.313>

The UK licence-free entry represents analogue PMR446 at 446.0–446.2 MHz,
narrow FM, and no more than 500 mW. Ofcom's current guidance also identifies
mobile/hand-portable operation, simplex use, receiver capability, no airborne
use, non-interference/non-protection, the channel raster, transmitter timeout,
and the applicable equipment standards. The planned K1 60-second timeout is
below the stated 180-second maximum.

For this project the operator may enable `LicenceFreePlan` on the explicit
assumption that the equipment-side conditions, including the antenna
arrangement, are satisfied. That permission is an operator assertion, not
firmware evidence, equipment approval, or legal advice. Frequency, raster,
mode, bandwidth, power, simplex/mobile state, and timeout remain independent
checks in the complete authorization path. Until those remaining request
fields are integrated, this rule snapshot alone cannot mint a complete TX
authorization.

The initial generic power ceilings are policy ceilings, deliberately no higher
than 25 W for amateur operation. They do not model every licence tier,
band-specific reduction, antenna gain, ERP/EIRP conversion, or individualized
condition. A later complete request must take the minimum of the regional
rule, grant, hardware envelope, calibration, and operator-selected power.
