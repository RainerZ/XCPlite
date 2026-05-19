#pragma once

/*----------------------------------------------------------------------------
| File:
|   xcplib_no_a2l_cfg.h
|
| Description:
|   XCPlite configuration OVERRIDES for the no-A2L use case.
|   Applied AFTER the defaults in xcplib_cfg.h via:
|     cmake: target_compile_definitions(xcplite PRIVATE "XCPLIB_CFG_OVERRIDE=\"xcplib_no_a2l_cfg.h\"")
|
|   Use case: The A2L database is generated externally by the xcpclient tool
|   from the application ELF file.  No runtime A2L generation, persistence
|   or A2L upload features are needed, reducing code size and removing any
|   dependency on a filesystem.
|
|   Only settings that DIFFER from the POSIX defaults are listed here.
 ----------------------------------------------------------------------------*/

//-------------------------------------------------------------------------------
// Calibration segments
// No persistence — A2L is not generated on-target; no .BIN state file needed
#undef OPTION_ENABLE_PERSISTENCE

// Absolute addressing required for xcpclient to locate calibration variables by address
#define OPTION_CAL_SEGMENTS_ABS

//-------------------------------------------------------------------------------
// Queue — use 32-bit mutex-based queue (compatible with 32-bit platforms and Windows)
#undef OPTION_QUEUE_64_VAR_SIZE
#undef OPTION_QUEUE_64_FIX_SIZE
#define OPTION_QUEUE_32

//-------------------------------------------------------------------------------
// A2L / ELF — generated externally from ELF by xcpclient; disable on-target features
#undef OPTION_ENABLE_A2L_GENERATOR
#undef OPTION_ENABLE_A2L_UPLOAD
