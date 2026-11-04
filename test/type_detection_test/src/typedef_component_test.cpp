// test/type_detection_test/src/typedef_component_test.cpp
// Checks the offsets, element types and dimensions the A2L_*_COMPONENT builders of a2l.hpp derive for struct members.
// The builders are evaluated directly with a null type pointer, as A2lCreateTypedef does, no XCP server or A2L file needed.

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <type_traits>

#include "a2l.hpp" // for A2L_*_COMPONENT

namespace {

// Members beyond 64 KiB, the offsets used to be truncated to 16 bit
struct BigStruct {
    uint8_t pad[70000];
    double scalar;
    float curve[8];
    uint16_t map[4][3];
};

// std::array members, the dimensions used to be detected as 0
struct StdArrayStruct {
    uint32_t counter;
    std::array<float, 8> curve;
    std::array<std::array<int16_t, 3>, 4> map;
    const std::array<uint8_t, 5> axis{};
};

struct Point {
    float x, y, z;
};

struct Cloud {
    uint16_t count;
    Point origin;
    std::array<Point, 16> points;
    Point grid[2][3];
};

int failures = 0;

void check(bool condition, const char *what) {
    if (!condition) {
        printf("  FAILED: %s\n", what);
        failures++;
    }
}

template <typename Builder> auto build(Builder builder) { return builder((Cloud *)nullptr); }

// -----------------------------------------------------------------------------
// Non-standard-layout types, not available with OPTION_A2L_STANDARD_LAYOUT_ONLY, which turns them into compile errors
#ifndef OPTION_A2L_STANDARD_LAYOUT_ONLY

// C++17 [support.types.layout]: offsetof on a type which is not standard-layout is conditionally-supported. GCC and
// Clang support it and report -Winvalid-offsetof, MSVC supports it silently. The only case without a constant offset,
// a member of a virtual base class, is a hard compile error on all three compilers.
// Each type below is asserted to be non-standard-layout, and each offset the A2L_*_COMPONENT builders derive is
// compared with the actual member address in a real object, which is what the XCP tool accesses (instance address + offset).

// The offsetof uses on the types below trigger -Winvalid-offsetof on GCC and Clang, which is the subject of this test,
// the region ends after non_standard_layout_test()
#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Winvalid-offsetof"
#endif

// 1. Virtual functions: hidden vtable pointer in front of the members
class VirtualClass {
  public:
    virtual ~VirtualClass() = default;
    virtual void step() { counter++; }
    uint32_t counter = 0;
    double value = 0.0;
    float curve[4] = {};
};

// 2. Mixed access control of the data members
class MixedAccess {
  public:
    uint8_t first = 0;

  private:
    uint32_t hidden = 0;

  public:
    double last = 0.0;

    // Private members can only be registered from inside the class, as point_cloud_demo does
    static size_t hiddenOffsetFromBuilder() { return A2L_MEASUREMENT_COMPONENT(hidden, "", "")((MixedAccess *)nullptr).offset; }
    static size_t hiddenOffsetFromObject() {
        MixedAccess obj;
        return (size_t)((const uint8_t *)&obj.hidden - (const uint8_t *)&obj);
    }
};

// 3. Data members in both, base and derived class
struct Base {
    uint16_t base_value = 0;
    uint8_t base_array[3] = {};
};
struct Derived : Base {
    double derived_value = 0.0;
};

// 4. Multiple inheritance, the second base class subobject is not at offset 0
struct BaseA {
    uint64_t a = 0;
};
struct BaseB {
    uint8_t b = 0;
    float b_curve[5] = {};
};
struct MultiDerived : BaseA, BaseB {
    int16_t own = 0;
};

// 5. Polymorphic class with multiple inheritance, two vtable pointers
struct PolyA {
    virtual ~PolyA() = default;
    uint32_t pa = 0;
};
struct PolyB {
    virtual ~PolyB() = default;
    uint32_t pb = 0;
};
struct PolyDerived : PolyA, PolyB {
    double pd = 0.0;
};

// 6. Empty base class and first member of the same type, may not share an address
struct EmptyBase {};
struct EmptyBaseConflict : EmptyBase {
    EmptyBase first;
    int32_t value = 0;
};

// 7. Non-standard-layout member in an otherwise plain struct, nested typedef component
struct WithVirtualMember {
    uint8_t x = 0;
    VirtualClass inner;
    std::array<VirtualClass, 3> inner_array;
    uint16_t y = 0;
};

// Not testable, compile error: member of a virtual base class
//   struct VBase { int v; }; struct VDerived : virtual VBase { int d; };
//   A2L_MEASUREMENT_COMPONENT(v, "", "")((VDerived *)nullptr);  // error: invalid application of 'offsetof' to a field of a virtual base

static_assert(!std::is_standard_layout_v<VirtualClass>);
static_assert(!std::is_standard_layout_v<MixedAccess>);
static_assert(!std::is_standard_layout_v<Derived>);
static_assert(!std::is_standard_layout_v<MultiDerived>);
static_assert(!std::is_standard_layout_v<PolyDerived>);
static_assert(!std::is_standard_layout_v<EmptyBaseConflict>);
static_assert(!std::is_standard_layout_v<WithVirtualMember>);

// Offset of a member in a real object
template <typename T, typename M> size_t objectOffset(const T &obj, const M &member) { return (size_t)((const uint8_t *)&member - (const uint8_t *)&obj); }

void non_standard_layout_test() {

    printf("  non-standard-layout types:\n");

    // 1. Virtual functions
    {
        VirtualClass obj;
        auto c = A2L_MEASUREMENT_COMPONENT(counter, "", "")((VirtualClass *)nullptr);
        auto v = A2L_PARAMETER_COMPONENT(value, "", "", 0, 1)((VirtualClass *)nullptr);
        auto k = A2L_CURVE_COMPONENT(curve, "", "", 0, 1)((VirtualClass *)nullptr);
        check(c.offset == objectOffset(obj, obj.counter) && c.offset >= sizeof(void *), "VirtualClass.counter behind vtable pointer");
        check(v.offset == objectOffset(obj, obj.value), "VirtualClass.value");
        check(k.offset == objectOffset(obj, obj.curve) && k.x_dim == 4, "VirtualClass.curve");
        printf("    VirtualClass: counter=%zu value=%zu curve=%zu\n", c.offset, v.offset, k.offset);
    }

    // 2. Mixed access control
    {
        MixedAccess obj;
        auto f = A2L_MEASUREMENT_COMPONENT(first, "", "")((MixedAccess *)nullptr);
        auto l = A2L_MEASUREMENT_COMPONENT(last, "", "")((MixedAccess *)nullptr);
        check(f.offset == objectOffset(obj, obj.first), "MixedAccess.first");
        check(MixedAccess::hiddenOffsetFromBuilder() == MixedAccess::hiddenOffsetFromObject(), "MixedAccess.hidden (private)");
        check(l.offset == objectOffset(obj, obj.last), "MixedAccess.last");
        printf("    MixedAccess: first=%zu hidden=%zu last=%zu\n", f.offset, MixedAccess::hiddenOffsetFromBuilder(), l.offset);
    }

    // 3. Data members in base and derived class, registered for the derived type
    {
        Derived obj;
        auto b = A2L_MEASUREMENT_COMPONENT(base_value, "", "")((Derived *)nullptr);
        auto a = A2L_MEASUREMENT_ARRAY_COMPONENT(base_array, "", "")((Derived *)nullptr);
        auto d = A2L_MEASUREMENT_COMPONENT(derived_value, "", "")((Derived *)nullptr);
        check(b.offset == objectOffset(obj, obj.base_value), "Derived.base_value");
        check(a.offset == objectOffset(obj, obj.base_array) && a.dim == 3, "Derived.base_array");
        check(d.offset == objectOffset(obj, obj.derived_value), "Derived.derived_value");
        printf("    Derived: base_value=%zu base_array=%zu derived_value=%zu\n", b.offset, a.offset, d.offset);
    }

    // 4. Multiple inheritance
    {
        MultiDerived obj;
        auto a = A2L_MEASUREMENT_COMPONENT(a, "", "")((MultiDerived *)nullptr);
        auto b = A2L_MEASUREMENT_COMPONENT(b, "", "")((MultiDerived *)nullptr);
        auto c = A2L_CURVE_COMPONENT(b_curve, "", "", 0, 1)((MultiDerived *)nullptr);
        auto o = A2L_MEASUREMENT_COMPONENT(own, "", "")((MultiDerived *)nullptr);
        check(a.offset == objectOffset(obj, obj.a), "MultiDerived.a");
        check(b.offset == objectOffset(obj, obj.b) && b.offset > 0, "MultiDerived.b in second base");
        check(c.offset == objectOffset(obj, obj.b_curve) && c.x_dim == 5, "MultiDerived.b_curve");
        check(o.offset == objectOffset(obj, obj.own), "MultiDerived.own");
        printf("    MultiDerived: a=%zu b=%zu b_curve=%zu own=%zu\n", a.offset, b.offset, c.offset, o.offset);
    }

    // 5. Polymorphic multiple inheritance
    {
        PolyDerived obj;
        auto a = A2L_MEASUREMENT_COMPONENT(pa, "", "")((PolyDerived *)nullptr);
        auto b = A2L_MEASUREMENT_COMPONENT(pb, "", "")((PolyDerived *)nullptr);
        auto d = A2L_MEASUREMENT_COMPONENT(pd, "", "")((PolyDerived *)nullptr);
        check(a.offset == objectOffset(obj, obj.pa), "PolyDerived.pa");
        check(b.offset == objectOffset(obj, obj.pb), "PolyDerived.pb");
        check(d.offset == objectOffset(obj, obj.pd), "PolyDerived.pd");
        printf("    PolyDerived: pa=%zu pb=%zu pd=%zu\n", a.offset, b.offset, d.offset);
    }

    // 6. Empty base class conflict
    {
        EmptyBaseConflict obj;
        auto v = A2L_MEASUREMENT_COMPONENT(value, "", "")((EmptyBaseConflict *)nullptr);
        check(v.offset == objectOffset(obj, obj.value), "EmptyBaseConflict.value");
        printf("    EmptyBaseConflict: value=%zu\n", v.offset);
    }

    // 7. Non-standard-layout members
    {
        WithVirtualMember obj;
        auto i = A2L_TYPEDEF_COMPONENT(inner, "VirtualClass", 1)((WithVirtualMember *)nullptr);
        auto a = A2L_TYPEDEF_COMPONENT(inner_array, "VirtualClass", 3)((WithVirtualMember *)nullptr);
        auto y = A2L_MEASUREMENT_COMPONENT(y, "", "")((WithVirtualMember *)nullptr);
        check(i.offset == objectOffset(obj, obj.inner), "WithVirtualMember.inner");
        check(a.offset == objectOffset(obj, obj.inner_array) && a.dim == 3, "WithVirtualMember.inner_array");
        check(objectOffset(obj, obj.inner_array[2]) - a.offset == 2 * sizeof(VirtualClass), "WithVirtualMember.inner_array element stride");
        check(y.offset == objectOffset(obj, obj.y), "WithVirtualMember.y");
        printf("    WithVirtualMember: inner=%zu inner_array=%zu y=%zu\n", i.offset, a.offset, y.offset);
    }
}

#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic pop
#endif

#endif // OPTION_A2L_STANDARD_LAYOUT_ONLY

} // namespace

int typedef_component_test() {

    printf("\nA2L typedef component test:\n");

    // Offsets beyond 64 KiB
    {
        auto p = A2L_PARAMETER_COMPONENT(scalar, "", "", 0, 1)((BigStruct *)nullptr);
        check(p.offset == offsetof(BigStruct, scalar) && p.offset > 0xFFFF, "BigStruct.scalar offset");
        check(p.type_id == A2L_TYPE_DOUBLE, "BigStruct.scalar type");
        auto c = A2L_CURVE_COMPONENT(curve, "", "", 0, 1)((BigStruct *)nullptr);
        check(c.offset == offsetof(BigStruct, curve) && c.x_dim == 8 && c.y_dim == 1 && c.type_id == A2L_TYPE_FLOAT, "BigStruct.curve");
        auto m = A2L_MAP_COMPONENT(map, "", "", 0, 1)((BigStruct *)nullptr);
        check(m.offset == offsetof(BigStruct, map) && m.x_dim == 4 && m.y_dim == 3 && m.type_id == A2L_TYPE_UINT16, "BigStruct.map");
        auto a = A2L_MEASUREMENT_ARRAY_COMPONENT(curve, "", "")((BigStruct *)nullptr);
        check(a.offset == offsetof(BigStruct, curve) && a.dim == 8, "BigStruct.curve measurement");
        auto s = A2L_MEASUREMENT_COMPONENT(scalar, "", "")((BigStruct *)nullptr);
        check(s.offset == offsetof(BigStruct, scalar) && s.dim == 1, "BigStruct.scalar measurement");
    }

    // std::array members
    {
        auto c = A2L_CURVE_COMPONENT(curve, "", "", 0, 1)((StdArrayStruct *)nullptr);
        check(c.offset == offsetof(StdArrayStruct, curve) && c.x_dim == 8 && c.y_dim == 1 && c.type_id == A2L_TYPE_FLOAT, "std::array curve");
        auto m = A2L_MAP_COMPONENT(map, "", "", 0, 1)((StdArrayStruct *)nullptr);
        check(m.offset == offsetof(StdArrayStruct, map) && m.x_dim == 4 && m.y_dim == 3 && m.type_id == A2L_TYPE_INT16, "std::array map");
        auto x = A2L_AXIS_COMPONENT(axis, "", "", 0, 1)((StdArrayStruct *)nullptr);
        check(x.offset == offsetof(StdArrayStruct, axis) && x.x_dim == 5 && x.y_dim == 0 && x.type_id == A2L_TYPE_UINT8, "const std::array axis");
        auto a = A2L_MEASUREMENT_ARRAY_COMPONENT(curve, "", "")((StdArrayStruct *)nullptr);
        check(a.dim == 8 && a.type_id == A2L_TYPE_FLOAT, "std::array measurement");
    }

    // Nested typedef components, dim checked at compile time against the member
    {
        auto o = build(A2L_TYPEDEF_COMPONENT(origin, "Point", 1));
        check(o.offset == offsetof(Cloud, origin) && o.dim == 1, "Cloud.origin");
        auto p = build(A2L_TYPEDEF_COMPONENT(points, "Point", 16));
        check(p.offset == offsetof(Cloud, points) && p.dim == 16, "Cloud.points (std::array)");
        auto g = build(A2L_TYPEDEF_COMPONENT(grid, "Point", 6));
        check(g.offset == offsetof(Cloud, grid) && g.dim == 6, "Cloud.grid (2D C array)");
    }

#ifndef OPTION_A2L_STANDARD_LAYOUT_ONLY
    non_standard_layout_test();
#else
    printf("  non-standard-layout types: skipped, OPTION_A2L_STANDARD_LAYOUT_ONLY\n");
#endif

    // Complete typedef registration, instantiates A2lCreateTypedefTemplate including the layout check (no-op at runtime
    // without XcpInit)
    A2lCreateTypedef(Point, "Point", A2L_MEASUREMENT_COMPONENT(x, "", ""), A2L_MEASUREMENT_COMPONENT(y, "", ""), A2L_MEASUREMENT_COMPONENT(z, "", ""));
    A2lCreateTypedef(Cloud, "Cloud", A2L_MEASUREMENT_COMPONENT(count, "", ""), A2L_TYPEDEF_COMPONENT(origin, "Point", 1), A2L_TYPEDEF_COMPONENT(points, "Point", 16),
                     A2L_TYPEDEF_COMPONENT(grid, "Point", 6));

    // These must not compile (verified manually, uncomment one at a time):
    // A2lCreateTypedef(VirtualClass, "", A2L_MEASUREMENT_COMPONENT(counter, "", ""));               // with OPTION_A2L_STANDARD_LAYOUT_ONLY
    // struct R { int &r; };            A2L_MEASUREMENT_COMPONENT(r, "", "")((R *)nullptr);       // reference member
    // struct P { Point *p; };          A2L_TYPEDEF_COMPONENT(p, "Point", 1)((P *)nullptr);       // pointer member
    // A2L_CURVE_COMPONENT(map, "", "", 0, 1)((BigStruct *)nullptr);                              // 2D array as curve
    // A2L_PARAMETER_COMPONENT(curve, "", "", 0, 1)((BigStruct *)nullptr);                        // array as scalar
    // A2L_CURVE_COMPONENT(scalar, "", "", 0, 1)((BigStruct *)nullptr);                           // scalar as curve
    // build(A2L_TYPEDEF_COMPONENT(points, "Point", 15));                                         // wrong dim

    printf("  %s (%d failures)\n", failures == 0 ? "passed" : "FAILED", failures);
    return failures;
}
