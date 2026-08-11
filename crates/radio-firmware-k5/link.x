OUTPUT_ARCH(arm)
ENTRY(Reset)

/*
 * EVID-DP32-002: DP32G030 has 64 KiB program flash and 16 KiB data RAM.
 * EVID-K5-009: qualified UV-K5 V1 deployment reserves the final 4 KiB for the
 * stock bootloader, leaving an application region ending at 0x0000F000.
 */
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x00000000, LENGTH = 60K
  RAM   (rwx) : ORIGIN = 0x20000000, LENGTH = 16K
}

/*
 * EVID-K5-019: the firmware running on these units starts its stack sixteen
 * bytes below the top of RAM. Nothing records why; AFIK pays the sixteen bytes
 * rather than assume the top of RAM is free.
 */
__stack_top = ORIGIN(RAM) + LENGTH(RAM) - 16;

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

  .data : ALIGN(4)
  {
    __data_start = .;
    *(.data .data.*);
    . = ALIGN(4);
    __data_end = .;
  } > RAM AT > FLASH
  __data_load_start = LOADADDR(.data);
  __flash_image_end = LOADADDR(.data) + SIZEOF(.data);

  .bss (NOLOAD) : ALIGN(4)
  {
    __bss_start = .;
    *(.bss .bss.* COMMON);
    . = ALIGN(4);
    __bss_end = .;
  } > RAM

  __ram_image_end = .;
  __application_end = ORIGIN(FLASH) + LENGTH(FLASH);

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
  ASSERT(SIZEOF(.vector_table) == 16,
         "K5 vector table must contain exactly SP, Reset, NMI and HardFault")
  ASSERT(__flash_image_end <= ORIGIN(FLASH) + LENGTH(FLASH),
         "target image exceeds the application region below the bootloader")
  ASSERT(__data_start % 4 == 0 && __data_end % 4 == 0,
         "initialised data must be word aligned for the startup copy")
  ASSERT(__bss_start % 4 == 0 && __bss_end % 4 == 0,
         "zeroed data must be word aligned for the startup clear")
  ASSERT(__data_load_start % 4 == 0,
         "initialised data must load from a word-aligned flash address")
  /*
   * The stack grows down from __stack_top towards __ram_image_end. A kilobyte
   * is not a measurement; it is the headroom this image is required to leave,
   * and the gate fails rather than let a future change quietly spend it.
   */
  ASSERT(__ram_image_end + 1024 <= __stack_top,
         "target image leaves less than 1 KiB of stack headroom")
}
