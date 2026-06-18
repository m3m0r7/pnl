# pnl — binding-generation roadmap

Status of the `pnl-packages` example-execution effort (each package's `EXAMPLES.md`
run through PHP FFI on Ubuntu 24.04 / Alpine 3.21). Install is 141/144 Ubuntu,
138/144 Alpine (the gaps are libraries the distro doesn't package: libduckdb,
libtcc, libquickjs (ubuntu), libnfc/libnlopt/libnng/libserialport (alpine)).

What follows is the **remaining** example-execution work, grouped by what kind of
fix it needs. The generator/SDK fixes already landed are recorded in
`/Volumes/develop/pnl-docker-test/RESULTS.md`.

## Validation status (2026-06-17, Alpine docker harness)

The A/B generator fixes below were implemented in `src/cli/header_adapter.rs` and
verified against the real packages via `iter-zlib.sh`:

- **libmongoc** — now **passes end-to-end** (`mongoc_init`, prints `1.27.1`). Needed
  three fixes: function-pointer struct fields → `void *`; an object-like macro with an
  empty `()` body dropped from `const.php`; and a guard so a spurious
  `typedef int bool(int *);` never redefines the builtin `bool` (which had turned every
  `bool` struct field into "function type is not allowed").
- **openssl** — cdef now **parses** (typedef-renderer fix + a new rule: a function whose
  *return* type is an inline function pointer, `DSA_meth_get_sign`, renders the return as
  `void *`). Remaining example error is a missing/unexported symbol `OpenSSL_version`
  (example-side, not cdef).
- **libmbedtls** — cdef now **parses** (incomplete by-value typedef member kept opaque).
  Remaining error is a `static inline` symbol with no export (`mbedtls_md_info_from_type`)
  — the documented not-generator-fixable category.
- **libidn2 / liboniguruma / libpcre2** — byte-pointer params now reach the cdef as
  `char *` and PHP FFI accepts the string (verified: `const char *src`,
  `const char *pattern`). Remaining errors are example-side: liboniguruma's
  `OnigEncoding` int-vs-global, libpcre2's by-ref-literal out-parameter.
- Regression check: zlib (1.3.2) and libsodium (1.0.20) still pass.

Three generator fixes were added during validation beyond the original A/B list:
function-returning-function-pointer → `void *`; drop operand-less object-like macro
constants; never typedef over a C fundamental type name.

## Example/manifest pass (2026-06-17, Alpine docker)

The param-shape (section B) failures were resolved — most by a generator/SDK fix
rather than an example edit, all verified end-to-end:

- **libargon2 / libbz2 / libssh** — PASS. Root cause was the wrapper, not the
  example: a single-level `void *` parameter was typed `ContextInterface|CData|null`,
  rejecting the PHP string the example passed, even though PHP FFI accepts a string
  for `void *`. The wrapper now also admits `string` for a `void *` param
  (`types::is_void_pointer`, threaded through `ParamView.void_pointer` into the
  method/global templates). libargon2 prints `hash ok`; libbz2/libssh run clean.
- **libidn2** — PASS (`ACE label: www.xn--bcher-kva.example`). The `uint8_t **`
  out-parameter is a C-string out, but `OutParameterMarshaller` read the `void *`
  holder with `FFI::string()` (which rejects `void *`); it now casts to `char *`
  first via `Allocator::readCString()`. The example was rewritten to the idiomatic
  by-reference form (no manual allocation) and uses `\u{00fc}` for a real UTF-8 IDN.
- **liboniguruma** — `onig_version()` works; the example now documents that
  `onig_new()` needs pointers to exported global *variables* (`ONIG_ENCODING_UTF8`,
  a syntax) which the generated function bindings can't reach — a real SDK gap
  (no FFI global-variable access), not an example bug.
- **libpcre2** — still blocked by a generator bug, NOT example-fixable: the
  `PCRE2_SPTR8 pattern` first parameter is dropped from the cdef entirely, so the
  remaining args shift and a literal lands in a by-ref slot. Needs investigation of
  why that parameter is dropped during collection.

## Tail sweep (2026-06-17, Alpine docker)

- **libnghttp2** — PASS (`version: 1.69.0`, `Protocol: h2`, `Header valid: 1`). The
  `nghttp2_info` struct renders; a returned pointer arrives as a `Types\nghttp2_info`
  context, so the example unwraps it with `->cdata()` and reads the `char *` fields
  through `\Pnlx\Util::cString()`.
- **libconfig** — `config_t` now renders fully and the cdef loads. Two new generator
  rules made it sizable: a function-pointer-*typedef* struct field
  (`config_include_fn_t include_fn`) → opaque `void *` (generalised from the inline
  `(*)` case via `is_function_pointer_entity`, canonical-type based), and an
  enum-typedef struct field (`config_error_t error_type`) is rendered (it projects to
  `int`) instead of tripping the value-field None-gate. The example documents that
  *allocating* a `config_t` still needs the extension's own FFI scope — pnl doesn't
  expose package-struct allocation to examples (the library-less Allocator/static
  FFI::new only know builtin types). A real SDK gap, like liboniguruma's globals.
- **libsdl** — cdef loads; the example needs a display (`XDG_RUNTIME_DIR`), so
  "install + cdef loads" is the achievable headless bar (met).
- **libmbedtls** — cdef loads; the example calls `mbedtls_md_info_from_type`, a
  `static inline` accessor with no exported symbol — the documented
  not-generator-fixable category.

Regression-checked: libmongoc (1.27.1), zlib (1.3.2) still pass after the struct-field
rendering refactor.

## SDK capabilities: exported data symbols + package-struct allocation (2026-06-17)

Two things the typed function bindings can't express are now first-class, through one
central `\Pnlx\FFI\GlobalMemory` (the only component that reaches a booted
`NativeLibrary`, via `AbstractExtension::pnlxNativeLibrary()` — so the generated entity
stays a pure bag of C functions). `GlobalMemory` resolves each item once, caches it,
and keeps it alive for the request (so a `CData` handed to a native call isn't GC'd);
it exposes `symbol()`, `allocate()`, `free()`, `clear()`.

- **Exported data symbols (C globals)** — the generator emits each as `extern <type>
  <name>;` in the cdef (`collect_global`/`VarDecl`) AND a marker class under
  `src/generated/symbol/<name>.php` (implementing `\Pnlx\FFI\SymbolInterface`), plus an
  entity constant `Liboniguruma::OnigEncodingUTF8 = …\Symbol\OnigEncodingUTF8::class`.
  The constant is a cheap compile-time string; you **pass it straight to the function**
  (`Liboniguruma::onig_get_syntax_options(Liboniguruma::OnigDefaultSyntax)`) and the
  dispatch (`ArgumentMarshaller::unwrap`) resolves the marker to its `\FFI\CData` through
  `GlobalMemory` *internally* — `GlobalMemory` is not example-facing for symbols. Pointer
  params accept the marker `string`. Address-vs-value is picked from the symbol's type (a
  struct instance → its address, a pointer global → its value). **liboniguruma works**
  (`default syntax options: 0x0`), even with `OnigEncodingType` opaque (the linker
  resolves a name to an address regardless of declared type).
- **Package-struct allocation** — `GlobalMemory::allocate(Libconfig::class, 'config_t')`
  allocates a generated struct in the extension's own FFI scope (the library-less
  Allocator only knows builtins) and holds it for the request. **libconfig works**
  (`config init/destroy`, with `FFI::addr()`).

This replaced an earlier `Native::of()` + `__callStatic` sentinel approach that leaked
the raw `NativeLibrary` to examples — reverted.

Also fixed: an array parameter whose element is a typedef'd pointer
(`onig_initialize(OnigEncoding encodings[], …)`) was decayed to a single `OnigEncoding`,
losing a pointer level — `source_argument_type` dropped the trailing `[]` with the name.
It now preserves the array→pointer decay (`OnigEncoding *encodings`), so `onig_initialize`
returns `rc=0`. (`onig_new`'s `pattern_end` still needs a `pattern + len` pointer, a C
convention awkward in any FFI — example-mechanics, not a pnl bug.)

Remaining generator bug, NOT the SDK:
- **libpcre2** — `PCRE2_SPTR8 pattern` first param dropped from the cdef (macro
  reconstruction).

Regression sweep clean: zlib, libsodium, liblz4, libwebp, libyaml, libuuid, libmongoc,
libsdl(loads), liboniguruma, libconfig.

## A. Generator-fixable cdef/codegen (next, highest leverage)

- **libmbedtls** — DONE. `has_incomplete_value_member` now also rejects a by-value
  member whose bare *typedef* aliases an aggregate the cdef never emits (the member
  hid inside an anonymous union, slipping past the `struct X`/`union X` keyword
  scan), so the enclosing struct stays opaque. Unit test
  `keeps_struct_opaque_when_it_embeds_an_incomplete_typedef_by_value`.
- **libmongoc** — DONE. Function-pointer struct fields (`void (*destroy)(…)`) now
  render as opaque, pointer-sized `void *` (a function pointer is pointer-sized, and
  the generated PHP can't invoke a struct-stored callback anyway), mirroring the
  `void *` treatment of fn-pointer params. Empirically the only thing PHP FFI 8.5
  rejects in a struct is a function-*type* typedef used by value (already kept
  opaque); a fn-*pointer* field loads fine, but `void *` is strictly safer when a
  callback parameter references a type FFI can't size in struct context. Unit test
  `renders_function_pointer_struct_fields_as_void_pointers`.
- **openssl** — DONE. `typedef_declaration` now distinguishes a function-*pointer*
  typedef (first `(` followed by `*`) from a function-*type* typedef, so a nested
  function-pointer *parameter*'s `(*)` no longer captures the typedef name
  (`OSSL_FUNC_provider_register_child_cb_fn`). Unit test
  `function_type_typedef_keeps_nested_function_pointer_parameters`. (This is the most
  likely root cause for libmongoc too — confirm both in the docker harness.)
- **libpango** — NOT a generator bug. The `nm -D`/`nm -gU` export filter is correct
  (it strips `@@VER` suffixes and the Mach-O leading underscore). `pango_version_string`
  is genuinely absent from the installed `.so`'s exports (build/version skew), so the
  fix is example-side (call an exported symbol).
- **libsdl / libconfig** — still need a real example run to re-triage (macro
  resolution; libsdl additionally needs a display). Requires the docker harness.

## B. Param-shape / example judgement calls

These load the cdef but the example's argument types don't line up. Each is a
per-package decision between broadening a *general* generator rule (watch for
regressions) and fixing the stale `EXAMPLES.md`.

Empirical baseline (PHP FFI 8.5, verified directly): a PHP string is accepted for a
`char *` **and a `void *`** parameter, but rejected for `unsigned char *` /
`uint8_t *` / `signed char *` / `int8_t *` (all "Passing incompatible argument …,
expecting 'uint8_t*', found PHP 'string'").

- **libargon2 / libbz2 / libssh** — the premise was wrong: **PHP FFI 8.5 *does*
  coerce a PHP string into a `void *` / `const void *`** (verified). So these should
  already accept the string at the FFI boundary, or fail for a different reason.
  Re-triage with a real example run before any generator change; do **not** apply the
  risky broad `void *` → `const char *` rewrite.
- **liboniguruma** — the `onig_new(…, OnigEncoding enc)` global pointer is still
  example-side (pass `ONIG_ENCODING_UTF8`, a pointer, not an int). The *pattern*
  parameter `const OnigUChar *` is now handled by the byte-pointer rewrite below.
- **libidn2 / oniguruma pattern / pcre2** — DONE. A single-level pointer to a
  non-`char` byte type **parameter** is rewritten to `char *` in the cdef (return
  types untouched), so PHP FFI accepts the string; the generated wrapper already
  typed these as `string`. Covers builtins written directly (`idn2_lookup_u8(const
  uint8_t *src, …)`) **and** byte pointers reached through a typedef — both
  `const OnigUChar *` (`OnigUChar` → `unsigned char`) and pcre2's `PCRE2_SPTR8`
  (the typedef is itself `const unsigned char *`). A `char`-based typedef (libtidy
  `tmbchar`) is left alone because `char *` already works. Unit tests
  `rewrites_byte_pointer_parameters_to_char_pointers` and
  `rewrites_byte_pointer_parameters_hidden_behind_a_typedef`.
- **libnghttp2** — `nghttp2_info` is a simple struct (`{ int age; int version_num;
  const char *version_str; const char *proto_str; }`) that should render fine, so its
  opacity is likely a collection-gating artifact in a real header (not reproducible
  here without it). Re-check after a docker run; it may already be resolved by the
  fixes above.

## C. Not generator-fixable (document; "install + cdef loads" is the bar)

- **`static inline` only APIs** — the example calls a function with no exported symbol
  (defined `static inline` in the header): libnettle (`nettle_version_major`),
  libmsgpack (`msgpack_sbuffer_init`), libjansson (`json_decref`). Would require pnl to
  emit PHP shims for inline functions, or the example to avoid them.
- **Symbol versioning / renaming** — the example calls a name the `.so` exports under
  a decorated/renamed symbol: libicu (`u_getVersion` → `u_getVersion_74`), libgmp
  (`mpz_init` → `__gmpz_init`), libtheora (`th_*` vs `theora_*`). Possibly recoverable
  with an `nm -D`-aware alias map (strip ICU's `_NN` suffix, follow the `__g`-prefixed
  GMP names) — worth a focused investigation, otherwise example-side.
- **Multi-`.so` packages** — libbrotli's API is split across
  libbrotlienc/dec/common; needs multi-library support in one extension.
- **Hardware / display / TTY / server** — can't run head­less; the achievable bar is
  "install + cdef loads": libnfc, librtlsdr, libserialport, libusb (alpine), libglew,
  libvulkan, libsdl, libncurses/libnotcurses, libopenal (ALSA), libpq (needs a DB),
  libpcap.
- **Runtime crashes in complex examples** — load + simple calls work, the example's
  complex path segfaults: libgcrypt, libmpfr, libogg, libopenblas, libreadline,
  libassimp, libgumbo. Triage per package or simplify the example.

## Known limitation (by design)

By-reference out-parameters (`int *`, `char **`, `T **`) accept a plain variable so
the result writes back, but PHP forbids passing a *literal* to a by-reference slot —
`f(null)` fatals. Pass a variable (`$x = null; f($x)`) or omit the trailing
out-parameter (`f()`), which is the PHP-expressible equivalent of C `NULL`.
