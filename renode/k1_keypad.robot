*** Settings ***
Suite Setup       Setup
Suite Teardown    Teardown
Test Teardown     Test Teardown
Resource          ${RENODEKEYWORDS}

*** Variables ***
${TARGET_ELF}       %{AFIK_K1_ELF}
${RESET_ADDRESS}    %{AFIK_K1_RESET_ADDRESS}
${RENDER_ADDRESS}   %{AFIK_K1_RENDER_ADDRESS}
${KEYPAD_INIT_ADDRESS}    %{AFIK_K1_KEYPAD_INIT_ADDRESS}
${MENU_CONTROL}     0x50000500

*** Test Cases ***
Synthetic Menu Reaches Production Render Path
    Execute Command    mach create "k1-keypad-execution"
    Execute Command    machine LoadPlatformDescription @${CURDIR}/k1-keypad.repl
    Execute Command    sysbus LoadELF @${TARGET_ELF}
    Execute Command    cpu SP 0x20004000
    Execute Command    cpu PC ${RESET_ADDRESS}
    Execute Command    cpu AddHook ${KEYPAD_INIT_ADDRESS} "self.InfoLog('K1_INITIAL_DISPLAY_RETURNED')"
    Execute Command    cpu AddHook ${RENDER_ADDRESS} "self.InfoLog('K1_KEY_RENDER_REACHED'); self.IsHalted = True"

    Create Log Tester    5
    Start Emulation
    Wait For Log Entry    K1_INITIAL_DISPLAY_RETURNED    timeout=5
    Execute Command    sysbus WriteDoubleWord ${MENU_CONTROL} 1
    Wait For Log Entry    K1_KEY_RENDER_REACHED    timeout=5
