OUTPUT_ARCH(arm)
ENTRY(Reset)

/*
 * EVID-K1-020: the pinned PY32F071 source contract places the application at
 * 0x08002800, provides 118 KiB through 0x08020000, and declares 16 KiB SRAM.
 * These are an AFIK image boundary, not a claim about unobserved bootloader
 * remapping or board peripherals.
 */
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x08002800, LENGTH = 0x1d800
  RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 16K
}

SECTIONS
{
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

  /* Development-only observation word; this is not a hardware register. */
  .boot_sentinel ORIGIN(RAM) (NOLOAD) : ALIGN(4)
  {
    __boot_sentinel_start = .;
    KEEP(*(.boot_sentinel));
    __boot_sentinel_end = .;
  } > RAM

  /* This bounded image has no runtime data-copy or BSS-zeroing startup. */
  .data : ALIGN(4)
  {
    *(.data .data.*);
  } > RAM AT > FLASH

  .bss (NOLOAD) : ALIGN(4)
  {
    *(.bss .bss.* COMMON);
  } > RAM

  __ram_image_end = .;
  __application_origin = ORIGIN(FLASH);
  __application_end = ORIGIN(FLASH) + LENGTH(FLASH);
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
         "vector table must start at K1 application origin")
  ASSERT(SIZEOF(.vector_table) == 8,
         "minimum K1 vector table must contain exactly SP and Reset")
  ASSERT(__flash_image_end <= ORIGIN(FLASH) + LENGTH(FLASH),
         "K1 target image exceeds evidenced application flash")
  ASSERT(__boot_sentinel_start == ORIGIN(RAM),
         "K1 boot sentinel must remain at the development observation address")
  ASSERT(__boot_sentinel_end - __boot_sentinel_start == 4,
         "K1 boot sentinel must be exactly one word")
  ASSERT(SIZEOF(.data) == 0,
         "minimum K1 Reset handler does not initialise .data")
  ASSERT(SIZEOF(.bss) == 0,
         "minimum K1 Reset handler does not initialise .bss")
  ASSERT(__ram_image_end <= __stack_top,
         "K1 target image exceeds evidenced SRAM")
}
