// C++ type test fixture for the xcpclient unit tests in src/elf_reader/mod.rs (mod test).
// Covers: plain struct, private members, class, struct/class template instantiations, all struct/class
// inheritance combinations, static constexpr member, struct with class and template members, 64-bit enum.
//
// cpp_types.elf is built from this file with GCC 12.3.1 (xPack arm-none-eabi), DWARF 5, no libraries:
//   arm-none-eabi-g++ -g -gdwarf-5 -O0 -fdebug-prefix-map=$(pwd)=. -nostdlib -nostartfiles -Wl,-e,main \
//       -Wl,--unresolved-symbols=ignore-all -o cpp_types.elf cpp_types.cpp
//
#include <stdint.h>

// 1. plain struct, public members
struct PlainStruct { uint32_t a; float b; };
PlainStruct g_plain = {1, 2.0f};

// 2. struct with private members
struct PrivStruct { public: uint32_t pub_m; private: uint32_t priv_m; };
PrivStruct g_priv;

// 3. class, public members
class PubClass { public: uint32_t x; float y; };
PubClass g_pubclass = {3, 4.0f};

// 4. struct template instantiations
template <typename T> struct TplStruct { T value; uint32_t count; };
TplStruct<uint16_t> g_tpl_struct = {5, 6};
TplStruct<float>    g_tpl_struct_f = {7.0f, 8};

// 5. class template instantiation
template <typename T> class TplClass { public: T value; uint32_t count; };
TplClass<uint32_t> g_tpl_class = {9, 10};

// 6. struct derived from struct
struct BaseS { uint32_t base_a; };
struct DerivedSS : BaseS { uint32_t derived_b; };
DerivedSS g_derived_ss;

// 7. class derived from class
class BaseC { public: uint32_t cbase_a; };
class DerivedCC : public BaseC { public: uint32_t cderived_b; };
DerivedCC g_derived_cc;

// 8. mixed: class derived from struct, struct derived from class
class DerivedCS : public BaseS { public: uint32_t cs_b; };
DerivedCS g_derived_cs;
struct DerivedSC : BaseC { uint32_t sc_b; };
DerivedSC g_derived_sc;

// 9. static constexpr member (no storage)
struct WithStatic { static constexpr uint32_t K = 42; uint32_t v; };
WithStatic g_with_static;

// 10. struct containing a class member and a template member
struct Outer { PubClass inner_class; TplStruct<uint16_t> inner_tpl; uint32_t z; };
Outer g_outer;

// 11. 64-bit enum with a negative value
enum class BigEnum : int64_t { NEG = -1, ZERO = 0, POS = 1 };
BigEnum g_bigenum = BigEnum::NEG;

volatile uint32_t g_sink;
int main() {
    g_sink = g_plain.a + g_priv.pub_m + g_pubclass.x + g_tpl_struct.count + g_tpl_struct_f.count + g_tpl_class.count
           + g_derived_ss.derived_b + g_derived_cc.cderived_b + g_derived_cs.cs_b + g_derived_sc.sc_b
           + g_with_static.v + g_outer.z + (uint32_t)g_bigenum;
    return 0;
}
