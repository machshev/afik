*** Settings ***
Suite Setup       Setup
Suite Teardown    Teardown
Test Teardown     Test Teardown
Resource          ${RENODEKEYWORDS}

*** Variables ***
${BOOT_SENTINEL_ADDRESS}    0x20000000
${BOOT_SENTINEL_VALUE}      0xD032B007
${TARGET_ELF}               %{AFIK_DP32_ELF}

*** Test Cases ***
Reset Reaches Rust Boot Sentinel
    Execute Command    mach create "dp32g030-minimal"
    Execute Command    machine LoadPlatformDescription @${CURDIR}/dp32g030.repl
    Execute Command    sysbus LoadELF @${TARGET_ELF}

    ${before}=    Execute Command    sysbus ReadDoubleWord ${BOOT_SENTINEL_ADDRESS}
    Should Be Equal As Integers    ${before}    0

    Start Emulation
    ${after}=    Execute Command    sysbus ReadDoubleWord ${BOOT_SENTINEL_ADDRESS}
    Should Be Equal As Integers    ${after}    ${BOOT_SENTINEL_VALUE}
