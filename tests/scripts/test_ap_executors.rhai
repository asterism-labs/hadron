// AP executor entry test — verifies application processors enter the executor loop.
//
// Run: just script tests/scripts/test_ap_executors.rhai
// QEMU must be configured with -smp 4.

let vm = boot();

// Wait for at least one AP to enter the executor
vm.wait_serial("entering executor", 15);

// Verify all CPUs come online
vm.wait_serial("CPUs online", 15);

vm.quit();
print("AP executor entry test passed!");
