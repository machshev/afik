# Local PY32 dependencies

These dependencies are kept in AFIK because the released `py32-metapac 0.5.0`
contains incomplete PY32F071 metadata and the released `py32-hal 0.4.1` does
not expose an F071 chip feature.

`py32-metapac` was generated with `./d gen` from `py32-rs/py32-data` commit
`eb33b9ab85aa4652006e3435d84e1f9f7e5eca50`. That source models PY32F071 as
its own series, derived from its maintained DIE072 inventory with CAN disabled.

`py32-hal` starts from the crates.io 0.4.1 artifact and contains only these
local compatibility changes:

- expose all four concrete F071 package features;
- map the old F002B feature to the regenerated PAC's generic feature name;
- accept generic chip names while declaring build-time chip cfg values;
- retain DAC in the PAC inventory without generating a nonexistent HAL driver
  binding; and
- leave the F071 ADC HAL module and pin bindings disabled until its constants
  are independently evidenced; and
- add only a bounded transmit-only SPI surface with generated SCK/MOSI pin
  traits and cooperative fixed-chunk async writes for the evidenced K1 display.

The AFIK firmware selects `py32f071r1b` only for a compile-time inventory
check because the available primary product-page evidence names that package.
This is not a claim that the exact fitted K1 package suffix has been observed.
No HAL initialization is linked into the physical image by this milestone.
