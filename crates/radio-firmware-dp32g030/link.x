OUTPUT_ARCH(arm)
ENTRY(Reset)

/* EVID-DP32-002: DP32G030 reference manual v1.23, sections 5.1-5.2. */
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 64K
  RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 16K
}

SECTIONS
{
  /* EVID-ARM-003: Cortex-M0 vectors begin at address zero. */
  .vector_table ORIGIN(FLASH) : ALIGN(4)
  {
    KEEP(*(.vector_table));
  } > FLASH

  .text : ALIGN(4)
  {
    *(.text .text.*);
    *(.rodata .rodata.*);
  } > FLASH

  .ARM.exidx : ALIGN(4)
  {
    *(.ARM.exidx .ARM.exidx.*);
  } > FLASH
  __flash_image_end = .;

  /* Simulation-only observation word; this is not a hardware register. */
  .boot_sentinel ORIGIN(RAM) (NOLOAD) : ALIGN(4)
  {
    __boot_sentinel_start = .;
    KEEP(*(.boot_sentinel));
    __boot_sentinel_end = .;
  } > RAM

  /* This minimum image has no runtime data-copy or BSS-zeroing startup. */
  .data : ALIGN(4)
  {
    *(.data .data.*);
  } > RAM AT > FLASH

  .bss (NOLOAD) : ALIGN(4)
  {
    *(.bss .bss.* COMMON);
  } > RAM

  __ram_image_end = .;
  __stack_top = ORIGIN(RAM) + LENGTH(RAM);

  .ARM.attributes 0 :
  {
    *(.ARM.attributes);
  }

  /DISCARD/ :
  {
    *(.comment);
  }

  ASSERT(ADDR(.vector_table) == ORIGIN(FLASH),
         "vector table must start at flash origin")
  ASSERT(SIZEOF(.vector_table) == 8,
         "minimum vector table must contain exactly SP and Reset")
  ASSERT(__flash_image_end <= ORIGIN(FLASH) + LENGTH(FLASH),
         "target image exceeds evidenced flash")
  ASSERT(__boot_sentinel_start == ORIGIN(RAM),
         "boot sentinel must remain at the simulation observation address")
  ASSERT(__boot_sentinel_end - __boot_sentinel_start == 4,
         "boot sentinel must be exactly one word")
  ASSERT(SIZEOF(.data) == 0,
         "minimum Reset handler does not initialise .data")
  ASSERT(SIZEOF(.bss) == 0,
         "minimum Reset handler does not initialise .bss")
  ASSERT(__ram_image_end <= __stack_top,
         "target image exceeds evidenced RAM")
}
