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

// A static inline helper has no exported symbol, so it cannot be bound through FFI;
// it is surfaced as a throwing stub method (marked #[StaticInline]) instead of
// being dropped.
static inline int example_inline_double(int value) { return value * 2; }
