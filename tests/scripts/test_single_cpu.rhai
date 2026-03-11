// Single-CPU regression test — verifies SMP code doesn't break uniprocessor boot.
//
// Run: just script tests/scripts/test_single_cpu.rhai
// Default QEMU config uses 1 CPU.

let vm = boot();

// Verify normal boot completes
vm.wait_serial("Hello from userspace", 20);

vm.quit();
print("Single-CPU regression test passed!");
