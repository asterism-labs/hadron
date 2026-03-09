# Capability and Resource Model

Hadron follows a capability-based security model derived from Zircon. Every interaction between
processes, and between processes and the kernel, is mediated by handles. There is no ambient
authority: a process cannot access any kernel object unless it holds a handle to it, and that
handle's rights determine what operations are permitted.

---

## Core Concepts

### No Ambient Authority

In a POSIX system, a process with UID 0 (root) can open any file, kill any process, and configure
any network interface — not because it holds a specific capability, but because of its UID. This
ambient privilege is the root cause of many privilege escalation vulnerabilities: obtaining root
access, even temporarily, grants all permissions simultaneously.

Hadron has no equivalent of UID 0. A process that needs to access a specific device, create a
new process, or register a filesystem mount must hold the appropriate handle. Handles are passed
explicitly from parent to child during process creation or via channel messages. A process that
does not receive a handle at creation time has no way to obtain one through side channels.

### Handles and Rights

Every kernel object (Process, Thread, VMO, Channel, Interrupt, etc.) is identified by a `Koid`
— a globally unique 64-bit identifier that is never reused within a boot session. User processes
access objects through handles: integer indices into the process's handle table.

Each handle entry in the table stores:
- An `Arc<dyn KernelObject>` — a shared reference to the actual kernel object.
- A `Rights` bitmask — the set of operations this handle is permitted to perform.

The `Rights` flags are:

| Flag | Meaning |
|------|---------|
| `READ` | Read data (channel messages, VMO bytes) |
| `WRITE` | Write data |
| `EXECUTE` | Execute code from the object (VMO → executable mapping) |
| `MAP` | Map the object into an address space (VMO → VMAR) |
| `DUPLICATE` | Duplicate the handle, optionally with reduced rights |
| `TRANSFER` | Send the handle over a channel to another process |
| `SIGNAL` | Raise or clear user-visible signals on the object |
| `WAIT` | Wait on the object's signals |
| `MANAGE_PROCESS` | Start or kill a process |
| `MANAGE_THREAD` | Start, suspend, or kill a thread |
| `ENUMERATE` | List children (job → processes, process → threads) |
| `SET_POLICY` | Set policy on jobs or resources |

Rights are **monotonically decreasing**. When a handle is duplicated (`handle_dup`), the caller
specifies a rights mask that must be a subset of the original handle's rights. The new handle
cannot have rights the original lacked. The kernel enforces this in the `HandleTable::duplicate`
method.

A process can transfer a handle to another process by sending it over a channel. The receiving
process gets a new handle table entry with the same rights the sender specified. The sender loses
the handle (transfer is a move, not a copy) unless it duplicates first.

---

## Resource Object

The `Resource` object is the hierarchical capability tree. It does not represent a single kernel
object but rather a grant of authority to access a class of hardware or system resource.

### Root Resource

The root `Resource` is created during kernel bootstrap with full system authority. It is the only
object in the system created without a parent capability check. Every other privileged object
(MMIO regions, I/O ports, IRQ lines, IOMMU units) can only be accessed by a process that holds
a `Resource` handle with the appropriate subtype.

The root resource handle is passed to userboot. Userboot passes subsets of it to `devmgr`, which
passes further-reduced subsets to driver processes. By the time a driver process receives its
resource handles, they cover only the hardware resources belonging to its device.

### Resource Hierarchy

Resources form a tree. A resource can be subdivided to create child resources with narrower scope:

```
Root Resource (full authority)
├── IRQ Resource (IRQ lines 0..255)
│   ├── IRQ Resource (IRQ line 9 — PCI device A)
│   └── IRQ Resource (IRQ line 11 — PCI device B)
├── MMIO Resource (physical address range 0xFE000000..0xFEFFFFFF)
│   └── MMIO Resource (BAR0 of PCI device A: 0xFE010000..0xFE01FFFF)
├── IO Port Resource (ports 0x0000..0xFFFF)
└── IOMMU Resource (VT-d unit 0)
```

A child resource cannot exceed its parent's authority. Attempting to create a child resource
covering a range not contained in the parent returns `EACCES`.

Resource subtypes:

| Subtype | Authority |
|---------|-----------|
| `IRQ`   | Access to one or more hardware IRQ lines (create `Interrupt` objects) |
| `MMIO`  | Access to a physical MMIO address range (create `MmioFrame` VMOs) |
| `IOPORT` | Access to x86 I/O port range |
| `IOMMU` | Access to a VT-d unit (create `Bti` objects) |
| `SYSTEM` | System-wide operations (reboot, shutdown) |

---

## Job Policy

A `Job` is a container for processes. Every process belongs to a job, and every job (except the
root) belongs to a parent job. The job hierarchy parallels the process tree.

Jobs enforce policy constraints on their descendants:

| Policy | Effect |
|--------|--------|
| Maximum processes | Limit the number of processes in the subtree |
| No new jobs | Prevent any process in the subtree from creating child jobs |
| No socket creation | Prevent AF_UNIX socket creation |
| No VMO execution | Prevent mapping any VMO as executable (W^X enforcement) |
| Bad handle policy | What to do when a process closes a handle it does not own: `ALLOW`, `DENY`, `KILL_PROCESS` |

The `job_set_policy` syscall sets policy; `job_set_critical` marks a job such that if any process
in it exits with a non-zero status, the entire job is torn down.

---

## Per-Process Namespace as Capability Filter

A process's namespace (the set of VFS mount points it can see) is itself a capability. A process
only sees mount entries that were explicitly included in the namespace it was given at creation
time. This is the mechanism for filesystem-level sandboxing:

- A build worker process receives a namespace containing only its workspace directory and
  read-only system paths.
- A network service receives a namespace with `/etc` (configuration) but not `/home` (user data).
- A completely sandboxed process receives an empty namespace and can only access objects passed
  to it explicitly as handles.

The namespace is not a POSIX `chroot`. It does not merely change the root; it selects which mount
entries are visible at all. A sandboxed process cannot traverse up to a mount point it was not
given, because that mount point does not exist in its namespace.

---

## Comparison with POSIX Permissions

| Aspect | POSIX | Hadron |
|--------|-------|--------|
| Identity | UID/GID + groups | Handle possession |
| Privilege check | Ambient: UID 0 bypasses all checks | Explicit: must hold a handle with the right rights |
| Privilege delegation | `setuid` binary, `sudo`, capability bits | Pass a handle with reduced rights |
| Revocation | Kill the process | Close the handle (drops reference count) |
| File access control | ACL/DAC on filesystem | Namespace (which FS servers are visible) |
| Device access | `/dev` node owned by group | Handle to MMIO VMO or Interrupt object |
| Escalation path | Exploit setuid binary or kernel | No ambient privilege to escalate to |
| Auditability | `/proc/1/status`, `strace` | Handle table is the complete authority record |

The key practical difference is that in Hadron, a compromised userspace process cannot gain any
privilege it was not given at startup. There is no equivalent of "sudo" or "setuid" that grants
kernel-level access based on identity alone. Privilege must flow through the capability tree,
which means the attack surface for privilege escalation is dramatically reduced.

---

## Capability Passing Example

A concrete example: a user application gaining access to an audio device.

1. `devmgr` holds a handle to the audio driver's service channel (passed to it by the audio driver
   during startup).
2. `init` or a session manager, on behalf of the logged-in user, connects to `devmgr`'s service
   channel and requests a handle to the audio device.
3. `devmgr` verifies the requester's job policy (is audio access allowed for this session?) and,
   if so, calls `handle_dup` on the audio service channel handle with reduced rights
   (`READ | WRITE`, not `MANAGE_PROCESS`).
4. `devmgr` sends the reduced handle over a channel to the requesting process.
5. The application calls `channel_recv_fd` to receive the handle and can now talk to the audio
   driver via its service channel.

At no point did the application need UID 0, and `devmgr` never gave the application more rights
than are needed to send and receive audio data. If the application is compromised, the attacker
can play audio — but cannot use the audio handle to access any other device or process.
