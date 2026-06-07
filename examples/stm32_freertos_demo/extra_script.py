from pathlib import Path

Import("env")

project_dir = Path(env.subst("$PROJECT_DIR"))
pioenv = env.subst("$PIOENV")
xcplite_inc = project_dir.parent.parent / "inc"
xcplite_src = project_dir.parent.parent / "src"
project_inc = project_dir / "include"
stm32_freertos_src = project_dir / ".pio" / "libdeps" / pioenv / "STM32duino FreeRTOS" / "src"
stm32_freertos_kernel_inc = stm32_freertos_src / "FreeRTOS" / "Source" / "include"
stm32_freertos_cmsis_v2 = project_dir / ".pio" / "libdeps" / pioenv / "STM32duino FreeRTOS" / "portable" / "CMSIS_RTOS_V2"
stm32_lwip_src = project_dir / ".pio" / "libdeps" / pioenv / "STM32duino LwIP" / "src"

include_paths = []
for path in (
    stm32_freertos_kernel_inc,
    stm32_freertos_src,
    stm32_freertos_cmsis_v2,
    stm32_lwip_src,
    project_inc,
    xcplite_inc,
    xcplite_src,
):
    if path.exists():
        include_paths.append(path)

env.Append(CPPPATH=[str(path) for path in include_paths])

xcplite_sources = [
    "cal.c",
    "platform.c",
    "queue32.c",
    "xcpappl.c",
    "xcpethserver.c",
    "xcpethtl.c",
    "xcplite.c",
]

env.BuildSources(
    "$BUILD_DIR/xcplite",
    str(xcplite_src),
    src_filter=["+<{}>".format(source) for source in xcplite_sources],
)
