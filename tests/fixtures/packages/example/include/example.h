#define EXAMPLE_VERSION_MAJOR 1
#define EXAMPLE_FLAG_NONE 0x00
#define EXAMPLE_FLAG_FAST (1 << 0)
#define EXAMPLE_FLAG_SAFE (1 << 1)
#define EXAMPLE_FLAGS_ALL (EXAMPLE_FLAG_FAST | EXAMPLE_FLAG_SAFE)

enum example_mode {
    EXAMPLE_MODE_OFF = 0,
    EXAMPLE_MODE_ON,
    EXAMPLE_MODE_AUTO = 10
};

// Function-like macro that calls a library function: becomes a PHP function.
#define EXAMPLE_TWICE(N) example_add(N, N)
// Calls a function this library does not define: becomes a throwing function.
#define EXAMPLE_MISSING(X) example_absent(X)

const char *example_version(void);
int example_add(int left, int right);

// Declared (prefix-matching) but never defined in the native library, so the
// library does not export it — like SDL's app-provided `SDL_main`. The export-symbol
// filter (which parses the binary directly, not via `nm`) must drop it; otherwise
// FFI::cdef fails to resolve it and the whole extension breaks.
int example_unexported(int value);

// A function-pointer parameter is rendered as a real C callback type (not an opaque
// void *) so a PHP `callable` can be passed; example_apply invokes the callback
// synchronously and returns its result plus one.
int example_apply(int value, int (*callback)(int));

// A named enum used as both a parameter and a return type: it is surfaced as a PHP
// enum, so example_next_mode accepts and returns a `Pnlx\Example\Enums\example_mode`.
enum example_mode example_next_mode(enum example_mode mode);

// A struct with scalar fields: its `Types\example_point` wrapper gets typed
// getX()/setX()/getY()/setY() accessors. example_point_init writes through a
// PHP-allocated struct and example_point_sum reads one passed back in.
struct example_point {
    int x;
    int y;
};
void example_point_init(struct example_point *point, int x, int y);
int example_point_sum(const struct example_point *point);

// struct に値として埋め込まれた union。両方のラッパーへ型付きアクセサを生成し、
// aggregate 定義を依存順に出力する。
union example_number {
    int integer;
    float decimal;
};
struct example_value {
    int kind;
    union example_number number;
};
int example_value_integer(const struct example_value *value);
void example_value_init(struct example_value *value, int integer);

// メンバー名は独自の名前空間を持つため正しい C だが、フィールド名と typedef が
// 衝突するので PHP FFI は生成した本体を解釈できない。ラッパーは CData を内部に
// 隠したまま、サイズとアラインメントを使うフォールバックへ切り替える。
typedef int example_storage_word;
struct example_opaque {
    example_storage_word example_storage_word;
};
void example_opaque_write(struct example_opaque *value, int word);
int example_opaque_read(const struct example_opaque *value);

// A static inline helper has no exported symbol, so it cannot be bound through FFI;
// it is surfaced as a throwing stub method (marked #[StaticInline]) instead of
// being dropped.
static inline int example_inline_double(int value) { return value * 2; }
