#pragma once
#define __XCPLIB_APP_CFG_H__

/*----------------------------------------------------------------------------
| File:
|   xcplib_app_cfg.h
|
| Description:
|   Application specific xcplite configuration overrides for frida_demo
|   Applied AFTER the defaults in xcplib_cfg.h. Selected in CMakeLists.txt via
|     set(XCPLITE_CFG_OVERRIDE "${CMAKE_CURRENT_SOURCE_DIR}/config/xcplib_app_cfg.h")
|   before FetchContent_MakeAvailable(xcplite), which makes xcplite define
|   XCPLIB_CFG_OVERRIDE="xcplib_app_cfg.h" for the library and for this application.
|
|   Only #undef / #define OPTION_* here. All tunables are documented in docs/xcplib_cfg.md.
|
 ----------------------------------------------------------------------------*/

//-------------------------------------------------------------------------------
// - Logging

#undef OPTION_DEFAULT_DBG_LEVEL
#define OPTION_DEFAULT_DBG_LEVEL 4

//-------------------------------------------------------------------------------
// - Linktime calibration segment and event registration
// - No DAQ runtime event management
// - No on-target A2L generator
// - No persistence
// - Absolute addressing mode

#define OPTION_SECTION_REGISTRATION
#define OPTION_CAL_SEGMENTS_ABS
#undef OPTION_DAQ_EVENT_LIST

#undef OPTION_ENABLE_A2L_GENERATOR
#undef OPTION_ENABLE_PERSISTENCE

#undef OPTION_ENABLE_A2L_UPLOAD
#undef OPTION_ENABLE_ELF_UPLOAD
