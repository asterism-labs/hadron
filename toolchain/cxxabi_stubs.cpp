// cxxabi_stubs.cpp — Minimal C++ ABI support for cross-compiled C++ code.
//
// Provides operator new/delete (delegating to malloc/free) and the minimum
// set of __cxa_* symbols needed by Mesa's C++ code.

extern "C" {

void *malloc(unsigned long size);
void  free(void *ptr);
void  abort(void) __attribute__((noreturn));

} // extern "C"

// --- operator new / delete ---------------------------------------------------

void *operator new(unsigned long size) {
    return malloc(size);
}

void *operator new[](unsigned long size) {
    return malloc(size);
}

void operator delete(void *ptr) noexcept {
    free(ptr);
}

void operator delete[](void *ptr) noexcept {
    free(ptr);
}

void operator delete(void *ptr, unsigned long) noexcept {
    free(ptr);
}

void operator delete[](void *ptr, unsigned long) noexcept {
    free(ptr);
}

// --- C++ ABI -----------------------------------------------------------------

extern "C" {

// Called when a pure virtual function is invoked.
void __cxa_pure_virtual(void) {
    abort();
}

// atexit registration for static destructors — stub (we never run dtors).
int __cxa_atexit(void (*)(void *), void *, void *) {
    return 0;
}

// Guard variables for thread-safe static local initialization.
// Mesa may use static locals in C++ code.
int __cxa_guard_acquire(long long *guard) {
    if (*reinterpret_cast<char *>(guard)) {
        return 0; // Already initialized.
    }
    return 1; // Needs initialization.
}

void __cxa_guard_release(long long *guard) {
    *reinterpret_cast<char *>(guard) = 1;
}

void __cxa_guard_abort(long long *) {
    // Nothing to do for the stub.
}

// Thread-local storage — stub for single-threaded compilation phase.
void *__cxa_get_globals(void) {
    static char buf[64];
    return buf;
}

void *__cxa_get_globals_fast(void) {
    return __cxa_get_globals();
}

} // extern "C"
