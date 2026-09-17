// frida_demo - XCPlite measurement of functions hooked with Frida Gum
//
// The function foo() below is called cyclically by the main loop. It contains no XCPlite instrumentation at all.
// The Frida Gum Interceptor is used to hook foo(): Frida patches the first instructions of foo() with a jump into a
// trampoline, which calls our on_enter() callback before and our on_leave() callback after the original function code.
// Both callbacks run synchronously on the thread which called foo(), and both trigger an XCP DAQ event right there.
// The XCP client can then measure everything Frida hands us in the callback: the arguments, the return value, the call
// duration, thread id and call depth, and the complete CPU register snapshot Frida takes for the invocation.
//
// A second hook on the libc allocation functions malloc(), calloc() and aligned_alloc() shows the same for functions we do
// not even own: every allocation in the process, from any thread, triggers an XCP event with the requested size. The callers
// are recorded, so it is visible how few allocations the XCPlite library itself does (none after initialization).
//
// Platform note: on Linux (glibc) every allocation is seen, also the ones libc does internally (fopen, pthread_create, ...).
// On macOS the libc internals call the zone allocator directly inside the dyld shared cache and never pass the exported
// malloc(), only the calls from code outside the shared cache are seen: the application and the statically linked xcplite.
//
// Rules for code which runs inside a Frida callback:
// - The XCP event trigger functions are lock-free and never allocate, so they are safe to call from any hook.
// - Frida has no re-entrancy guard for listeners: a hooked function called from inside a callback is intercepted again.
//   The malloc hook therefore must not call anything which may allocate (printf, A2L registration, symbol lookup), it would
//   recurse into itself. All A2L registration is done in the init phase from main(), nothing in a hook calls A2l*.

#include <assert.h>
#include <dlfcn.h>
#include <signal.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

// Frida Gum umbrella header from the devkit
#include "frida-gum.h"

// XCPlite/libxcplite C headers
#include <a2l.h>    // for A2l generation
#include <xcplib.h> // for application programming interface

//-----------------------------------------------------------------------------------------------------
// XCP params

#define OPTION_PROJECT_NAME "frida_demo" // Project name, used to build the A2L file name
#define OPTION_PROJECT_VERSION "V1.0.0"  // EPK version string
#define OPTION_USE_TCP false             // TCP or UDP
#define OPTION_SERVER_PORT 5555          // Port
#define OPTION_SERVER_ADDR {0, 0, 0, 0}  // Bind addr, 0.0.0.0 = ANY
#define OPTION_QUEUE_SIZE (1024 * 64)    // Size of the measurement queue in bytes
#define OPTION_LOG_LEVEL 3               // Log level, 0 = no log, 1 = error, 2 = warning, 3 = info, 4 = print XCP commands
#define OPTION_XCP_MODE (XCP_MODE_LOCAL) // XCP single application server mode, no persistence
#define OPTION_A2L_MODE (A2L_MODE_WRITE_ALWAYS | A2L_MODE_FINALIZE_ON_CONNECT | A2L_MODE_AUTO_GROUPS)

//-----------------------------------------------------------------------------------------------------
// Calibration parameters

typedef struct params {
    uint32_t delay_us;       // Main loop period in us
    uint32_t foo_iterations; // Work done by foo(), changes its execution time
    uint32_t malloc_size;    // Bytes requested by the main loop on each cycle
    uint8_t foo_enabled;     // Call foo() from the main loop
} params_t;

// Default values (reference page, "FLASH")
const params_t params = {.delay_us = 10000, .foo_iterations = 1000, .malloc_size = 256, .foo_enabled = 1};

// Calibration segment handle
tXcpCalSegIndex params_calseg = XCP_UNDEFINED_CALSEG;

//-----------------------------------------------------------------------------------------------------
// XCP events
//
// The event descriptors are declared at file scope (this is what DaqCreateEvent() expands to), so the trigger macros
// can be used in the Frida callbacks below. XcpInit() finds the descriptors in the xcp_evts linker section.

static const tXcpEventDescriptor evt__foo_enter XCP_EVENT_SECTION_ATTR = XCP_EVENT_DESCRIPTOR_INIT("foo_enter", 0, 0);
static const tXcpEventDescriptor evt__foo_leave XCP_EVENT_SECTION_ATTR = XCP_EVENT_DESCRIPTOR_INIT("foo_leave", 0, 0);
static const tXcpEventDescriptor evt__malloc_leave XCP_EVENT_SECTION_ATTR = XCP_EVENT_DESCRIPTOR_INIT("malloc_leave", 0, 0);
static const tXcpEventDescriptor evt__mainloop XCP_EVENT_SECTION_ATTR = XCP_EVENT_DESCRIPTOR_INIT("mainloop", 0, 0);

//-----------------------------------------------------------------------------------------------------
// The function to be hooked
//
// No XCPlite instrumentation. XCP_NOINLINE is needed: Frida patches the code of the function, an inlined copy at the call
// site would not be hooked. The function must also be larger than the few instructions the trampoline jump replaces.

XCP_NOINLINE int32_t foo(int32_t a, uint32_t iterations) {
    uint32_t x = (uint32_t)a;
    for (uint32_t i = 0; i < iterations; i++) {
        x = x * 1103515245u + 12345u; // Linear congruential generator, just to burn some cycles
        x ^= x >> 13;
    }
    return (int32_t)x;
}

//-----------------------------------------------------------------------------------------------------
// Call context of foo(), filled by the hook callbacks
//
// foo() is called from the main thread only, so a single global instance is sufficient (absolute addressing mode).
// For a function called from several threads, the per-invocation data of the Frida invocation context, thread local
// storage or the stack frame relative addressing mode (see the malloc hook) would be used instead.

typedef struct {
    int32_t arg_a;           // Argument a
    uint32_t arg_iterations; // Argument iterations
    int32_t ret;             // Return value
    uint32_t call_count;     // Number of calls
    uint32_t duration_ns;    // Execution time from on_enter to on_leave in ns
    uint32_t thread_id;      // Thread id of the caller
    uint32_t depth;          // Nesting depth of hooked function calls
    uint64_t return_address; // Return address, the call site in the main loop
    uint64_t function;       // Address of foo()
} foo_ctx_t;

static foo_ctx_t foo_ctx;

//-----------------------------------------------------------------------------------------------------
// Allocation statistics, updated by the hook callbacks from any thread

// Where does an allocation come from
typedef enum {
    MALLOC_ORIGIN_APP = 0,        // Main thread in the run phase: the application
    MALLOC_ORIGIN_XCP_INIT = 1,   // Main thread in the init phase: XCP and A2L initialization, calibration segment, A2L registration
    MALLOC_ORIGIN_XCP_THREAD = 2, // Another thread: the XCP server threads (the only other threads in this process)
} malloc_origin_t;

// Plain types (not _Atomic) so the A2L type detection works, updated with the atomic builtins
static uint32_t malloc_count_app = 0;
static uint32_t malloc_count_xcp_init = 0;
static uint32_t malloc_count_xcp_threads = 0;
static uint64_t malloc_bytes_total = 0;

// Caller histogram: return address of the allocation function -> count, bytes. Fixed size open addressing, no allocation
#define MALLOC_CALLER_TABLE_SIZE 256
typedef struct {
    atomic_uintptr_t address;
    atomic_uint count;
    atomic_ullong bytes;
} malloc_caller_t;
static malloc_caller_t malloc_callers[MALLOC_CALLER_TABLE_SIZE];

static void malloc_caller_record(uintptr_t address, size_t size) {
    size_t i = (address >> 2) % MALLOC_CALLER_TABLE_SIZE;
    for (size_t n = 0; n < MALLOC_CALLER_TABLE_SIZE; n++) {
        uintptr_t expected = 0;
        if (atomic_load_explicit(&malloc_callers[i].address, memory_order_acquire) == address || atomic_compare_exchange_strong(&malloc_callers[i].address, &expected, address)) {
            atomic_fetch_add_explicit(&malloc_callers[i].count, 1, memory_order_relaxed);
            atomic_fetch_add_explicit(&malloc_callers[i].bytes, size, memory_order_relaxed);
            return;
        }
        i = (i + 1) % MALLOC_CALLER_TABLE_SIZE;
    }
    // Table full, drop
}

// Phase flag: allocations on the main thread are attributed to XCP init until main() sets this
static atomic_bool run_phase = false;
static GumThreadId main_thread_id;

//-----------------------------------------------------------------------------------------------------
// Allocation hook helper
//
// Called from the on_leave callback of malloc(), calloc() and aligned_alloc() on any thread. The values are held in local variables and measured with
// stack frame relative addressing: each thread has its own stack frame, so the measurement is thread safe without any
// per-thread state.
// XCP_NOINLINE: the stack frame layout must be the same for every call site, which needs a single copy of the function.
// The A2L registration is done by a priming call from main() with prime=true, after A2lInit(): the registration must run in
// this function (A2lSetStackAddrMode needs its stack frame), but it must not run inside the malloc hook, because it
// allocates and would re-enter the hook.

XCP_NOINLINE static void malloc_leave(uint8_t kind, uint64_t size, uint64_t ptr, uint64_t return_address, uint32_t thread_id, uint32_t depth, uint8_t origin, bool prime) {

    if (prime) {
        A2lSetStackAddrMode(malloc_leave);
        A2lCreateEnumConversion(malloc_kind, "3 0 \"malloc\" 1 \"calloc\" 2 \"aligned_alloc\"");
        A2lCreatePhysMeasurement(kind, "Allocation function", "conv.malloc_kind", 0, 2);
        A2lCreateMeasurement(size, "Requested size in bytes");
        A2lCreateMeasurement(ptr, "Returned pointer");
        A2lCreateMeasurement(return_address, "Return address, the caller of the allocation function");
        A2lCreateMeasurement(thread_id, "Thread id of the caller");
        A2lCreateMeasurement(depth, "Nesting depth of hooked function calls");
        A2lCreateEnumConversion(malloc_origin, "3 0 \"APP\" 1 \"XCP_INIT\" 2 \"XCP_THREAD\"");
        A2lCreatePhysMeasurement(origin, "Origin of the call: application, XCP init or XCP thread", "conv.malloc_origin", 0, 2);
    }

    // Statistics for the mainloop event and the console
    if (!prime) {
        switch (origin) {
        case MALLOC_ORIGIN_APP:
            __atomic_fetch_add(&malloc_count_app, 1, __ATOMIC_RELAXED);
            break;
        case MALLOC_ORIGIN_XCP_INIT:
            __atomic_fetch_add(&malloc_count_xcp_init, 1, __ATOMIC_RELAXED);
            break;
        default:
            __atomic_fetch_add(&malloc_count_xcp_threads, 1, __ATOMIC_RELAXED);
            break;
        }
        __atomic_fetch_add(&malloc_bytes_total, size, __ATOMIC_RELAXED);
        malloc_caller_record((uintptr_t)return_address, (size_t)size);
    }

    // XCP: Trigger the event, measures the local variables above
    DaqTriggerEvent(malloc_leave);
}

//-----------------------------------------------------------------------------------------------------
// Frida hook callbacks
//
// One listener is attached to all four functions, the attach options carry a tag to tell them apart.

typedef enum { HOOK_FOO = 1, HOOK_MALLOC = 2, HOOK_CALLOC = 3, HOOK_ALIGNED_ALLOC = 4 } hook_id_t;

static void on_enter(GumInvocationContext *ic, gpointer user_data) {
    (void)user_data;
    switch ((hook_id_t)GPOINTER_TO_SIZE(gum_invocation_context_get_listener_function_data(ic))) {

    case HOOK_FOO: {
        // Per-invocation scratch memory provided by Frida, survives from on_enter to on_leave of this call
        uint64_t *enter_time = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
        *enter_time = ApplXcpGetClock64();

        // Integer arguments are read from the argument registers or the stack according to the calling convention
        foo_ctx.arg_a = (int32_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 0));
        foo_ctx.arg_iterations = (uint32_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 1));

        // XCP: Trigger the event with the register context of this invocation as base pointer for relative addressing
        DaqTriggerEventExt(foo_enter, ic->cpu_context);
        break;
    }

    case HOOK_MALLOC: {
        // The argument registers are clobbered when the function returns, remember the requested size for on_leave
        uint64_t *size = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
        *size = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 0)); // malloc(size)
        break;
    }
    case HOOK_CALLOC: {
        uint64_t *size = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
        *size = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 0)) *
                (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 1)); // calloc(count, size)
        break;
    }
    case HOOK_ALIGNED_ALLOC: {
        uint64_t *size = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
        *size = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 1)); // aligned_alloc(alignment, size)
        break;
    }
    }
}

static void on_leave(GumInvocationContext *ic, gpointer user_data) {
    (void)user_data;
    hook_id_t hook_id = (hook_id_t)GPOINTER_TO_SIZE(gum_invocation_context_get_listener_function_data(ic));
    switch (hook_id) {

    case HOOK_FOO: {
        uint64_t *enter_time = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
        uint64_t now = ApplXcpGetClock64();
        foo_ctx.duration_ns = (uint32_t)(now - *enter_time);
        foo_ctx.ret = (int32_t)GPOINTER_TO_SIZE(gum_invocation_context_get_return_value(ic));
        foo_ctx.call_count++;
        foo_ctx.thread_id = gum_invocation_context_get_thread_id(ic);
        foo_ctx.depth = gum_invocation_context_get_depth(ic);
        foo_ctx.return_address = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_return_address(ic));
        foo_ctx.function = (uint64_t)GPOINTER_TO_SIZE(ic->function);

        // XCP: Trigger the event with the register context of this invocation as base pointer for relative addressing
        DaqTriggerEventExt(foo_leave, ic->cpu_context);
        break;
    }

    case HOOK_MALLOC:
    case HOOK_CALLOC:
    case HOOK_ALIGNED_ALLOC: {
        uint8_t kind = (uint8_t)(hook_id - HOOK_MALLOC); // 0 = malloc, 1 = calloc, 2 = aligned_alloc
        uint64_t *size = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
        uint64_t ptr = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_return_value(ic));
        uint64_t return_address = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_return_address(ic));
        uint32_t thread_id = gum_invocation_context_get_thread_id(ic);
        uint32_t depth = gum_invocation_context_get_depth(ic);
        uint8_t origin;
        if ((GumThreadId)thread_id != main_thread_id) {
            origin = MALLOC_ORIGIN_XCP_THREAD;
        } else if (atomic_load_explicit(&run_phase, memory_order_relaxed)) {
            origin = MALLOC_ORIGIN_APP;
        } else {
            origin = MALLOC_ORIGIN_XCP_INIT;
        }
        malloc_leave(kind, *size, ptr, return_address, thread_id, depth, origin, false);
        break;
    }
    }
}

//-----------------------------------------------------------------------------------------------------
// A2L registration of the Frida register context
//
// GumCpuContext is the register snapshot Frida takes in the trampoline. It lives in the trampoline's stack frame, so its
// address is different for every invocation: the hooks pass it as base pointer to DaqTriggerEventExt(), and the A2L
// describes the registers as offsets from that base pointer (relative addressing mode, address extension 3).
// A dummy instance is used to compute the offsets, its address is never accessed.

static GumCpuContext dummy_cpu_context;

static void register_cpu_context_typedef(void) {
    A2lTypedefBegin(GumCpuContext, &dummy_cpu_context, "Frida Gum CPU register context");
#if defined(__aarch64__)
    A2lTypedefMeasurementComponent(pc, "Program counter");
    A2lTypedefMeasurementComponent(sp, "Stack pointer");
    A2lTypedefMeasurementComponent(nzcv, "Condition flags");
    A2lTypedefMeasurementArrayComponent(x, "General purpose registers x0..x28, x0 = first argument and return value");
    A2lTypedefMeasurementComponent(fp, "Frame pointer x29");
    A2lTypedefMeasurementComponent(lr, "Link register x30");
#elif defined(__x86_64__)
    A2lTypedefMeasurementComponent(rip, "Instruction pointer");
    A2lTypedefMeasurementComponent(rsp, "Stack pointer");
    A2lTypedefMeasurementComponent(rbp, "Base pointer");
    A2lTypedefMeasurementComponent(rax, "rax, return value");
    A2lTypedefMeasurementComponent(rbx, "rbx");
    A2lTypedefMeasurementComponent(rcx, "rcx, 4th argument");
    A2lTypedefMeasurementComponent(rdx, "rdx, 3rd argument");
    A2lTypedefMeasurementComponent(rsi, "rsi, 2nd argument");
    A2lTypedefMeasurementComponent(rdi, "rdi, 1st argument");
    A2lTypedefMeasurementComponent(r8, "r8, 5th argument");
    A2lTypedefMeasurementComponent(r9, "r9, 6th argument");
    A2lTypedefMeasurementComponent(r10, "r10");
    A2lTypedefMeasurementComponent(r11, "r11");
    A2lTypedefMeasurementComponent(r12, "r12");
    A2lTypedefMeasurementComponent(r13, "r13");
    A2lTypedefMeasurementComponent(r14, "r14");
    A2lTypedefMeasurementComponent(r15, "r15");
#else
#error "frida_demo: register context not described for this architecture"
#endif
    A2lTypedefEnd();
}

//-----------------------------------------------------------------------------------------------------
// Console statistics, called from the main loop

static void print_statistics(GumInterceptor *interceptor) {

    printf("foo: %u calls, last duration %u ns, arg_a=%d ret=%d\n", foo_ctx.call_count, foo_ctx.duration_ns, foo_ctx.arg_a, foo_ctx.ret);
    printf("malloc: app=%u xcp_init=%u xcp_threads=%u, %llu bytes total\n", __atomic_load_n(&malloc_count_app, __ATOMIC_RELAXED),
           __atomic_load_n(&malloc_count_xcp_init, __ATOMIC_RELAXED), __atomic_load_n(&malloc_count_xcp_threads, __ATOMIC_RELAXED),
           (unsigned long long)__atomic_load_n(&malloc_bytes_total, __ATOMIC_RELAXED));

    // Resolve the caller addresses to symbol names. This allocates, so the malloc hook is switched off for this thread.
    // The Gum symbol resolver needs debug symbols and does not work everywhere (not with the macOS devkit), dladdr() is
    // the fallback: it resolves the symbols of the executable (Linux needs -rdynamic for that, see CMakeLists.txt) and of
    // the shared libraries. The return address is the immediate caller of malloc(), a libc function like fopen() which
    // allocates on behalf of XCPlite shows up under its own name.
    gum_interceptor_ignore_current_thread(interceptor);
    printf("malloc callers:\n");
    for (size_t i = 0; i < MALLOC_CALLER_TABLE_SIZE; i++) {
        uintptr_t address = atomic_load_explicit(&malloc_callers[i].address, memory_order_acquire);
        if (address == 0)
            continue;
        unsigned int count = atomic_load(&malloc_callers[i].count);
        unsigned long long bytes = atomic_load(&malloc_callers[i].bytes);
        GumDebugSymbolDetails details;
        Dl_info info;
        if (gum_symbol_details_from_address(GSIZE_TO_POINTER(address), &details)) {
            printf("  %8u calls %10llu bytes  %s (%s:%u)\n", count, bytes, details.symbol_name, details.file_name, details.line_number);
        } else if (dladdr(GSIZE_TO_POINTER(address), &info) != 0 && info.dli_sname != NULL) {
            const char *module = strrchr(info.dli_fname, '/');
            printf("  %8u calls %10llu bytes  %s+%lu (%s)\n", count, bytes, info.dli_sname, (unsigned long)(address - (uintptr_t)info.dli_saddr),
                   module != NULL ? module + 1 : info.dli_fname);
        } else {
            printf("  %8u calls %10llu bytes  0x%lx\n", count, bytes, (unsigned long)address);
        }
    }
    gum_interceptor_unignore_current_thread(interceptor);
}

//-----------------------------------------------------------------------------------------------------
// Main

static volatile bool running = true;
static void sig_handler(int sig) {
    (void)sig;
    running = false;
}

int main(int argc, char *argv[]) {
    (void)argc;

    printf("\nXCP on Ethernet frida_demo V%s - %s\n", OPTION_PROJECT_VERSION, argv[0]);
    signal(SIGINT, sig_handler);
    signal(SIGTERM, sig_handler);

    // Frida: initialize the embedded Gum runtime and hook foo() and the allocation functions before anything else
    // foo() is our own function, its address is known. The libc functions are found by name in the loaded modules.
    // The allocation hooks are attached before XcpInit(), so the allocations of the XCP initialization are counted as well.
    // The hooks are safe before XcpInit(): the XCP event triggers are passive until XCP is started.
    gum_init_embedded();
    main_thread_id = gum_process_get_current_thread_id();

    GumInterceptor *interceptor = gum_interceptor_obtain();
    GumInvocationListener *listener = gum_make_call_listener(on_enter, on_leave, NULL, NULL);
    const struct {
        const char *name;
        gpointer address;
        hook_id_t id;
    } hooks[] = {
        {"foo", (gpointer)foo, HOOK_FOO},
        {"malloc", GSIZE_TO_POINTER(gum_module_find_global_export_by_name("malloc")), HOOK_MALLOC},
        {"calloc", GSIZE_TO_POINTER(gum_module_find_global_export_by_name("calloc")), HOOK_CALLOC},
        {"aligned_alloc", GSIZE_TO_POINTER(gum_module_find_global_export_by_name("aligned_alloc")), HOOK_ALIGNED_ALLOC},
    };
    const size_t hook_count = sizeof(hooks) / sizeof(hooks[0]);
    GumAttachOptions options[sizeof(hooks) / sizeof(hooks[0])] = {{{0}, 0, 0}};
    GumAttachReturn results[sizeof(hooks) / sizeof(hooks[0])];

    gum_interceptor_begin_transaction(interceptor);
    for (size_t i = 0; i < hook_count; i++) {
        options[i].listener_function_data = GSIZE_TO_POINTER(hooks[i].id);
        results[i] = gum_interceptor_attach(interceptor, hooks[i].address, listener, &options[i]);
    }
    gum_interceptor_end_transaction(interceptor); // The code patches are applied here

    bool attached = true;
    for (size_t i = 0; i < hook_count; i++) {
        printf("Frida: hook %s() at %p: %s (%d)\n", hooks[i].name, hooks[i].address, results[i] == GUM_ATTACH_OK ? "ok" : "FAILED", (int)results[i]);
        if (results[i] != GUM_ATTACH_OK)
            attached = false;
    }
    if (!attached)
        return 1;

    // XCP: Set log level (1-error, 2-warning, 3-info, 4-show XCP commands)
    XcpSetLogLevel(OPTION_LOG_LEVEL);

    // XCP: Initialize the XCP singleton, activate XCP
    if (!XcpInit(OPTION_PROJECT_NAME, OPTION_PROJECT_VERSION, OPTION_XCP_MODE)) {
        printf("Failed to initialize XCP\n");
        return 1;
    }
    XcpSetElfName(argv[0]); // ELF file name for upload via GET_ID, optional

    // XCP: Initialize the XCP server
    uint8_t addr[4] = OPTION_SERVER_ADDR;
    if (!XcpEthServerInit(addr, OPTION_SERVER_PORT, OPTION_USE_TCP, OPTION_QUEUE_SIZE)) {
        return 1;
    }

    // XCP: Enable runtime A2L generation
    if (!A2lInit(addr, OPTION_SERVER_PORT, OPTION_USE_TCP, OPTION_A2L_MODE)) {
        return 1;
    }

    // XCP: Calibration segment and parameters
    params_calseg = XcpCreateCalSeg("params", &params, sizeof(params));
    assert(params_calseg != XCP_UNDEFINED_CALSEG);
    A2lSetSegmentAddrMode(params_calseg, params);
    A2lCreateParameter(params.delay_us, "Main loop period", "us", 0, 1000000);
    A2lCreateParameter(params.foo_iterations, "Work done by foo(), changes its execution time", "", 0, 10000000);
    A2lCreateParameter(params.malloc_size, "Bytes requested by the main loop on each cycle", "bytes", 0, 1000000);
    A2lCreateParameter(params.foo_enabled, "Call foo() from the main loop", "", 0, 1);

    // XCP: Register the Frida register context typedef
    register_cpu_context_typedef();

    // XCP: Event foo_enter, arguments (absolute addressing) and register context (relative addressing to the base pointer of the event)
    A2lSetAbsoluteAddrMode(foo_enter);
    A2lCreateMeasurement(foo_ctx.arg_a, "foo() argument a");
    A2lCreateMeasurement(foo_ctx.arg_iterations, "foo() argument iterations");
    GumCpuContext *foo_enter_cpu_context = &dummy_cpu_context; // Placeholder, offset 0 from the base pointer of the event
    A2lSetRelativeAddrMode(foo_enter, foo_enter_cpu_context);
    A2lCreateTypedefReference(foo_enter_cpu_context, GumCpuContext, "Register context at entry of foo()");

    // XCP: Event foo_leave, results (absolute addressing) and register context (relative addressing)
    A2lSetAbsoluteAddrMode(foo_leave);
    A2lCreateMeasurement(foo_ctx.ret, "foo() return value");
    A2lCreatePhysMeasurement(foo_ctx.duration_ns, "foo() execution time", "ns", 0, 1000000);
    A2lCreateMeasurement(foo_ctx.call_count, "foo() call counter");
    A2lCreateMeasurement(foo_ctx.thread_id, "Thread id of the caller of foo()");
    A2lCreateMeasurement(foo_ctx.depth, "Nesting depth of hooked function calls");
    A2lCreateMeasurement(foo_ctx.return_address, "Return address, the call site of foo()");
    A2lCreateMeasurement(foo_ctx.function, "Address of foo()");
    GumCpuContext *foo_leave_cpu_context = &dummy_cpu_context;
    A2lSetRelativeAddrMode(foo_leave, foo_leave_cpu_context);
    A2lCreateTypedefReference(foo_leave_cpu_context, GumCpuContext, "Register context at return of foo()");

    // XCP: Event mainloop, global statistics
    uint32_t counter = 0;
    A2lSetAbsoluteAddrMode(mainloop);
    A2lCreateMeasurement(malloc_count_app, "Allocations by the application (main thread, run phase)");
    A2lCreateMeasurement(malloc_count_xcp_init, "Allocations during XCP initialization (main thread, init phase)");
    A2lCreateMeasurement(malloc_count_xcp_threads, "Allocations by the XCP server threads");
    A2lCreateMeasurement(malloc_bytes_total, "Total bytes requested");
    A2lSetStackAddrMode(mainloop);
    A2lCreateMeasurement(counter, "Main loop counter");

    // XCP: Event malloc_leave, priming call: registers the stack frame relative measurements of malloc_leave() outside the hook
    malloc_leave(0, 0, 0, 0, 0, 0, 0, true);

    // From now on, allocations on the main thread are the application's
    atomic_store(&run_phase, true);
    printf("XCP initialization done: %u allocations\n", __atomic_load_n(&malloc_count_xcp_init, __ATOMIC_RELAXED));

    // Main loop
    printf("Start main loop... (Ctrl+C to stop)\n");
    uint64_t last_print = ApplXcpGetClock64();
    while (running) {

        // XCP: Lock the calibration segment, wait-free
        const params_t *p = (const params_t *)XcpLockCalSeg(params_calseg);
        uint32_t delay_us = p->delay_us;

        // Call the hooked function
        if (p->foo_enabled) {
            (void)foo((int32_t)counter, p->foo_iterations);
        }

        // Call the hooked libc function
        void *mem = malloc(p->malloc_size);
        free(mem);

        // XCP: Unlock the calibration segment
        XcpUnlockCalSeg(params_calseg);

        counter++;

        // XCP: Trigger the mainloop event
        DaqTriggerEvent(mainloop);

        // Statistics every 2 seconds
        uint64_t now = ApplXcpGetClock64();
        if (now - last_print > 2000000000ULL) {
            last_print = now;
            print_statistics(interceptor);
            fflush(stdout);
        }

        usleep(delay_us);
    }

    // Frida: remove the hooks
    gum_interceptor_detach(interceptor, listener);
    g_object_unref(listener);
    g_object_unref(interceptor);

    // XCP: Shutdown
    XcpDisconnect();
    A2lFinalize();
    XcpEthServerShutdown();

    gum_deinit_embedded();
    return 0;
}
