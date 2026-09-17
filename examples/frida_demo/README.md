# frida_demo - XCP measurement of functions hooked with Frida Gum

This example combines XCPlite with [Frida Gum](https://frida.re), the in-process instrumentation engine of the Frida toolkit. A function `foo()` is called cyclically by the main loop. It contains **no XCPlite instrumentation at all**. Frida's Interceptor hooks it, and the hook callbacks trigger XCP DAQ events with everything Frida hands over: the arguments, the return value, the call duration, thread id, call depth and the complete CPU register snapshot of the invocation.

A second hook on the libc allocation functions `malloc()`, `calloc()` and `aligned_alloc()` does the same for functions we do not even own: every allocation in the process, from any thread, triggers an XCP event with the requested size. The callers are recorded, which makes visible how few allocations the XCPlite library itself does: one during initialization, none afterwards.

## What it demonstrates

| Feature | How it is demonstrated |
|---|---|
| Measuring a function without instrumenting its source | `foo()` is hooked with `gum_interceptor_attach()`, the `on_enter`/`on_leave` callbacks trigger the XCP events `foo_enter` and `foo_leave` |
| XCP event trigger inside a hook | The callbacks run synchronously on the calling thread, `DaqTriggerEventExt()` is called right there. No ring buffer, no helper thread |
| Relative addressing with a per-invocation base pointer | The Frida register snapshot `GumCpuContext` lives in the trampoline's stack frame at a different address on every call. The hooks pass it as base pointer to `DaqTriggerEventExt()`, the A2L describes it as a typedef instance relative to that base pointer |
| Stack frame relative measurement from any thread | The allocation hook hands its values to a `XCP_NOINLINE` helper function, whose local variables are measured stack frame relative. Every thread has its own stack, so this is thread safe without per-thread state |
| Hooking foreign code by name | `malloc()`, `calloc()` and `aligned_alloc()` are located with `gum_module_find_global_export_by_name()` |
| Attributing allocations to XCPlite | Thread and phase classification (`origin`), a lock-free caller histogram and symbol resolution in the main loop |
| Calibration | `params.foo_iterations` changes the execution time of `foo()`, `params.malloc_size` the size seen by the allocation hook |

## Frida Gum in five minutes

Frida is known for its Python/JavaScript side (`frida-server`, `frida-trace`) which injects a JavaScript engine into a running process. Underneath is **Gum**, a C library (GLib/GObject based) which does the actual work: inline hooking, code tracing, memory, module and thread introspection. Gum can be used *embedded*: linked statically into your own program, no injection, no `frida-server`, no ptrace, no special privileges. That is the mode this example uses.

**Devkits.** Every Frida release publishes prebuilt static devkits on the [GitHub release page](https://github.com/frida/frida/releases): `frida-gum-devkit-<version>-<os>-<arch>.tar.xz` (about 10 MB) with exactly three files: `frida-gum.h`, `libfrida-gum.a` (GLib, capstone etc. bundled) and `frida-gum-example.c`. The `CMakeLists.txt` of this example downloads the devkit for the host platform with `FetchContent`. Devkits exist for macOS (arm64, x86_64), Linux (x86_64, arm64, armhf, ...), Windows, Android and iOS. License: wxWindows Library Licence (LGPL with a static linking exception).

**The Interceptor.** `gum_interceptor_attach()` overwrites the first instructions of the target function with a jump into a generated trampoline. The displaced instructions are relocated. The trampoline saves all registers (that is the `GumCpuContext`), calls the `on_enter` callback of every attached listener, restores the registers, runs the original function, and replaces the return address so that `on_leave` runs when the function returns:

```c
gum_init_embedded();
GumInterceptor *interceptor = gum_interceptor_obtain();
GumInvocationListener *listener = gum_make_call_listener(on_enter, on_leave, user_data, NULL);

GumAttachOptions options = {0};
options.listener_function_data = GSIZE_TO_POINTER(HOOK_FOO); // Tag, read with gum_invocation_context_get_listener_function_data()
gum_interceptor_begin_transaction(interceptor);
gum_interceptor_attach(interceptor, (gpointer)foo, listener, &options);
gum_interceptor_end_transaction(interceptor); // The code patches are applied here
```

The callbacks receive a `GumInvocationContext`:

```c
gpointer gum_invocation_context_get_nth_argument(ctx, n);     // on_enter
gpointer gum_invocation_context_get_return_value(ctx);        // on_leave
gpointer gum_invocation_context_get_return_address(ctx);
guint    gum_invocation_context_get_thread_id(ctx);
guint    gum_invocation_context_get_depth(ctx);               // Nesting depth of hooked calls
gpointer gum_invocation_context_get_listener_invocation_data(ctx, size); // Per call scratch memory, survives from on_enter to on_leave
ctx->cpu_context                                              // GumCpuContext *, the register snapshot
```

Things to know when writing hooks:

- The hooked function must not be inlined (`XCP_NOINLINE`), and it must be larger than the few instructions the trampoline jump replaces. Otherwise `gum_interceptor_attach()` fails with `GUM_ATTACH_WRONG_SIGNATURE`.
- The callbacks run **synchronously on the thread which called the hooked function**. A hook on `malloc()` runs on every thread of the process.
- Frida has **no re-entrancy guard for listeners**: a hooked function called from inside a callback is intercepted again. An allocation hook must therefore not call anything which may allocate (`printf`, A2L registration, symbol lookup), it would recurse into itself.
- The XCP event trigger functions are lock-free and never allocate, they are safe to call from any hook.

Gum has more engines which are not used here: the **Stalker** traces execution by dynamically recompiling code basic block by basic block, and delivers call, return and block events or lets a transformer insert callouts with live register state.

## Building

```bash
cd examples/frida_demo
./build.sh local          # against the xcplite working tree this example is part of
./build.sh                # against the xcplite release tag pinned in CMakeLists.txt
./build.sh clean local    # clean rebuild
```

The first configure downloads the Frida Gum devkit into `build/_deps/frida_gum-src/`. To use an already extracted devkit (offline build, another version), pass `-DFRIDA_DEVKIT_DIR=<dir>` to CMake. The version is selected with `-DFRIDA_VERSION=<version>`.

Supported platforms: macOS arm64 and x86_64, Linux x86_64, arm64 and armhf (gcc or clang). The register context typedef in `main.c` is written for arm64 and x86_64.

## Running

```bash
./build/frida_demo
```

```text
Frida: hook foo() at 0x104d94000: ok (0)
Frida: hook malloc() at 0x18a75dce8: ok (0)
Frida: hook calloc() at 0x18a75f88c: ok (0)
Frida: hook aligned_alloc() at 0x18a797cac: ok (0)
...
XCP initialization done: 1 allocations
Start main loop... (Ctrl+C to stop)
foo: 513 calls, last duration 29583 ns, arg_a=512 ret=-1491677212
malloc: app=513 xcp_init=1 xcp_threads=0, 196928 bytes total
malloc callers:
       513 calls     131328 bytes  main+2472 (frida_demo)
         1 calls      65600 bytes  queueInitFromMemory+212 (frida_demo)
```

The statistics are printed every 2 seconds from the main loop. The one allocation of XCPlite is the transmit queue in `XcpEthServerInit()`, nothing is allocated afterwards, also not on connect, A2L finalization or DAQ start (the XCP server threads show `xcp_threads=0`).

## Measurement with xcpclient

The `xcpclient` tool (`tools/xcpclient`) is an XCP client for testing, no CANape needed. Run it from `examples/frida_demo` while the demo is running:

```bash
xcpclient --udp --upload-a2l --list-mea .                                                   # Upload the A2L file, list everything
xcpclient --udp --a2l frida_demo.a2l --mea "foo_ctx.*" --time 3 --verbose 2                 # One sample per call of foo()
xcpclient --udp --a2l frida_demo.a2l --mea "foo_leave_cpu_context.*" --time 2 --verbose 2   # Register context at the return of foo()
xcpclient --udp --a2l frida_demo.a2l --mea "kind|size|origin|thread_id" --time 2 --csv malloc.csv  # Every allocation, all threads
xcpclient --udp --a2l frida_demo.a2l --cal params.foo_iterations 100000                     # foo_ctx.duration_ns rises
xcpclient --udp --a2l frida_demo.a2l --cal params.malloc_size 4096                          # size in the malloc_leave event changes
xcpclient --udp --a2l frida_demo.a2l --mea "counter|malloc_count.*" --time 3
```

`foo_leave_cpu_context.x[0]` (arm64) or `foo_leave_cpu_context.rax` (x86_64) is the return value of `foo()`, the same as `foo_ctx.ret`.

## Events and variables

| Event | Triggered from | Addressing | Variables |
|---|---|---|---|
| `foo_enter` | `on_enter` of `foo()` | absolute, relative | `foo_ctx.arg_a`, `foo_ctx.arg_iterations`, `foo_enter_cpu_context.*` |
| `foo_leave` | `on_leave` of `foo()` | absolute, relative | `foo_ctx.ret`, `foo_ctx.duration_ns`, `foo_ctx.call_count`, `foo_ctx.thread_id`, `foo_ctx.depth`, `foo_ctx.return_address`, `foo_ctx.function`, `foo_leave_cpu_context.*` |
| `malloc_leave` | `on_leave` of `malloc()`, `calloc()`, `aligned_alloc()`, any thread | stack | `kind`, `size`, `ptr`, `return_address`, `thread_id`, `depth`, `origin` |
| `mainloop` | main loop | absolute, stack | `counter`, `malloc_count_app`, `malloc_count_xcp_init`, `malloc_count_xcp_threads`, `malloc_bytes_total` |

`origin` classifies an allocation by thread and phase: `APP` (main thread after initialization), `XCP_INIT` (main thread during XCP initialization and A2L registration), `XCP_THREAD` (the XCP server threads, the only other threads in this process).

## Design notes

**Event descriptors at file scope.** `DaqCreateEvent()` expands to declarations plus a statement, so it can only be used inside a function. The Frida callbacks need the descriptors in scope, therefore `main.c` declares them directly at file scope with `XCP_EVENT_SECTION_ATTR`, which is what the macro does. `XcpInit()` finds them in the `xcp_evts` linker section.

**Hooks before `XcpInit()`.** The allocation hooks are attached first, so the allocations of the XCP initialization are counted. The event triggers are passive until XCP is started.

**No A2L registration inside a hook.** All `A2l*` calls happen in the init phase from `main()`. The stack frame relative variables of the allocation hook helper `malloc_leave()` need `A2lSetStackAddrMode()` called from inside that function, so `main()` makes one priming call `malloc_leave(..., prime = true)` after `A2lInit()`. The registration allocates and re-enters the hook, which is harmless because the nested calls do not touch the A2L generator. The register context typedef is registered with a dummy `GumCpuContext` instance for the offsets, the base pointer at runtime is the live `ctx->cpu_context`.

**Symbol resolution.** The caller histogram stores return addresses only. The names are resolved in the main loop with `gum_symbol_details_from_address()` and `dladdr()` as fallback, inside `gum_interceptor_ignore_current_thread()` so the resolver's own allocations are not counted. On Linux the executable is linked with `-rdynamic` (`ENABLE_EXPORTS`) so `dladdr()` sees its functions. The return address is the immediate caller, an allocation made by a libc function on behalf of XCPlite (`fopen`, `pthread_create`) shows up under the libc function's name.

**Platform note on macOS.** The libc internals (`fopen`, `strdup`, `pthread_create`, ...) call the zone allocator directly inside the dyld shared cache and never pass the exported `malloc()`. Only calls from code outside the shared cache are seen: the application and the statically linked xcplite. On Linux (glibc) every allocation is seen.

## Ideas for further experiments

- **Calibrate code you cannot edit**: `gum_invocation_context_replace_return_value()` or `replace_nth_argument()` in `on_leave`/`on_enter`, driven by a calibration parameter.
- **Stalker**: follow the main thread while inside `foo()` and count basic blocks and calls per invocation.
- **Hook by name**: a command line argument `--hook <symbol>` with `gum_find_function()` to attach to any function of the executable.
- **Offline A2L from the ELF file**: the DWARF debug information describes the parameters of every function as register locations at entry (`x0..x7` on arm64, `rdi, rsi, ...` on x86_64), which are fixed offsets inside `GumCpuContext`. The `xcpclient` A2L generator could describe the typed arguments of a hooked function relative to the register context base pointer without any `A2l*` call in the application. The dynamic address slot is not generated by `xcpclient` yet, see `docs/OFFLINE_A2L.md`.

## See also

- [examples/fetchcontent_example](../fetchcontent_example/README.md) - the `FetchContent` consumer pattern this example is based on
- [docs/TECHNICAL.md](../../docs/TECHNICAL.md) - addressing modes and the `DaqTriggerEventExt()` base pointer
- [Frida Gum documentation](https://frida.re/docs/gum/) and [frida-gum on GitHub](https://github.com/frida/frida-gum)
