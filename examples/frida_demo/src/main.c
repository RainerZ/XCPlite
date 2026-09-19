// frida_demo - XCPlite measurement of functions hooked with Frida Gum
//
// Use case 1: the function foo() below is called cyclically by the main loop. It contains no XCPlite instrumentation at all.
// The Frida Gum Interceptor is used to hook foo(): Frida patches the first instructions of foo() with a jump into a
// trampoline, which calls our foo_on_enter() callback before and our foo_on_leave() callback after the original function code.
// Both callbacks run synchronously on the thread which called foo(), and both trigger an XCP DAQ event right there.
// The XCP client can then measure everything Frida hands us in the callback: the arguments, the return value, the call
// duration, thread id and call depth, and the complete CPU register snapshot Frida takes for the invocation.
//
// Use case 2 (OPTION_HOOK_ALLOC): a second, independent listener hooks the libc allocation functions malloc(), calloc() and
// aligned_alloc(), functions we do not even own: every allocation in the process, from any thread, triggers an XCP event
// with the requested size. The callers are recorded, so it is visible how few allocations the XCPlite library itself does
// (one at initialization, none afterwards).
//
// Rules for code which runs inside a Frida callback:
// - The XCP event trigger functions are lock-free and never allocate, so they are safe to call from any hook.
// - Frida has no re-entrancy guard for listeners: a hooked function called from inside a callback is intercepted again.
//   The allocation hook therefore must not call anything which may allocate (printf, A2L registration, symbol lookup), it
//   would recurse into itself. All A2L registration is done in the init phase from main(), nothing in a hook calls A2l*.

#include <assert.h>
#include <signal.h>
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
// Options

// Use case 2: hook the libc allocation functions
// Comment out to reduce the example to the interception of foo()
// #define OPTION_HOOK_ALLOC

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
    uint8_t foo_enabled;     // Call foo() from the main loop
} params_t;

// Default values (reference page, "FLASH")
const params_t params = {.delay_us = 10000, .foo_iterations = 1000, .foo_enabled = 1};

// Calibration segment handle
tXcpCalSegIndex params_calseg = XCP_UNDEFINED_CALSEG;

//=====================================================================================================
// Use case 1: hook foo()
//=====================================================================================================

//-----------------------------------------------------------------------------------------------------
// XCP events for foo

#ifdef OPTION_SECTION_REGISTRATION

DaqDeclareEvent(foo_enter);
DaqDeclareEvent(foo_leave);

#else

// The events are created at runtime with DaqCreateEvent() where their measurements are registered
// (default configuration of xcplite, dynamic event management).
// The trigger macros in the Frida callbacks look up the event by name on their first execution, so the events need not be visible at file scope.

#endif

//-----------------------------------------------------------------------------------------------------
// The function to be hooked
//
// No XCPlite instrumentation. XCP_NOINLINE is needed: Frida patches the code of the function, an inlined copy at the call
// site would not be hooked. The function must also be larger than the few instructions the trampoline jump replaces.

XCP_NOINLINE int32_t foo(uint32_t a, uint32_t iterations) {
    uint32_t x = a;
    for (uint32_t i = 0; i < iterations; i++) {
        x = x + 1;
    }
    return x;
}

//-----------------------------------------------------------------------------------------------------
// Call context of foo(), filled by the hook callbacks
//
// foo() is called from the main thread only, so a single global instance is sufficient (absolute addressing mode).
// For a function called from several threads, the per-invocation data of the Frida invocation context, thread local
// storage or the stack frame relative addressing mode (see the allocation hook) would be used instead.

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
// Frida hook callbacks for foo()

static void foo_on_enter(GumInvocationContext *ic, gpointer user_data) {
    (void)user_data;

    // Per-invocation scratch memory provided by Frida, survives from on_enter to on_leave of this call
    uint64_t *enter_time = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
    *enter_time = ApplXcpGetClock64();

    // Integer arguments are read from the argument registers or the stack according to the calling convention
    foo_ctx.arg_a = (int32_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 0));
    foo_ctx.arg_iterations = (uint32_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 1));

    // XCP: Trigger the event with the register context of this invocation as base pointer for relative addressing
    // @@@@ TODO: xcpclient does not handle dynamic addressing mode, copy it for now to a local static, which gets the correct event id
    static GumCpuContext foo_enter_cpu_context;
    memcpy(&foo_enter_cpu_context, ic->cpu_context, sizeof(GumCpuContext));

    DaqTriggerEventExt(foo_enter, ic->cpu_context);
}

static void foo_on_leave(GumInvocationContext *ic, gpointer user_data) {
    (void)user_data;

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
}

//-----------------------------------------------------------------------------------------------------
// A2L registration for the foo() hook
//
// GumCpuContext is the register snapshot Frida takes in the trampoline. It lives in the trampoline's stack frame, so its
// address is different for every invocation: the hooks pass it as base pointer to DaqTriggerEventExt(), and the A2L
// describes the registers as offsets from that base pointer (relative addressing mode, address extension 3).

#ifndef OPTION_SECTION_REGISTRATION

static void foo_hook_register_a2l(void) {

    // Typedef for the register context
    A2lTypedefBegin(GumCpuContext, NULL, "Frida Gum CPU register context");
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

    // Event foo_enter: arguments (absolute addressing) and register context (relative addressing to the base pointer of the event)
    DaqCreateEvent(foo_enter);
    A2lSetAbsoluteAddrMode(foo_enter);
    A2lCreateMeasurement(foo_ctx.arg_a, "foo() argument a");
    A2lCreateMeasurement(foo_ctx.arg_iterations, "foo() argument iterations");
    GumCpuContext *foo_enter_cpu_context = NULL; // Placeholder, offset 0 from the base pointer of the event
    A2lSetRelativeAddrMode(foo_enter, foo_enter_cpu_context);
    A2lCreateTypedefReference(foo_enter_cpu_context, GumCpuContext, "Register context at entry of foo()");

    // Event foo_leave: results (absolute addressing) and register context (relative addressing)
    DaqCreateEvent(foo_leave);
    A2lSetAbsoluteAddrMode(foo_leave);
    A2lCreateMeasurement(foo_ctx.ret, "foo() return value");
    A2lCreatePhysMeasurement(foo_ctx.duration_ns, "foo() execution time", "ns", 0, 1000000);
    A2lCreateMeasurement(foo_ctx.call_count, "foo() call counter");
    A2lCreateMeasurement(foo_ctx.thread_id, "Thread id of the caller of foo()");
    A2lCreateMeasurement(foo_ctx.depth, "Nesting depth of hooked function calls");
    A2lCreateMeasurement(foo_ctx.return_address, "Return address, the call site of foo()");
    A2lCreateMeasurement(foo_ctx.function, "Address of foo()");
    GumCpuContext *foo_leave_cpu_context = NULL; // Placeholder, offset 0 from the base pointer of the event
    A2lSetRelativeAddrMode(foo_leave, foo_leave_cpu_context);
    A2lCreateTypedefReference(foo_leave_cpu_context, GumCpuContext, "Register context at return of foo()");
}

#endif

//-----------------------------------------------------------------------------------------------------
// Attach the foo() hook
//
// foo() is our own function, its address is known, no symbol lookup is needed.

static GumInvocationListener *foo_listener = NULL;

static bool foo_hook_attach(GumInterceptor *interceptor) {

    foo_listener = gum_make_call_listener(foo_on_enter, foo_on_leave, NULL, NULL);

    gum_interceptor_begin_transaction(interceptor);
    GumAttachReturn result = gum_interceptor_attach(interceptor, (gpointer)foo, foo_listener, NULL);
    gum_interceptor_end_transaction(interceptor); // The code patch is applied here

    printf("Frida: hook foo() at %p: %s (%d)\n", (void *)foo, result == GUM_ATTACH_OK ? "ok" : "FAILED", (int)result);
    return result == GUM_ATTACH_OK;
}

static void foo_hook_detach(GumInterceptor *interceptor) {
    gum_interceptor_detach(interceptor, foo_listener);
    g_object_unref(foo_listener);
    foo_listener = NULL;
}

static void foo_print_statistics(void) { printf("foo: %u calls, last duration %u ns, arg_a=%d ret=%d\n", foo_ctx.call_count, foo_ctx.duration_ns, foo_ctx.arg_a, foo_ctx.ret); }

//=====================================================================================================
// Use case 2: hook the libc allocation functions
//=====================================================================================================

#ifdef OPTION_HOOK_ALLOC

#include <dlfcn.h>
#include <stdatomic.h>

// Platform note: on Linux (glibc) every allocation is seen, also the ones libc does internally (fopen, pthread_create, ...).
// On macOS the libc internals call the zone allocator directly inside the dyld shared cache and never pass the exported
// malloc(), only the calls from code outside the shared cache are seen: the application and the statically linked xcplite.

//-----------------------------------------------------------------------------------------------------
// Allocation statistics, updated by the hook callbacks from any thread

// Where does an allocation come from
typedef enum {
    ALLOC_ORIGIN_APP = 0,        // Main thread in the run phase: the application
    ALLOC_ORIGIN_XCP_INIT = 1,   // Main thread in the init phase: XCP and A2L initialization, calibration segment, A2L registration
    ALLOC_ORIGIN_XCP_THREAD = 2, // Another thread: the XCP server threads (the only other threads in this process)
} alloc_origin_t;

// Plain types (not _Atomic) so the A2L type detection works, updated with the atomic builtins
static uint32_t alloc_count_app = 0;
static uint32_t alloc_count_xcp_init = 0;
static uint32_t alloc_count_xcp_threads = 0;
static uint64_t alloc_bytes_total = 0;

// Caller histogram: return address of the allocation function -> count, bytes. Fixed size open addressing, no allocation
#define ALLOC_CALLER_TABLE_SIZE 256
typedef struct {
    atomic_uintptr_t address;
    atomic_uint count;
    atomic_ullong bytes;
} alloc_caller_t;
static alloc_caller_t alloc_callers[ALLOC_CALLER_TABLE_SIZE];

static void alloc_caller_record(uintptr_t address, size_t size) {
    size_t i = (address >> 2) % ALLOC_CALLER_TABLE_SIZE;
    for (size_t n = 0; n < ALLOC_CALLER_TABLE_SIZE; n++) {
        uintptr_t expected = 0;
        if (atomic_load_explicit(&alloc_callers[i].address, memory_order_acquire) == address || atomic_compare_exchange_strong(&alloc_callers[i].address, &expected, address)) {
            atomic_fetch_add_explicit(&alloc_callers[i].count, 1, memory_order_relaxed);
            atomic_fetch_add_explicit(&alloc_callers[i].bytes, size, memory_order_relaxed);
            return;
        }
        i = (i + 1) % ALLOC_CALLER_TABLE_SIZE;
    }
    // Table full, drop
}

// Phase flag: allocations on the main thread are attributed to XCP init until main() sets this
static atomic_bool alloc_run_phase = false;
static GumThreadId alloc_main_thread_id;

//-----------------------------------------------------------------------------------------------------
// Allocation hook helper
//
// Called from the on_leave callback of malloc(), calloc() and aligned_alloc() on any thread. The values are held in local
// variables and measured with stack frame relative addressing: each thread has its own stack frame, so the measurement is
// thread safe without any per-thread state.
// XCP_NOINLINE: the stack frame layout must be the same for every call site, which needs a single copy of the function.
// The A2L registration is done by a priming call from main() with prime=true, after A2lInit(): the registration must run in
// this function (A2lSetStackAddrMode needs its stack frame), but it must not run inside the hook, because it allocates and
// would re-enter the hook.

#ifdef OPTION_SECTION_REGISTRATION
DaqDeclareEvent(alloc_enter);
DaqDeclareEvent(alloc_leave);
#endif

XCP_NOINLINE static void alloc_leave(uint8_t alloc_kind, uint64_t alloc_size, uint64_t alloc_ptr, uint64_t alloc_return_address, uint32_t alloc_thread_id, uint32_t alloc_depth,
                                     uint8_t alloc_origin, bool prime) {

    if (prime) {
        DaqCreateEvent(alloc_leave);
        A2lSetStackAddrMode(alloc_leave);
        A2lCreateEnumConversion(alloc_kind, "3 0 \"malloc\" 1 \"calloc\" 2 \"aligned_alloc\"");
        A2lCreatePhysMeasurement(alloc_kind, "Allocation function", "conv.alloc_kind", 0, 2);
        A2lCreateMeasurement(alloc_size, "Requested size in bytes");
        A2lCreateMeasurement(alloc_ptr, "Returned pointer");
        A2lCreateMeasurement(alloc_return_address, "Return address, the caller of the allocation function");
        A2lCreateMeasurement(alloc_thread_id, "Thread id of the caller");
        A2lCreateMeasurement(alloc_depth, "Nesting depth of hooked function calls");
        A2lCreateEnumConversion(alloc_origin, "3 0 \"APP\" 1 \"XCP_INIT\" 2 \"XCP_THREAD\"");
        A2lCreatePhysMeasurement(alloc_origin, "Origin of the call: application, XCP init or XCP thread", "conv.alloc_origin", 0, 2);
    }

    // Statistics for the mainloop event and the console
    if (!prime) {
        switch (alloc_origin) {
        case ALLOC_ORIGIN_APP:
            __atomic_fetch_add(&alloc_count_app, 1, __ATOMIC_RELAXED);
            break;
        case ALLOC_ORIGIN_XCP_INIT:
            __atomic_fetch_add(&alloc_count_xcp_init, 1, __ATOMIC_RELAXED);
            break;
        default:
            __atomic_fetch_add(&alloc_count_xcp_threads, 1, __ATOMIC_RELAXED);
            break;
        }
        __atomic_fetch_add(&alloc_bytes_total, alloc_size, __ATOMIC_RELAXED);
        alloc_caller_record((uintptr_t)alloc_return_address, (size_t)alloc_size);
    }

    // XCP: Trigger the event, measures the local variables above
    DaqTriggerEvent(alloc_leave);
}

//-----------------------------------------------------------------------------------------------------
// Frida hook callbacks for the allocation functions
//
// One listener is attached to three functions, the attach options carry a tag to tell them apart.

typedef enum { HOOK_MALLOC = 0, HOOK_CALLOC = 1, HOOK_ALIGNED_ALLOC = 2 } alloc_hook_id_t;

static void alloc_on_enter(GumInvocationContext *ic, gpointer user_data) {
    (void)user_data;

    // The argument registers are clobbered when the function returns, remember the requested size for on_leave
    uint64_t *size = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
    switch ((alloc_hook_id_t)GPOINTER_TO_SIZE(gum_invocation_context_get_listener_function_data(ic))) {
    case HOOK_MALLOC: // malloc(size)
        *size = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 0));
        break;
    case HOOK_CALLOC: // calloc(count, size)
        *size = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 0)) * (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 1));
        break;
    case HOOK_ALIGNED_ALLOC: // aligned_alloc(alignment, size)
        *size = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_nth_argument(ic, 1));
        break;
    }
}

static void alloc_on_leave(GumInvocationContext *ic, gpointer user_data) {
    (void)user_data;

    uint8_t kind = (uint8_t)GPOINTER_TO_SIZE(gum_invocation_context_get_listener_function_data(ic));
    uint64_t *size = GUM_IC_GET_INVOCATION_DATA(ic, uint64_t);
    uint64_t ptr = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_return_value(ic));
    uint64_t return_address = (uint64_t)GPOINTER_TO_SIZE(gum_invocation_context_get_return_address(ic));
    uint32_t thread_id = gum_invocation_context_get_thread_id(ic);
    uint32_t depth = gum_invocation_context_get_depth(ic);
    uint8_t origin;
    if ((GumThreadId)thread_id != alloc_main_thread_id) {
        origin = ALLOC_ORIGIN_XCP_THREAD;
    } else if (atomic_load_explicit(&alloc_run_phase, memory_order_relaxed)) {
        origin = ALLOC_ORIGIN_APP;
    } else {
        origin = ALLOC_ORIGIN_XCP_INIT;
    }
    alloc_leave(kind, *size, ptr, return_address, thread_id, depth, origin, false);
}

//-----------------------------------------------------------------------------------------------------
// A2L registration for the allocation hook, called from main() after A2lInit()

static void alloc_hook_register_a2l(void) {

    // Event mainloop: global statistics
    A2lSetAbsoluteAddrMode(mainloop);
    A2lCreateMeasurement(alloc_count_app, "Allocations by the application (main thread, run phase)");
    A2lCreateMeasurement(alloc_count_xcp_init, "Allocations during XCP initialization (main thread, init phase)");
    A2lCreateMeasurement(alloc_count_xcp_threads, "Allocations by the XCP server threads");
    A2lCreateMeasurement(alloc_bytes_total, "Total bytes requested");

    // Event alloc_leave, priming call: registers the stack frame relative measurements of alloc_leave() outside the hook
    alloc_leave(0, 0, 0, 0, 0, 0, 0, true);
}

//-----------------------------------------------------------------------------------------------------
// Attach the allocation hooks
//
// The libc functions are found by name in the loaded modules.
// Attached before XcpInit(), so the allocations of the XCP initialization are counted as well. This is safe, the XCP event
// triggers are passive until XCP is started.

static GumInvocationListener *alloc_listener = NULL;

static bool alloc_hook_attach(GumInterceptor *interceptor) {

    alloc_main_thread_id = gum_process_get_current_thread_id();
    alloc_listener = gum_make_call_listener(alloc_on_enter, alloc_on_leave, NULL, NULL);

    const struct {
        const char *name;
        alloc_hook_id_t id;
    } hooks[] = {{"malloc", HOOK_MALLOC}, {"calloc", HOOK_CALLOC}, {"aligned_alloc", HOOK_ALIGNED_ALLOC}};
    const size_t hook_count = sizeof(hooks) / sizeof(hooks[0]);
    gpointer addresses[sizeof(hooks) / sizeof(hooks[0])];
    GumAttachOptions options[sizeof(hooks) / sizeof(hooks[0])] = {{{0}, 0, 0}};
    GumAttachReturn results[sizeof(hooks) / sizeof(hooks[0])];

    gum_interceptor_begin_transaction(interceptor);
    for (size_t i = 0; i < hook_count; i++) {
        addresses[i] = GSIZE_TO_POINTER(gum_module_find_global_export_by_name(hooks[i].name));
        options[i].listener_function_data = GSIZE_TO_POINTER(hooks[i].id);
        results[i] = gum_interceptor_attach(interceptor, addresses[i], alloc_listener, &options[i]);
    }
    gum_interceptor_end_transaction(interceptor); // The code patches are applied here

    bool attached = true;
    for (size_t i = 0; i < hook_count; i++) {
        printf("Frida: hook %s() at %p: %s (%d)\n", hooks[i].name, addresses[i], results[i] == GUM_ATTACH_OK ? "ok" : "FAILED", (int)results[i]);
        if (results[i] != GUM_ATTACH_OK)
            attached = false;
    }
    return attached;
}

static void alloc_hook_detach(GumInterceptor *interceptor) {
    gum_interceptor_detach(interceptor, alloc_listener);
    g_object_unref(alloc_listener);
    alloc_listener = NULL;
}

// End of the init phase, from now on allocations on the main thread are the application's
static void alloc_hook_start_run_phase(void) {
    atomic_store(&alloc_run_phase, true);
    printf("XCP initialization done: %u allocations\n", __atomic_load_n(&alloc_count_xcp_init, __ATOMIC_RELAXED));
}

//-----------------------------------------------------------------------------------------------------
// Console statistics, called from the main loop

static void alloc_print_statistics(GumInterceptor *interceptor) {

    printf("alloc: app=%u xcp_init=%u xcp_threads=%u, %llu bytes total\n", __atomic_load_n(&alloc_count_app, __ATOMIC_RELAXED),
           __atomic_load_n(&alloc_count_xcp_init, __ATOMIC_RELAXED), __atomic_load_n(&alloc_count_xcp_threads, __ATOMIC_RELAXED),
           (unsigned long long)__atomic_load_n(&alloc_bytes_total, __ATOMIC_RELAXED));

    // Resolve the caller addresses to symbol names. This allocates, so the hooks are switched off for this thread.
    // The Gum symbol resolver needs debug symbols and does not work everywhere (not with the macOS devkit), dladdr() is
    // the fallback: it resolves the symbols of the executable (Linux needs -rdynamic for that, see CMakeLists.txt) and of
    // the shared libraries. The return address is the immediate caller of the allocation function, a libc function like
    // fopen() which allocates on behalf of XCPlite shows up under its own name.
    gum_interceptor_ignore_current_thread(interceptor);
    printf("alloc callers:\n");
    for (size_t i = 0; i < ALLOC_CALLER_TABLE_SIZE; i++) {
        uintptr_t address = atomic_load_explicit(&alloc_callers[i].address, memory_order_acquire);
        if (address == 0)
            continue;
        unsigned int count = atomic_load(&alloc_callers[i].count);
        unsigned long long bytes = atomic_load(&alloc_callers[i].bytes);
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

#endif // OPTION_HOOK_ALLOC

//=====================================================================================================
// Main
//=====================================================================================================

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

    // Frida: initialize the embedded Gum runtime, get the process wide interceptor
    gum_init_embedded();
    GumInterceptor *interceptor = gum_interceptor_obtain();

#ifdef OPTION_HOOK_ALLOC
    // Frida: hook the allocation functions before XcpInit()
    if (!alloc_hook_attach(interceptor)) {
        return 1;
    }
#endif

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
#ifndef OPTION_SECTION_REGISTRATION
    if (!A2lInit(addr, OPTION_SERVER_PORT, OPTION_USE_TCP, OPTION_A2L_MODE)) {
        return 1;
    }
#endif

    // XCP: Calibration segment and parameters
    params_calseg = XcpCreateCalSeg("params", &params, sizeof(params));
    assert(params_calseg != XCP_UNDEFINED_CALSEG);
#ifndef OPTION_SECTION_REGISTRATION
    A2lSetSegmentAddrMode(params_calseg, params);
    A2lCreateParameter(params.delay_us, "Main loop period", "us", 0, 1000000);
    A2lCreateParameter(params.foo_iterations, "Work done by foo(), changes its execution time", "", 0, 10000000);
    A2lCreateParameter(params.foo_enabled, "Call foo() from the main loop", "", 0, 1);
#endif

    // XCP: Event mainloop
    uint32_t counter = 0;
    DaqCreateEvent(mainloop);
#ifndef OPTION_SECTION_REGISTRATION
    A2lSetStackAddrMode(mainloop);
    A2lCreateMeasurement(counter, "Main loop counter");
#endif

    // Frida: hook foo() and register its measurements
    if (!foo_hook_attach(interceptor)) {
        return 1;
    }
#ifndef OPTION_SECTION_REGISTRATION
    foo_hook_register_a2l();
#endif

#ifdef OPTION_HOOK_ALLOC
    // XCP: Register the measurements of the allocation hook, end of the init phase
    alloc_hook_register_a2l();
    alloc_hook_start_run_phase();
#endif

    // Main loop
    printf("Start main loop... (Ctrl+C to stop)\n");
    uint64_t last_print = ApplXcpGetClock64();
    while (running) {

        // XCP: Lock the calibration segment, wait-free
        const params_t *p = (const params_t *)XcpLockCalSeg(params_calseg);
        uint32_t delay_us = p->delay_us;

        // Call the hooked function
        if (p->foo_enabled) {
            (void)foo(counter, p->foo_iterations);
        }

#ifdef OPTION_HOOK_ALLOC
        // One application allocation per cycle, visible in the alloc_leave event and the statistics
        void *mem = malloc(counter & 0xFF);
        free(mem);
#endif

        // XCP: Unlock the calibration segment
        XcpUnlockCalSeg(params_calseg);

        counter++;

        // XCP: Trigger the mainloop event
        DaqTriggerEvent(mainloop);

        // Statistics every 2 seconds
        uint64_t now = ApplXcpGetClock64();
        if (now - last_print > 2000000000ULL) {
            last_print = now;
            foo_print_statistics();
#ifdef OPTION_HOOK_ALLOC
            alloc_print_statistics(interceptor);
#endif
            fflush(stdout);
        }

        usleep(delay_us);
    }

    // Frida: remove the hooks
    foo_hook_detach(interceptor);
#ifdef OPTION_HOOK_ALLOC
    alloc_hook_detach(interceptor);
#endif
    g_object_unref(interceptor);

    // XCP: Shutdown
    XcpDisconnect();
#ifndef OPTION_SECTION_REGISTRATION
    A2lFinalize();
#endif
    XcpEthServerShutdown();

    gum_deinit_embedded();
    return 0;
}
