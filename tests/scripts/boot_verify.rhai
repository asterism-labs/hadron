// Boot verification script.
//
// Builds and boots the kernel, waits for key serial messages confirming
// successful kernel init and userboot launch, then exits cleanly.

let vm = boot();

vm.wait_serial("kernel bootstrap complete", 10);
vm.wait_serial("Hello from userspace", 30);

vm.quit();
print("Boot verification passed!");
