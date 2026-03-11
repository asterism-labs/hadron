// SMP boot smoke test — verifies all APs come online.
//
// Run: just script tests/scripts/test_smp_boot.rhai
// QEMU must be configured with -smp 4 (set via gluon profile or QEMU args).

let vm = boot();

// Wait for SMP initialization message
vm.wait_serial("SMP:", 10);

// Wait for all CPUs to report online
vm.wait_serial("CPUs online", 15);

// Verify userspace still boots correctly with SMP
vm.wait_serial("Hello from userspace", 20);

vm.quit();
print("SMP boot smoke test passed!");
