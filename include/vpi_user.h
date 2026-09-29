#ifndef VPI_USER_H
#define VPI_USER_H

/* VPI (Verilog Procedural Interface) — IEEE 1800-2017 Annex K.
 *
 * Every constant below is the value the standard assigns it, so a C
 * file compiled against a vendor `vpi_user.h` and linked to xezim
 * agrees with xezim about what `vpiIntVal` or `cbValueChange` means.
 * An earlier version of this header invented its own numbering, which
 * meant any such file silently took the wrong branch.
 *
 * xezim implements the subset declared at the bottom of this file.
 * Functions the standard defines but xezim does not implement are NOT
 * declared here: a call to one is a compile error, which is the loud
 * failure we want, rather than a link-time surprise or a stub that
 * silently returns nothing. Every routine of IEEE 1800-2017 clause 38 is
 * declared; vpi_get_data / vpi_put_data are defined but always fail,
 * because xezim has no $save / $restart (see their declarations).
 *
 * A VPI module is loaded with `--vpi-lib <so>` (or `-m`), after which its
 * `vlog_startup_routines` run once, before simulation. VPI is also callable
 * from a DPI shared object loaded with `--dpi-lib`.
 */

#include <stdint.h>
#include <stddef.h>
#include <stdarg.h>

/* PLI type definitions (IEEE 1800-2017 Annex K.1). */
typedef int32_t  PLI_INT32;
typedef uint32_t PLI_UINT32;
typedef int64_t  PLI_INT64;
typedef uint64_t PLI_UINT64;
typedef char     PLI_BYTE8;
typedef short    PLI_INT16;
typedef unsigned short PLI_UINT16;

/* vpiHandle — opaque handle to a simulation object. */
typedef PLI_UINT32 *vpiHandle;

/* --- vpi_get(vpiType, ...) object types (Annex K, "Object types") ------
 * Only the codes xezim can actually return are listed. `vpiLogicVar` is
 * an alias of `vpiReg`, exactly as in the standard header. */
#define vpiIntegerVar         25   /* integer variable */
#define vpiIterator           27   /* iterator (vpi_iterate result) */
#define vpiMemory             29   /* unpacked array */
#define vpiMemoryWord         30   /* one word of an unpacked array */
#define vpiModule             32
#define vpiNet                36   /* scalar or vector net */
#define vpiNetBit             37
#define vpiParameter          41
#define vpiPartSelect         42   /* part-select / packed-struct member */
#define vpiPort               44   /* module port */
#define vpiPortBit            45   /* bit of vector module port */
#define vpiRealVar            47   /* real variable */
#define vpiReg                48   /* scalar or vector reg (4-state) */
#define vpiRegBit             49
#define vpiTimeVar            63

#define vpiConstant            7   /* a literal / computed argument value */
#define vpiSysFuncCall        56
#define vpiSysTaskCall        57

/* Traversal relations. */
#define vpiScope              84   /* containing scope */
#define vpiSysTfCall          85   /* the $systf call now executing */
#define vpiArgument           89   /* argument of a $systf call */
#define vpiInternalScope      92   /* internal scopes of a module */
#define vpiVariables         100   /* variables declared in a module */

/* vpi_get(vpiFuncType, sysTfCallHandle) -> the sysfunctype it was registered with. */
#define vpiFuncType           44
#define vpiSysFuncType        vpiFuncType
/* SystemVerilog object types (IEEE 1800-2017 sv_vpi_user.h). */
#define vpiLongIntVar        610
#define vpiShortIntVar       611
#define vpiIntVar            612
#define vpiShortRealVar      613
#define vpiByteVar           614
#define vpiStringVar         616
#define vpiEnumVar           617
#define vpiStructVar         618
#define vpiUnionVar          619
#define vpiBitVar            620   /* 2-state bit variable */
#define vpiLogicVar         vpiReg /* 4-state logic variable */

/* --- vpi_get_value / vpi_put_value format codes (Table 38-44) --------- */
#define vpiBinStrVal           1
#define vpiOctStrVal           2
#define vpiDecStrVal           3
#define vpiHexStrVal           4
#define vpiScalarVal           5
#define vpiIntVal              6
#define vpiRealVal             7
#define vpiStringVal           8
#define vpiVectorVal           9
#define vpiStrengthVal        10
#define vpiTimeVal            11
#define vpiObjTypeVal         12
#define vpiSuppressVal        13
#define vpiShortIntVal        14
#define vpiLongIntVal         15
#define vpiShortRealVal       16
#define vpiRawTwoStateVal     17
#define vpiRawFourStateVal    18

/* --- vpiScalarVal codes ----------------------------------------------- */
#define vpi0                   0
#define vpi1                   1
#define vpiZ                   2
#define vpiX                   3
#define vpiH                   4
#define vpiL                   5
#define vpiDontCare            6

/* --- vpi_put_value flags ---------------------------------------------- */
#define vpiNoDelay             1
#define vpiInertialDelay       2
#define vpiTransportDelay      3
#define vpiPureTransportDelay  4
#define vpiForceFlag           5
#define vpiReleaseFlag         6

/* --- vpi_get properties ----------------------------------------------- */
#define vpiUndefined         (-1)
#define vpiType                1
#define vpiName                2
#define vpiFullName            3
#define vpiSize                4
#define vpiDefName             9   /* module definition name */
#define vpiTimeUnit           11   /* module time unit; NULL: the simulation's */
#define vpiTimePrecision      12   /* module time precision; NULL: the simulation's */
#define vpiScalar             17
#define vpiVector             18
#define vpiDirection          20
#define vpiSigned             65

/* --- vpiDirection values --------------------------------------------- */
#define vpiInput               1
#define vpiOutput              2
#define vpiInout               3
#define vpiMixedIO             4
#define vpiNoDirection         5

/* --- vpi_time types --------------------------------------------------- */
#define vpiScaledRealTime      1
#define vpiSimTime             2
#define vpiSuppressTime        3

/* --- vpi_control operations ------------------------------------------- */
#define vpiStop               66   /* ends the run, like $stop */
#define vpiFinish             67   /* ends the run, like $finish */
#define vpiReset              68   /* NOT supported: xezim cannot rewind */

/* --- vpi_chk_error severity levels and states ------------------------- */
#define vpiNotice              1
#define vpiWarning             2
#define vpiError               3
#define vpiSystem              4
#define vpiInternal            5
#define vpiCompile             1
#define vpiPLI                 2
#define vpiRun                 3

/* --- callback reasons (Table 38-49) ----------------------------------- */
#define cbValueChange          1
#define cbReadWriteSynch       6
#define cbReadOnlySynch        7
#define cbNextSimTime          8
#define cbAfterDelay           9
#define cbStartOfSimulation   11
#define cbEndOfSimulation     12
#define cbStartOfReset        19
#define cbEndOfReset          20

/* s_vpi_vecval — 4-state vector element (IEEE 1800-2017 §38.10.1).
 * Layout-compatible with svLogicVecVal (§35.5.5), so UVM's HDL backdoor
 * can assign between the two without translation.
 *
 * Bit encoding, per element bit i:
 *     aval bval   value
 *       0    0      0
 *       1    0      1
 *       0    1      Z
 *       1    1      X
 */
typedef struct t_vpi_vecval {
    PLI_INT32 aval;
    PLI_INT32 bval;
} s_vpi_vecval, *p_vpi_vecval;

/* s_vpi_time — time value. */
typedef struct t_vpi_time {
    PLI_INT32 type;    /* vpiSimTime / vpiScaledRealTime / vpiSuppressTime */
    PLI_UINT32 high;
    PLI_UINT32 low;
    double real;
} s_vpi_time, *p_vpi_time;

/* s_vpi_strengthval — one bit of a vpiStrengthVal value (§38.15). */
typedef struct t_vpi_strengthval {
    PLI_INT32 logic;   /* vpi0 / vpi1 / vpiX / vpiZ */
    PLI_INT32 s0, s1;  /* strength codes below */
} s_vpi_strengthval, *p_vpi_strengthval;

/* Strength codes. */
#define vpiSupplyDrive      0x80
#define vpiStrongDrive      0x40
#define vpiPullDrive        0x20
#define vpiWeakDrive        0x08
#define vpiLargeCharge      0x10
#define vpiMediumCharge     0x04
#define vpiSmallCharge      0x02
#define vpiHiZ              0x01

/* s_vpi_value — value in one of the formats above.
 *
 * Formats and the union member that carries them:
 *   vpi*StrVal, vpiStringVal       str
 *   vpiScalarVal                   scalar
 *   vpiIntVal                      integer
 *   vpiRealVal                     real
 *   vpiTimeVal                     time
 *   vpiVectorVal                   vector
 *   vpiStrengthVal                 strength: one s_vpi_strengthval per bit,
 *                                  LSB first. A variable reads strong; a net
 *                                  reads the drive strength of the continuous
 *                                  assignment driving it (what %v shows), strong
 *                                  when none was given; z reads vpiHiZ. Put only
 *                                  to a scalar: the logic value is written, and
 *                                  the strengths are checked but not stored.
 * The s_vpi_value union has no member for the formats below, so xezim uses
 * the standard's implementation-specific `misc` field where one is needed:
 *   vpiShortIntVal                 integer (sign-extended 16-bit value)
 *   vpiShortRealVal                real (value rounded to single precision)
 *   vpiLongIntVal                  misc -> one PLI_INT64
 *   vpiRawTwoStateVal              misc -> ceil(size/8) aval bytes
 *   vpiRawFourStateVal             misc -> ceil(size/8) aval bytes, then as
 *                                  many bval bytes (the vpi_get_value_array
 *                                  layout of one element)
 */
typedef struct t_vpi_value {
    PLI_INT32 format;
    union {
        PLI_BYTE8                *str;
        PLI_INT32                 scalar;
        PLI_INT32                 integer;
        double                    real;
        struct t_vpi_time        *time;
        struct t_vpi_vecval      *vector;
        struct t_vpi_strengthval *strength;
        PLI_BYTE8                *misc;
    } value;
} s_vpi_value, *p_vpi_value;

/* s_vpi_vlog_info — tool identification, filled by vpi_get_vlog_info. */
typedef struct t_vpi_vlog_info {
    PLI_INT32   argc;
    PLI_BYTE8 **argv;
    PLI_BYTE8  *product;
    PLI_BYTE8  *version;
} s_vpi_vlog_info, *p_vpi_vlog_info;

/* s_cb_data — callback registration and dispatch (IEEE 1800-2017 §38.7). */
typedef struct t_cb_data s_cb_data, *p_cb_data;
struct t_cb_data {
    PLI_INT32    reason;
    PLI_INT32  (*cb_rtn)(p_cb_data cb_data_p);
    vpiHandle    obj;
    p_vpi_time   time;
    p_vpi_value  value;
    PLI_INT32    index;
    PLI_BYTE8   *user_data;
};

/* ---------------------------------------------------------------------
 * Implemented by xezim. Signatures match IEEE 1800-2017 Annex K exactly.
 * ------------------------------------------------------------------ */

/* Resolve a hierarchical name. `scope` is ignored (xezim resolves against
 * the flat signal table); pass NULL. Returns NULL if the name does not
 * name a signal. Tries the full name, then each successively shorter
 * suffix, so "top.dut.sig", "dut.sig" and "sig" all resolve. */
vpiHandle vpi_handle_by_name(PLI_BYTE8 *name, vpiHandle scope);

/* One-to-one traversal. Only vpiScope is modelled: the containing scope of
 * an object, or the parent of a module. As an xezim extension,
 * vpi_handle(vpiScope, NULL) returns the top module — the standard route is
 * vpi_scan(vpi_iterate(vpiModule, NULL)), but enough code spells it the
 * short way that supporting it is worth more than returning NULL. Any other
 * relation returns NULL. */
vpiHandle vpi_handle(PLI_INT32 type, vpiHandle refHandle);

/* One-to-many traversal. Returns NULL when the relation yields nothing.
 * Supported for a module reference: vpiModule and vpiInternalScope (child
 * instances), vpiNet, vpiReg, vpiVariables, vpiParameter, vpiMemory.
 * With a NULL reference, vpiModule yields the single top module. */
vpiHandle vpi_iterate(PLI_INT32 type, vpiHandle refHandle);

/* Hand out the next object. When the iterator is exhausted it returns NULL
 * and FREES the iterator (IEEE 1800-2017 section 38.32) — do not free it
 * yourself. */
vpiHandle vpi_scan(vpiHandle iterator);

/* Select one word of a vpiMemory object. NULL if out of range. */
vpiHandle vpi_handle_by_index(vpiHandle object, PLI_INT32 index);

/* vpiName, vpiFullName, and vpiDefName (modules only). Returns NULL for any
 * other property. The string is simulator-owned and valid until the next
 * vpi_get_str call on this thread. */
PLI_BYTE8 *vpi_get_str(PLI_INT32 property, vpiHandle object);

/* Formatted output, interleaved with $display. */
int vpi_printf(PLI_BYTE8 *format, ...);
int vpi_vprintf(PLI_BYTE8 *format, va_list ap);
int vpi_mcd_printf(PLI_UINT32 mcd, PLI_BYTE8 *format, ...);
int vpi_mcd_vprintf(PLI_UINT32 mcd, PLI_BYTE8 *format, va_list ap);

/* Multichannel descriptors, shared with $fopen: bit 0 is stdout, and each
 * vpi_mcd_open (or one-argument $fopen) takes a free bit for a file opened
 * for writing. vpi_mcd_open returns 0 when the file cannot be opened;
 * vpi_mcd_close and vpi_mcd_flush return 0; vpi_mcd_name returns the file
 * name ("stdout" for bit 0), or NULL for a channel that is not open. */
PLI_UINT32 vpi_mcd_open(PLI_BYTE8 *name);
PLI_UINT32 vpi_mcd_close(PLI_UINT32 mcd);
PLI_INT32 vpi_mcd_flush(PLI_UINT32 mcd);
PLI_BYTE8 *vpi_mcd_name(PLI_UINT32 cd);

/* Flushes stdout and every open file. Returns 0. */
PLI_INT32 vpi_flush(void);

/* Register a system task or function. `tfname` must begin with '$', and
 * `type` must be vpiSysTask or vpiSysFunc. `compiletf` runs once per call
 * instance (a call site in one module instance), immediately before that
 * instance's first `calltf` — xezim has no separate compile phase for it.
 *
 * A vpiSysFunc is dispatched when its `$name` appears in an expression. It
 * returns whatever it deposits with vpi_put_value on its own call handle
 * (vpi_handle(vpiSysTfCall, NULL)); a function that deposits nothing returns
 * 0. `sizetf` is called once per invocation for vpiSizedFunc/vpiSizedSignedFunc
 * to learn the return width; the other sysfunctypes size themselves. */
typedef struct t_vpi_systf_data {
    PLI_INT32   type;         /* vpiSysTask or vpiSysFunc */
    PLI_INT32   sysfunctype;  /* vpi[Int,Real,Time,Sized,SizedSigned]Func */
    PLI_BYTE8  *tfname;       /* first character must be '$' */
    PLI_INT32 (*calltf)(PLI_BYTE8 *);
    PLI_INT32 (*compiletf)(PLI_BYTE8 *);
    PLI_INT32 (*sizetf)(PLI_BYTE8 *);
    PLI_BYTE8  *user_data;
} s_vpi_systf_data, *p_vpi_systf_data;

#define vpiSysTask             1
#define vpiSysFunc             2
#define vpiIntFunc             1
#define vpiRealFunc            2
#define vpiTimeFunc            3
#define vpiSizedFunc           4
#define vpiSizedSignedFunc     5

/* Returns the registration's vpiUserSystf object (NULL on failure). */
vpiHandle vpi_register_systf(p_vpi_systf_data systf_data_p);

/* Fills *systf_data_p from a vpiUserSystf handle: the one vpi_register_systf
 * returned, vpi_handle(vpiUserSystf, callHandle), or a vpi_scan of
 * vpi_iterate(vpiUserSystf, NULL). `tfname` points at simulator-owned
 * storage valid for the whole run. */
#define vpiUserSystf          67
void vpi_get_systf_info(vpiHandle object, p_vpi_systf_data systf_data_p);

/* User data of a system task / function CALL INSTANCE: a call site in one
 * module instance, so the same site reached again (a loop, a later time
 * step) sees the same data, and another site or another instance of the
 * module has its own. `obj` is a call handle (vpi_handle(vpiSysTfCall,
 * NULL)). vpi_put_userdata returns 1 on success, 0 on failure;
 * vpi_get_userdata returns NULL when nothing was put, or on failure. */
PLI_INT32 vpi_put_userdata(vpiHandle obj, void *userdata);
void *vpi_get_userdata(vpiHandle obj);

/* vpi_get(vpiUserDefn, callHandle) is 1: every call handle is of a
 * user-defined system task or function. */
#define vpiUserDefn           45

/* s_vpi_error_info — filled by vpi_chk_error. `message` and `product` point at
 * simulator-owned storage valid until the next vpi_chk_error call. */
typedef struct t_vpi_error_info {
    PLI_INT32  state;    /* vpiCompile / vpiPLI / vpiRun */
    PLI_INT32  level;    /* vpiNotice / vpiWarning / vpiError / ... */
    PLI_BYTE8 *message;
    PLI_BYTE8 *product;
    PLI_BYTE8 *code;
    PLI_BYTE8 *file;
    PLI_INT32  line;
} s_vpi_error_info, *p_vpi_error_info;

/* Reports the last VPI diagnostic and CLEARS it, returning its severity level,
 * or 0 when nothing has failed since the previous call. `error_info_p` may be
 * NULL if you only want the level. This is the way to notice that a
 * vpi_get_value / vpi_put_value / vpi_register_* call did not do what you
 * asked — most of them cannot report failure any other way. */
PLI_INT32 vpi_chk_error(p_vpi_error_info error_info_p);

/* vpiStop and vpiFinish end the run, like $stop / $finish; both accept the
 * usual diagnostic-level argument, which xezim ignores. vpiReset is rejected.
 * Returns 1 on success, 0 on failure (see vpi_chk_error). */
PLI_INT32 vpi_control(PLI_INT32 operation, ...);

/* The entry point xezim calls for every `--vpi-lib` module: a NULL-terminated
 * array of registration routines (IEEE 1800-2017 section 38.2). Define it in
 * your VPI module; do not call it yourself. */
extern void (*vlog_startup_routines[])(void);

/* Returns vpiUndefined (-1) for a property xezim does not model.
 * Supported: vpiType, vpiSize, vpiSigned, vpiScalar, vpiVector, and
 * vpiTimeUnit / vpiTimePrecision (of a module, or of the simulation for NULL). */
PLI_INT32 vpi_get(PLI_INT32 property, vpiHandle object);
PLI_INT64 vpi_get64(PLI_INT32 property, vpiHandle object);

/* 1 when both handles refer to the same object, else 0. */
PLI_INT32 vpi_compare_objects(vpiHandle object1, vpiHandle object2);

/* On success, fills *value_p in the requested format. On failure — a bad
 * handle, or a format xezim cannot supply — sets value_p->format to
 * vpiSuppressVal and writes nothing else (IEEE 1800-2017 §38.15), which
 * is the ONLY way a caller can detect the failure. Always check it.
 *
 * Every format above is supported (see s_vpi_value for where each one's
 * value goes). vpiObjTypeVal picks vpiIntVal for an integer-typed object of
 * up to 32 bits, vpiRealVal for a real, vpiTimeVal for a time variable,
 * vpiStringVal for a string, vpiScalarVal for any other 1-bit object and
 * vpiVectorVal for any other vector. A real object read in any format but
 * vpiRealVal, vpiShortRealVal or vpiStringVal is first rounded to an
 * integer; vpiStringVal of a real is its decimal text (16 significant
 * digits). Octal and hex digits read x / z when all of their bits are,
 * X / Z when only some are.
 *
 * For the pointer-valued formats (vectors, strings, times, strengths and
 * the misc formats), the returned pointer addresses simulator-owned storage
 * that is valid only until the next vpi_get_value call on this thread.
 * Copy it out. */
void vpi_get_value(vpiHandle expr, p_vpi_value value_p);

/* Writes value_p to the object. flags selects vpiNoDelay (immediate),
 * vpiForceFlag or vpiReleaseFlag; the delay flags behave as vpiNoDelay
 * because xezim has no VPI event scheduling. Returns NULL. Every format
 * but vpiObjTypeVal and vpiSuppressVal is accepted (vpiStrengthVal only for
 * a scalar object); an integral value put to a real object converts to
 * real, a real put to an integral object is rounded. A format xezim cannot
 * decode writes nothing and is reported through vpi_chk_error. */
vpiHandle vpi_put_value(vpiHandle object, p_vpi_value value_p,
                        p_vpi_time time_p, PLI_INT32 flags);

/* Reads the current simulation time. `object` is accepted for IEEE
 * compatibility and currently ignored. `time_p->type` selects the format:
 * vpiSimTime fills high/low, vpiScaledRealTime fills real. */
void vpi_get_time(vpiHandle object, p_vpi_time time_p);

/* Releases a handle obtained from vpi_handle, vpi_iterate or vpi_scan.
 * Returns 1 on success, 0 on failure (including a NULL handle). */
PLI_INT32 vpi_free_object(vpiHandle object);
PLI_INT32 vpi_release_handle(vpiHandle object);
PLI_INT32 vpi_get_vlog_info(p_vpi_vlog_info vlog_info_p);

/* Only cbValueChange, cbReadWriteSynch, cbReadOnlySynch, cbNextSimTime,
 * cbAfterDelay, cbStartOfSimulation, cbEndOfSimulation, and cbStartOfReset
 * are dispatched. Any other reason is rejected with a NULL return rather
 * than silently accepted.
 *
 * cbAfterDelay takes its delay from cb_data_p->time, RELATIVE to now, and
 * counts as pending simulation work — a testbench driven only from VPI
 * timers keeps running rather than ending at time 0. cbReadWriteSynch and
 * cbReadOnlySynch are one-shot end-of-time-step callbacks; writes applied
 * from a cbReadWriteSynch open a fresh delta in the same slot, so a clock
 * driven there triggers edge-sensitive blocks normally. When a
 * cbValueChange fires, cb_data_p->obj, ->time and ->value are populated;
 * ->value uses the format of the value struct supplied at registration
 * (vpiIntVal if none was given). */
vpiHandle vpi_register_cb(p_cb_data cb_data_p);
/* Fills `cb_data_p` from a callback object returned by vpi_register_cb.
 * Returns 1 on success, 0 on failure. */
PLI_INT32 vpi_get_cb_info(vpiHandle cb_obj, p_cb_data cb_data_p);
PLI_INT32 vpi_remove_cb(vpiHandle cb_obj);

/* --- Arrays (IEEE 1800-2017 sections 38.16, 38.20, 38.35) ------------- */

/* Array object types. vpi_handle_by_name answers vpiMemory for a
 * one-dimensional array, vpiRegArray / vpiNetArray for a multi-dimensional
 * one and for a sub-array of one (`top.m[1]` of `logic [7:0] m[0:2][0:3]`). */
#define vpiNetArray          114
#define vpiRegArray          116

/* The subobject selected by `num_index` indices, leftmost first: one per
 * unpacked dimension still open on `obj` gives an element, fewer a
 * sub-array. Further indices select through the element's packed
 * dimensions: a part-select (vpiPartSelect) until the last one, which
 * selects a bit (vpiRegBit / vpiNetBit). A plain vector takes only packed
 * indices. NULL when the indices do not form a legal select.
 * vpi_handle_by_index on an array is the one-index case. */
vpiHandle vpi_handle_by_multi_index(vpiHandle obj, PLI_INT32 num_index,
                                    PLI_INT32 *index_array);

typedef struct t_vpi_arrayvalue {
    PLI_UINT32 format;  /* vpi[Int,Real,Time,ShortInt,LongInt,ShortReal,
                           RawTwoState,RawFourState,Vector]Val */
    PLI_UINT32 flags;   /* vpiUserAllocFlag; vpiOneValue, vpiPropagateOff */
    union {
        PLI_INT32           *integers;
        PLI_INT16           *shortints;
        PLI_INT64           *longints;
        PLI_BYTE8           *rawvals;
        struct t_vpi_vecval *vectors;
        struct t_vpi_time   *times;
        double              *reals;
        float               *shortreals;
    } value;
} s_vpi_arrayvalue, *p_vpi_arrayvalue;

#define vpiUserAllocFlag      0x2000   /* get: value points at caller memory */
#define vpiOneValue           0x4000   /* put: one value for every element */
#define vpiPropagateOff       0x8000   /* put: do not wake the readers */

/* Read / write `num` consecutive elements of a static unpacked array (or a
 * sub-array) starting at index_p — one index per open dimension, leftmost
 * first. The rightmost dimension varies fastest, and every dimension runs
 * from its declared left bound towards its right one; a section that runs
 * past the end of the array is an error.
 *
 * Formats: vpiIntVal, vpiTimeVal, vpiVectorVal, vpiRawTwoStateVal and
 * vpiRawFourStateVal for any integral element type; vpiRealVal for real
 * elements; vpiShortRealVal for shortreal ones. vpiShortIntVal reads
 * shortint / byte elements and writes shortint / int / longint ones;
 * vpiLongIntVal reads longint / shortint / byte elements and writes
 * longint ones. Raw layout per element: ceil(size/8) aval bytes, then (four
 * state) as many bval bytes, bit 0 in the LSB of the first byte.
 *
 * vpi_get_value_array stores into simulator-owned memory, valid until the
 * next call, unless vpiUserAllocFlag says value points at the caller's
 * buffer. On any error it sets the value pointer to NULL (and reports
 * through vpi_chk_error). vpi_put_value_array writes like vpi_put_value
 * with vpiNoDelay; vpiOneValue writes the first value to every element,
 * and vpiPropagateOff stores the values without waking the processes and
 * callbacks that watch them. Any other flag is an error, and on any error
 * nothing is written. */
void vpi_get_value_array(vpiHandle object, p_vpi_arrayvalue arrayvalue_p,
                         PLI_INT32 *index_p, PLI_UINT32 num);
void vpi_put_value_array(vpiHandle object, p_vpi_arrayvalue arrayvalue_p,
                         PLI_INT32 *index_p, PLI_UINT32 num);

/* --- Delays (IEEE 1800-2017 sections 38.10, 38.22, 38.32) -------------- */

#define vpiInterModPath       26   /* intermodule path (port to port) */
#define vpiModPath            31   /* module path (specify block) */
#define vpiTchk               61   /* timing check */

/* vpi_get(vpiTchkType, tchk). */
#define vpiTchkType           38
#define vpiSetup               1
#define vpiHold                2
#define vpiPeriod              3
#define vpiWidth               4
#define vpiSkew                5
#define vpiRecovery            6
#define vpiNoChange            7
#define vpiSetupHold           8
#define vpiFullskew            9
#define vpiRecrem             10
#define vpiRemoval            11
#define vpiTimeskew           12

typedef struct t_vpi_delay {
    struct t_vpi_time *da;   /* caller-allocated array of delay values */
    PLI_INT32 no_of_delays;
    PLI_INT32 time_type;     /* vpiScaledRealTime or vpiSimTime */
    PLI_INT32 mtm_flag;      /* min:typ:max triples */
    PLI_INT32 append_flag;   /* put: add to the current delays */
    PLI_INT32 pulsere_flag;  /* delay, reject limit, error limit triples */
} s_vpi_delay, *p_vpi_delay;

/* vpi_iterate(vpiModPath, module) and vpi_iterate(vpiTchk, module) give the
 * module paths and timing checks of a module instance (vpiFullName of a
 * module path is its output net's; of a timing check, its scope plus the
 * check's name). vpi_handle_multi(vpiInterModPath, outPort, inPort) gives
 * the interconnect between an output port and an input port of the same
 * size; only those two reference handles are read. */
vpiHandle vpi_handle_multi(PLI_INT32 type, vpiHandle refHandle1,
                           vpiHandle refHandle2, ...);

/* The delays of an object, and the values the simulator then uses:
 *   - a net or port: the delay of the gate or continuous assignment that
 *     drives it, or its SDF / VPI annotation — 1, 2 (rise, fall) or 3
 *     (rise, fall, turn-off) delays. xezim lowers every gate and continuous
 *     assignment onto the net it drives, so the net is where their delays
 *     are read and (as an extension) written. A net whose driver was built
 *     without a delay in a form that cannot take one, or that is driven
 *     through module paths, refuses vpi_put_delays;
 *   - vpiModPath: 1, 2, 3, 6 or 12 transition delays;
 *   - vpiTchk: its limits, as many as the check has (one, or two for
 *     $setuphold, $recrem, $fullskew and $nochange), in source order;
 *   - vpiInterModPath: 2 or 3 delays, stored where SDF INTERCONNECT delays
 *     land: on the input port's net.
 * time_type vpiSimTime counts simulation ticks (a negative timing check
 * limit is two's complement in high/low); vpiScaledRealTime counts the
 * object's module time unit. xezim keeps one value per delay (the active
 * min:typ:max selection) and models inertial delays with no separate pulse
 * limits: with mtm_flag, min, typ and max all read that value and a put
 * takes the active selection's; with pulsere_flag, the reject and error
 * limits read the delay, and a put whose limits differ from its delay is
 * refused. append_flag adds the given values to the current ones. Errors
 * (and refusals, which write nothing) are reported through vpi_chk_error. */
void vpi_get_delays(vpiHandle object, p_vpi_delay delay_p);
void vpi_put_delays(vpiHandle object, p_vpi_delay delay_p);

/* --- Save / restart (IEEE 1800-2017 sections 38.9, 38.31) -------------- */

/* The standard allows these only from cbStartOfSave / cbEndOfSave (put) and
 * cbStartOfRestart / cbEndOfRestart (get) callbacks. xezim has no $save or
 * $restart, so those callbacks never fire and both routines always fail:
 * they return 0 (no bytes transferred) and report the error through
 * vpi_chk_error. */
PLI_INT32 vpi_get_data(PLI_INT32 id, PLI_BYTE8 *dataLoc, PLI_INT32 numOfBytes);
PLI_INT32 vpi_put_data(PLI_INT32 id, PLI_BYTE8 *dataLoc, PLI_INT32 numOfBytes);

/* DPI scope/runtime primitives live in svdpi.h with their proper
 * `svScope` type. Included here so both are visible together. */
#include "svdpi.h"

#endif /* VPI_USER_H */
