MEMORY
{
    /* AFIK K1 application. The final 8 KiB erase sector, 0x0801E000 to
       0x08020000, is reserved for the retained configuration image and is
       deliberately outside the application region. */
    FLASH : ORIGIN = 0x08002800, LENGTH =  110K
    RAM   : ORIGIN = 0x20000000, LENGTH =   16K /* SRAM */
}
