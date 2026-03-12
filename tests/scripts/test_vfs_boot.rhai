// Phase 5 VFS boot verification script.
//
// Builds and boots the kernel, waits for VFS setup messages confirming
// ramfs and devmgr servers started, mount succeeded, and `ls /` output
// is correct.

let vm = boot();

// Phase 2 should pass first.
vm.wait_serial("Phase 2 verification PASSED!", 30);

// Phase 5 VFS setup.
vm.wait_serial("ramfs: ready", 30);
vm.wait_serial("devmgr: ready", 30);
vm.wait_serial("ls /:", 30);
vm.wait_serial("Phase 5 verification PASSED!", 30);

vm.quit();
print("VFS boot verification passed!");
