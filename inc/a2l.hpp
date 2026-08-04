#pragma once
#define __A2L_HPP__

/*-----------------------------------------------------------------------------
| File:
|   a2l.hpp - Public C++ API for A2L generation
|
| Description:
|   Public C++ header for A2L generation
|
| Copyright (c) Vector Informatik GmbH. All rights reserved.
| See LICENSE file in the project root for details.
|
 ----------------------------------------------------------------------------*/

// ============================================================================
// Helper macros for C++ one time and thread safe A2L registrations
// Mutex protection is needed in multi-threaded contexts, because the A2L registration macros are not thread-safe
// ============================================================================

/*
Usage examples:

if (A2lOnce()) {
    // Not Thread-safe !
    // This block executes exactly once globally across all threads
}

if (A2lThreadOnce()) {
    // This block executes exactly once per thread
    // Note: despite taking an internal mutex, the guard is a temporary whose destructor (and so the mutex unlock)
    // runs at the end of the if-condition, before this block executes - so the mutex does not serialize the block's
    // body across threads, only the brief once-check itself. Callers still needing that must synchronize themselves,
    // e.g. with A2lLock()/A2lUnlock().
}
*/

#include <a2l.h>

#ifdef __cplusplus

#include <mutex>
#include <thread>

namespace xcp {
namespace a2l {

struct empty_type {};

// RAII guard for execute-once pattern with optional mutex protection
// Location parameter ensures each call site gets its own once_flag
// Static members belong to the type, so each location is a different type which gets its own once_flag
template <bool WithMutex = false, bool PerThread = false, int Location = 0> class A2lOnceGuard {
  private:
    inline static std::once_flag once_flag_;
    inline static std::conditional_t<WithMutex, std::mutex, empty_type> mutex_;
    std::conditional_t<WithMutex, std::unique_lock<std::mutex>, empty_type> lock_;

  public:
    A2lOnceGuard() {
        if constexpr (WithMutex) {
            lock_ = std::unique_lock<std::mutex>(mutex_);
        }
    }

    explicit operator bool() const {
        bool execute = false;
        std::call_once(once_flag_, [&execute]() { execute = true; });
        return execute;
    }
};

// Specialization for per-thread execution
template <bool WithMutex, int Location> class A2lOnceGuard<WithMutex, true, Location> {
  private:
    inline static std::conditional_t<WithMutex, std::mutex, empty_type> mutex_;
    std::conditional_t<WithMutex, std::unique_lock<std::mutex>, empty_type> lock_;

  public:
    A2lOnceGuard() {
        if constexpr (WithMutex) {
            lock_ = std::unique_lock<std::mutex>(mutex_);
        }
    }

    explicit operator bool() const {
        static thread_local bool executed = false;
        if (!executed) {
            executed = true;
            return true;
        }
        return false;
    }
};

// Enhanced convenience macros for C++ RAII-style once execution
// Equivalent to the C macros A2lOnce(name)/A2lThreadOnce(name) in a2l.h, but take no name argument: the call site's
// __LINE__ makes each A2lOnceGuard instantiation a distinct type with its own static flag, so uniqueness is automatic.

/// Execute the following block exactly once, globally across all threads.
/// Not thread-safe on its own: a concurrent second caller's std::call_once check is itself thread-safe, so exactly
/// one caller enters the block, but nothing serializes the block's body against other, unrelated A2L registration
/// activity on other threads - use A2lLock()/A2lUnlock() inside the block if that is needed.
#define A2lOnce()                                                                                                                                                                  \
    xcp::a2l::A2lOnceGuard<false, false, __LINE__> {}
/// Execute the following block exactly once per calling thread (unlike A2lOnce, does not block other threads and each
/// thread runs the block independently on its first call). See the usage-example note above this section regarding
/// the guard's internal mutex - it does not serialize the block's body.
#define A2lThreadOnce()                                                                                                                                                            \
    xcp::a2l::A2lOnceGuard<true, true, __LINE__> {}

// =============================================================================
// Variadic template to create typedefs
// Once execution and thread safety is handled inside the macro
// Usage example:
//   A2lCreateTypedef(ParametersT, "Typedef for ParametersT",
//                    A2L_PARAMETER_COMPONENT(min, "Minimum value", "miles/h", -0.0, 10.0),
//                    A2L_PARAMETER_COMPONENT(max, "Maximum value", "miles/h", -70.0, 80.0)
// );
// =============================================================================

// Helper lambda-based macros used with A2lCreateTypedef
//
// Each expands to a lambda that A2lCreateTypedef calls with a `TypeName*` to derive the field's offset and type via
// offsetof/decltype - so these may only be used as arguments to A2lCreateTypedef, not evaluated standalone.
// @param field_name member of the typedef'd struct/class this component describes; its address is never taken directly,
// only offsetof(StructType, field_name), so it is safe to use on members that are not yet initialized
// @param comment free-text description
// @param unit physical unit string, or a "conv.<name>" reference to a conversion (see A2lCreateLinearConversion/A2lCreateEnumConversion in a2l.h)
// @param min_value, max_value physical value limits shown to the XCP tool

/// Scalar calibration parameter component
#define A2L_PARAMETER_COMPONENT(field_name, comment, unit, min_value, max_value)                                                                                                   \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        return xcp::a2l::A2lParameterComponentInfo<FieldType>(#field_name, (uint16_t)offsetof(StructType, field_name), 1, 1, comment, unit, min_value, max_value, NULL, NULL);     \
    }

// Multi dimensional parameters (curve, map, axis), auto-detect array dimensions (automatic size detection from type)
// Note: when y_dim is set to 0, it is used to identify axis

/// 1-dimensional calibration curve component (field_name must be a 1D array member); x_dim is auto-detected from the
/// field's array extent. No shared axis - the XCP tool creates a default one.
#define A2L_CURVE_COMPONENT(field_name, comment, unit, min_value, max_value)                                                                                                       \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        using ElementType = std::remove_reference_t<decltype(std::declval<StructType>().field_name[0])>;                                                                           \
        constexpr size_t x_dim = std::extent_v<FieldType, 0>;                                                                                                                      \
        return xcp::a2l::A2lParameterComponentInfo<ElementType>(#field_name, (uint16_t)offsetof(StructType, field_name), x_dim, 1, comment, unit, min_value, max_value, NULL,      \
                                                                NULL);                                                                                                             \
    }
/// Like A2L_CURVE_COMPONENT, but shares its axis with an A2L_AXIS_COMPONENT of the same typedef.
/// @param axis identifier of the sibling A2L_AXIS_COMPONENT field to share (stringified, not evaluated)
#define A2L_CURVE_WITH_AXIS_COMPONENT(field_name, comment, unit, min_value, max_value, axis)                                                                                       \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        using ElementType = std::remove_reference_t<decltype(std::declval<StructType>().field_name[0])>;                                                                           \
        constexpr size_t x_dim = std::extent_v<FieldType, 0>;                                                                                                                      \
        return xcp::a2l::A2lParameterComponentInfo<ElementType>(#field_name, (uint16_t)offsetof(StructType, field_name), x_dim, 1, comment, unit, min_value, max_value, #axis,     \
                                                                NULL);                                                                                                             \
    }
/// 2-dimensional calibration map component (field_name must be a 2D array member); x_dim/y_dim are auto-detected. No
/// shared axes - the XCP tool creates default ones.
#define A2L_MAP_COMPONENT(field_name, comment, unit, min_value, max_value)                                                                                                         \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        using ElementType = std::remove_reference_t<decltype(std::declval<StructType>().field_name[0][0])>;                                                                        \
        constexpr size_t x_dim = std::extent_v<FieldType, 0>;                                                                                                                      \
        constexpr size_t y_dim = std::extent_v<FieldType, 1>;                                                                                                                      \
        return xcp::a2l::A2lParameterComponentInfo<ElementType>(#field_name, (uint16_t)offsetof(StructType, field_name), x_dim, y_dim, comment, unit, min_value, max_value, NULL,  \
                                                                NULL);                                                                                                             \
    }
/// Like A2L_MAP_COMPONENT, but shares its axes with two A2L_AXIS_COMPONENT fields of the same typedef.
/// @param x_axis, y_axis identifiers of the sibling A2L_AXIS_COMPONENT fields to share (stringified, not evaluated)
#define A2L_MAP_WITH_AXIS_COMPONENT(field_name, comment, unit, min_value, max_value, x_axis, y_axis)                                                                               \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        using ElementType = std::remove_reference_t<decltype(std::declval<StructType>().field_name[0][0])>;                                                                        \
        constexpr size_t x_dim = std::extent_v<FieldType, 0>;                                                                                                                      \
        constexpr size_t y_dim = std::extent_v<FieldType, 1>;                                                                                                                      \
        return xcp::a2l::A2lParameterComponentInfo<ElementType>(#field_name, (uint16_t)offsetof(StructType, field_name), x_dim, y_dim, comment, unit, min_value, max_value,        \
                                                                #x_axis, #y_axis);                                                                                                 \
    }
/// Standalone axis component (field_name must be a 1D array member), usable as the shared axis of one or more
/// A2L_CURVE_WITH_AXIS_COMPONENT/A2L_MAP_WITH_AXIS_COMPONENT fields in the same typedef. Internally a parameter
/// component with y_dim forced to 0, which is what marks it as an axis rather than a curve.
#define A2L_AXIS_COMPONENT(field_name, comment, unit, min_value, max_value)                                                                                                        \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        using ElementType = std::remove_reference_t<decltype(std::declval<StructType>().field_name[0])>;                                                                           \
        constexpr size_t x_dim = std::extent_v<FieldType, 0>;                                                                                                                      \
        return xcp::a2l::A2lParameterComponentInfo<ElementType>(#field_name, (uint16_t)offsetof(StructType, field_name), x_dim, 0, comment, unit, min_value, max_value, NULL,      \
                                                                NULL);                                                                                                             \
    }

/// Scalar measurement component.
/// @param field_name member of the typedef'd struct/class this component describes
/// @param comment free-text description
/// @param unit physical unit string, or a "conv.<name>" reference to a conversion (see A2L_PARAMETER_COMPONENT above)
#define A2L_MEASUREMENT_COMPONENT(field_name, comment, unit)                                                                                                                       \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        return xcp::a2l::A2lMeasurementComponentInfo<FieldType>(#field_name, (uint16_t)offsetof(StructType, field_name), 1, comment, unit);                                        \
    }

/// 1-dimensional array measurement component (field_name must be a 1D array member); dim is auto-detected.
/// @param comment, unit as in A2L_MEASUREMENT_COMPONENT above
#define A2L_MEASUREMENT_ARRAY_COMPONENT(field_name, comment, unit)                                                                                                                 \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        using ElementType = std::remove_reference_t<decltype(std::declval<StructType>().field_name[0])>;                                                                           \
        constexpr size_t x_dim = std::extent_v<FieldType, 0>;                                                                                                                      \
        return xcp::a2l::A2lMeasurementComponentInfo<ElementType>(#field_name, (uint16_t)offsetof(StructType, field_name), x_dim, comment, unit);                                  \
    }

/// Nested-typedef component: field_name's own type was itself registered as an A2L typedef via a separate
/// A2lCreateTypedef call - use this instead of A2L_MEASUREMENT_COMPONENT for struct/class members, since the field's
/// C++ type does not map to a single scalar A2L type id.
/// @param field_name member of the enclosing typedef'd struct/class
/// @param type_name name of field_name's own type, as registered with A2lCreateTypedef(type_name, ...) - not auto-detected
/// @param dim 1 for a scalar member, or the array length for an array of that typedef - not auto-detected
#define A2L_TYPEDEF_COMPONENT(field_name, type_name, dim)                                                                                                                          \
    [](auto type_ptr) {                                                                                                                                                            \
        using StructType = std::remove_pointer_t<decltype(type_ptr)>;                                                                                                              \
        using FieldType = decltype(StructType::field_name);                                                                                                                        \
        return xcp::a2l::A2lTypedefComponentInfo<FieldType>(#field_name, type_name, (uint16_t)(dim), (uint16_t)offsetof(StructType, field_name));                                  \
    }

// Helper struct to hold typedef component information for parameter components
template <typename FieldType> struct A2lParameterComponentInfo {
    const char *name;
    const tA2lTypeId type_id;
    const size_t offset;
    const uint16_t x_dim;
    const uint16_t y_dim;
    const char *comment;
    const char *unit;
    double min_value;
    double max_value;
    const char *x_axis;
    const char *y_axis;

    // Constructor for 2 dimensional array parameter component with physical unit and limit (name, x_dim, y_dim, comment, unit, min, max)
    constexpr A2lParameterComponentInfo(const char *name, size_t offset, uint16_t x_dim, uint16_t y_dim, const char *comment, const char *unit, double min_value, double max_value,
                                        const char *x_axis, const char *y_axis)
        : name(name), type_id(GetTypeId<FieldType>()), offset(offset), x_dim(x_dim), y_dim(y_dim), comment(comment), unit(unit), min_value(min_value), max_value(max_value),
          x_axis(x_axis), y_axis(y_axis) {}
};

// Helper struct to hold typedef component information for measurement components
template <typename FieldType> struct A2lMeasurementComponentInfo {
    const char *name;
    const tA2lTypeId type_id;
    const size_t offset;
    const uint16_t dim;
    const char *comment;
    const char *unit;

    // Constructor for array of physical component (name, dim, comment, unit, min, max)
    constexpr A2lMeasurementComponentInfo(const char *name, size_t offset, uint16_t dim, const char *comment, const char *unit)
        : name(name), type_id(GetTypeId<FieldType>()), offset(offset), dim(dim), comment(comment), unit(unit) {}
};

// Helper struct to hold typedef component information for typedef components
template <typename FieldType> struct A2lTypedefComponentInfo {
    const char *name;
    const char *type_name;
    const uint16_t dim;
    const size_t offset;

    // Constructor for array of typedef component
    constexpr A2lTypedefComponentInfo(const char *name, const char *type_name, uint16_t dim, size_t offset) : name(name), type_name(type_name), dim(dim), offset(offset) {}
};

// =============================================================================

// Helper template function to register a parameter component
template <typename T> void A2lCreateTypedefComponentTemplate(const A2lParameterComponentInfo<T> &info) {
    A2lTypedefParameterComponent_(info.name, info.type_id, info.x_dim, info.y_dim, info.offset, info.comment, info.unit, info.min_value, info.max_value, info.x_axis, info.y_axis);
}
// Helper template function to register a measurement component
template <typename T> void A2lCreateTypedefComponentTemplate(const A2lMeasurementComponentInfo<T> &info) {
    A2lTypedefMeasurementComponent_(info.name, info.type_id, info.dim, info.offset, info.comment, info.unit, 0.0, 0.0);
}
// Helper template function to register a typedef component
template <typename T> void A2lCreateTypedefComponentTemplate(const A2lTypedefComponentInfo<T> &info) { A2lTypedefComponent_(info.name, info.type_name, info.dim, info.offset); }

/// Create an A2L typedef for struct/class type_name and its fields, from a list of A2L_*_COMPONENT builders.
/// Executes at most once per type_name (thread-safe, via std::call_once and A2lLock()/A2lUnlock() - unlike
/// A2lOnce()/A2lThreadOnce() above, this is real mutex protection held for the whole registration, not just a check).
/// Does nothing if XCP is not activated.
/// @param type_name struct/class type this typedef describes; used both as the A2L type name (stringified) and to
/// instantiate each component builder lambda with a `type_name*` for offsetof/decltype
/// @param comment free-text description
/// @param ... one or more A2L_PARAMETER_COMPONENT/A2L_CURVE_COMPONENT/A2L_MAP_COMPONENT/A2L_AXIS_COMPONENT/
/// A2L_MEASUREMENT_COMPONENT/A2L_MEASUREMENT_ARRAY_COMPONENT/A2L_TYPEDEF_COMPONENT builders, one per field
#define A2lCreateTypedef(type_name, comment, ...) xcp::a2l::A2lCreateTypedefTemplate<type_name>(#type_name, sizeof(type_name), comment, __VA_ARGS__);

// Template function for typedef creation
template <typename TypeName, typename... ComponentBuilders>
void A2lCreateTypedefTemplate(const char *type_name, size_t type_size, const char *comment, ComponentBuilders &&...builders) {
    if (XcpIsActivated()) {
        static std::once_flag once_flag;
        std::call_once(once_flag, [&]() {
            A2lLock();
            A2lTypedefBegin_(type_name, (uint32_t)type_size, comment);
            (A2lCreateTypedefComponentTemplate(builders((TypeName *)nullptr)), ...);
            A2lTypedefEnd_();
            A2lUnlock();
        });
    }
}

} // namespace a2l
} // namespace xcp

#endif // __cplusplus
